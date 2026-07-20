use stock_analysis::market_data::TopStock;

pub struct IntradayMarketInputs {
    pub limit_stocks: Result<Vec<TopStock>, String>,
    pub position_quotes: Result<Vec<TopStock>, String>,
}

pub fn acquire_intraday_market_inputs<LimitFetch, PositionFetch>(
    limit_fetch: LimitFetch,
    position_fetch: PositionFetch,
) -> IntradayMarketInputs
where
    LimitFetch: FnOnce() -> Result<Vec<TopStock>, String>,
    PositionFetch: FnOnce() -> Result<Vec<TopStock>, String>,
{
    let limit_stocks = limit_fetch();
    let position_quotes = position_fetch();
    IntradayMarketInputs {
        limit_stocks,
        position_quotes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn test_stock(code: &str) -> stock_analysis::market_data::TopStock {
        stock_analysis::market_data::TopStock {
            code: code.to_string(),
            name: "TEST_CODE position".to_string(),
            change_pct: 1.0,
            price: 10.0,
            volume_ratio: Some(1.5),
            main_net_yi: Some(0.2),
        }
    }

    #[test]
    fn limit_failure_does_not_prevent_position_quote_acquisition() {
        let position_called = Cell::new(false);
        let inputs = acquire_intraday_market_inputs(
            || Err("TEST_CODE limit source rejected".to_string()),
            || {
                position_called.set(true);
                Ok(vec![test_stock("TEST_CODE_000001")])
            },
        );

        assert!(position_called.get());
        assert!(inputs.limit_stocks.is_err());
        assert_eq!(
            inputs.position_quotes.expect("position source succeeds")[0].code,
            "TEST_CODE_000001"
        );
    }

    #[test]
    fn position_failure_does_not_discard_limit_up_data() {
        let inputs = acquire_intraday_market_inputs(
            || Ok(vec![test_stock("TEST_CODE_LIMIT")]),
            || Err("TEST_CODE position source rejected".to_string()),
        );

        assert_eq!(
            inputs.limit_stocks.expect("limit source succeeds")[0].code,
            "TEST_CODE_LIMIT"
        );
        assert!(inputs.position_quotes.is_err());
    }

    #[test]
    fn resolved_inputs_preserve_the_complete_source_matrix() {
        let cases = [
            (true, true, true, true),
            (true, false, true, false),
            (false, true, false, true),
            (false, false, false, false),
        ];

        for (limit_ok, position_ok, expect_limit, expect_position) in cases {
            let inputs = IntradayMarketInputs {
                limit_stocks: if limit_ok {
                    Ok(vec![test_stock("TEST_CODE_LIMIT")])
                } else {
                    Err("TEST_CODE limit rejected".to_string())
                },
                position_quotes: if position_ok {
                    Ok(vec![test_stock("TEST_CODE_POSITION")])
                } else {
                    Err("TEST_CODE position rejected".to_string())
                },
            };

            let resolved = resolve_intraday_market_inputs(Ok(inputs));
            let plan = resolved.consumer_plan();
            assert_eq!(resolved.limit_stocks.is_some(), expect_limit);
            assert_eq!(resolved.position_quotes.is_some(), expect_position);
            assert_eq!(resolved.limit_error.is_some(), !expect_limit);
            assert_eq!(resolved.position_error.is_some(), !expect_position);
            assert!(resolved.task_error.is_none());
            assert_eq!(plan.use_limit_data, expect_limit);
            assert_eq!(plan.use_position_data, expect_position);
            assert!(plan.run_independent_jobs);
        }
    }

    #[test]
    fn task_failure_keeps_independent_jobs_eligible() {
        let resolved = resolve_intraday_market_inputs(Err("TEST_CODE join failed".to_string()));
        let plan = resolved.consumer_plan();

        assert!(resolved.limit_stocks.is_none());
        assert!(resolved.position_quotes.is_none());
        assert!(resolved.limit_error.is_none());
        assert!(resolved.position_error.is_none());
        assert_eq!(resolved.task_error.as_deref(), Some("TEST_CODE join failed"));
        assert!(!plan.use_limit_data);
        assert!(!plan.use_position_data);
        assert!(plan.run_independent_jobs);
    }
}
