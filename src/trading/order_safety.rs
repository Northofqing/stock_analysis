//! BR-084: shared fail-closed validation for simulated and paper orders.

pub const MAX_SINGLE_ORDER_RMB: f64 = 1_000_000.0;
pub const SECONDARY_CONFIRM_RMB: f64 = 500_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetySide {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
pub struct OrderSafetyInput<'a> {
    pub code: &'a str,
    pub side: SafetySide,
    pub order_price: f64,
    pub quantity: u64,
    pub available_cash: Option<f64>,
    pub limit_down_price: Option<f64>,
    pub limit_up_price: Option<f64>,
    pub secondary_confirmed: bool,
}

pub fn validate(input: &OrderSafetyInput<'_>) -> Result<(), String> {
    crate::risk::env_guard::validate_symbol_for_current_env(input.code)?;

    if !input.order_price.is_finite() || input.order_price <= 0.0 {
        return Err(format!(
            "BR-084 invalid order price for {}: {}",
            input.code, input.order_price
        ));
    }
    if input.quantity == 0 || !input.quantity.is_multiple_of(100) {
        return Err(format!(
            "BR-084 quantity must be positive and divisible by 100: {}",
            input.quantity
        ));
    }

    let lower = input
        .limit_down_price
        .filter(|price| price.is_finite() && *price > 0.0)
        .ok_or_else(|| format!("BR-084 missing/invalid limit-down price for {}", input.code))?;
    let upper = input
        .limit_up_price
        .filter(|price| price.is_finite() && *price > 0.0)
        .ok_or_else(|| format!("BR-084 missing/invalid limit-up price for {}", input.code))?;
    if lower > upper {
        return Err(format!(
            "BR-084 invalid daily price range for {}: {}..{}",
            input.code, lower, upper
        ));
    }
    if input.order_price < lower || input.order_price > upper {
        return Err(format!(
            "BR-084 order price {} outside daily range {}..{} for {}",
            input.order_price, lower, upper, input.code
        ));
    }

    let notional = input.order_price * input.quantity as f64;
    if !notional.is_finite() || notional > MAX_SINGLE_ORDER_RMB {
        return Err(format!(
            "BR-084 order notional {:.2} exceeds {:.2}",
            notional, MAX_SINGLE_ORDER_RMB
        ));
    }
    if input.side == SafetySide::Buy {
        let cash = input
            .available_cash
            .filter(|cash| cash.is_finite() && *cash >= 0.0)
            .ok_or_else(|| "BR-084 buy requires valid available cash".to_string())?;
        if notional > cash {
            return Err(format!(
                "BR-084 order notional {:.2} exceeds available cash {:.2}",
                notional, cash
            ));
        }
    }
    if notional >= SECONDARY_CONFIRM_RMB && !input.secondary_confirmed {
        return Err(format!(
            "BR-084 secondary confirmation required for order notional {:.2}",
            notional
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> OrderSafetyInput<'static> {
        OrderSafetyInput {
            code: "TEST_CODE_000001",
            side: SafetySide::Buy,
            order_price: 10.0,
            quantity: 100,
            available_cash: Some(100_000.0),
            limit_down_price: Some(9.0),
            limit_up_price: Some(11.0),
            secondary_confirmed: false,
        }
    }

    #[test]
    fn accepts_complete_safe_order() {
        assert!(validate(&valid()).is_ok());
    }

    #[test]
    fn rejects_non_lot_and_non_positive_values() {
        for quantity in [0, 99, 101] {
            let mut input = valid();
            input.quantity = quantity;
            assert!(validate(&input).is_err());
        }
        let mut input = valid();
        input.order_price = 0.0;
        assert!(validate(&input).is_err());
    }

    #[test]
    fn rejects_missing_bounds_and_out_of_range_price() {
        let mut input = valid();
        input.limit_up_price = None;
        assert!(validate(&input).is_err());
        let mut input = valid();
        input.order_price = 12.0;
        assert!(validate(&input).is_err());

        let mut input = valid();
        input.order_price = 11.000_001;
        assert!(
            validate(&input).is_err(),
            "BR-084 forbids any price above the exact source limit"
        );
    }

    #[test]
    fn rejects_cash_and_single_order_limit_breaches() {
        let mut input = valid();
        input.available_cash = Some(500.0);
        assert!(validate(&input).is_err());
        let mut input = valid();
        input.order_price = 1_100.0;
        input.limit_up_price = Some(1_200.0);
        input.limit_down_price = Some(1_000.0);
        input.quantity = 1_000;
        input.available_cash = Some(2_000_000.0);
        assert!(validate(&input).is_err());
    }

    #[test]
    fn requires_secondary_confirmation_at_boundary() {
        let mut input = valid();
        input.order_price = 500.0;
        input.limit_down_price = Some(450.0);
        input.limit_up_price = Some(550.0);
        input.quantity = 1_000;
        input.available_cash = Some(600_000.0);
        assert!(validate(&input).is_err());
        input.secondary_confirmed = true;
        assert!(validate(&input).is_ok());
    }
}
