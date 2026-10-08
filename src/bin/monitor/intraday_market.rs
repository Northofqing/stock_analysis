use stock_analysis::market_data::TopStock;

/// Acquisitions retain their original observation types. The production
/// scanner uses LimitUpObservation and ScannerPositionQuotes rather than
/// reducing both to an interchangeable Vec<TopStock>.
pub struct IntradayMarketInputs<Limit = Vec<TopStock>, Position = Vec<TopStock>> {
    pub limit_stocks: Result<Limit, String>,
    pub position_quotes: Result<Position, String>,
}

pub struct ResolvedIntradayMarketInputs<Limit = Vec<TopStock>, Position = Vec<TopStock>> {
    pub limit_stocks: Option<Limit>,
    pub position_quotes: Option<Position>,
    pub limit_error: Option<String>,
    pub position_error: Option<String>,
    pub task_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntradayConsumerPlan {
    pub use_limit_data: bool,
    pub use_position_data: bool,
    pub run_independent_jobs: bool,
}

impl<Limit, Position> ResolvedIntradayMarketInputs<Limit, Position> {
    pub fn consumer_plan(&self) -> IntradayConsumerPlan {
        IntradayConsumerPlan {
            use_limit_data: self.limit_stocks.is_some(),
            use_position_data: self.position_quotes.is_some(),
            run_independent_jobs: true,
        }
    }
}

pub fn acquire_intraday_market_inputs<Limit, Position, LimitFetch, PositionFetch>(
    limit_fetch: LimitFetch,
    position_fetch: PositionFetch,
) -> IntradayMarketInputs<Limit, Position>
where
    LimitFetch: FnOnce() -> Result<Limit, String>,
    PositionFetch: FnOnce() -> Result<Position, String>,
{
    let limit_stocks = limit_fetch();
    let position_quotes = position_fetch();
    IntradayMarketInputs {
        limit_stocks,
        position_quotes,
    }
}

pub fn resolve_intraday_market_inputs<Limit, Position>(
    task_result: Result<IntradayMarketInputs<Limit, Position>, String>,
) -> ResolvedIntradayMarketInputs<Limit, Position> {
    match task_result {
        Ok(inputs) => {
            let (limit_stocks, limit_error) = match inputs.limit_stocks {
                Ok(stocks) => (Some(stocks), None),
                Err(error) => (None, Some(error)),
            };
            let (position_quotes, position_error) = match inputs.position_quotes {
                Ok(quotes) => (Some(quotes), None),
                Err(error) => (None, Some(error)),
            };
            ResolvedIntradayMarketInputs {
                limit_stocks,
                position_quotes,
                limit_error,
                position_error,
                task_error: None,
            }
        }
        Err(error) => ResolvedIntradayMarketInputs {
            limit_stocks: None,
            position_quotes: None,
            limit_error: None,
            position_error: None,
            task_error: Some(error),
        },
    }
}

/// Refresh the complete original quote batch after the slower overlay has
/// finished. No retry or fallback to an earlier quote is allowed here.
pub async fn acquire_scanner_quotes_after_overlay<
    OverlayFuture,
    Position,
    WindowCheck,
    PositionFetch,
>(
    overlay: OverlayFuture,
    mut window_open: WindowCheck,
    position_fetch: PositionFetch,
) -> (OverlayFuture::Output, Result<Position, String>)
where
    OverlayFuture: std::future::Future,
    Position: Send + 'static,
    WindowCheck: FnMut() -> bool + Send + 'static,
    PositionFetch: FnOnce() -> Result<Position, String> + Send + 'static,
{
    let overlay = overlay.await;
    let position_quotes = tokio::task::spawn_blocking(move || {
        if !window_open() {
            return Err("scanner_continuous_session_closed".into());
        }
        let quotes = position_fetch()?;
        if !window_open() {
            return Err("scanner_continuous_session_closed".into());
        }
        Ok(quotes)
    })
    .await
    .map_err(|error| format!("scanner_quote_acquisition_task_failed:{error}"))
    .and_then(|result| result);
    (overlay, position_quotes)
}

pub fn scanner_window_open_at(
    now: chrono::DateTime<chrono::Utc>,
    tick_date: chrono::NaiveDate,
) -> bool {
    let shanghai = chrono::FixedOffset::east_opt(8 * 3600).expect("Shanghai offset");
    now.with_timezone(&shanghai).date_naive() == tick_date
        && stock_analysis::trading::paper_sell::intraday_session_open_at(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[tokio::test(start_paused = true)]
    async fn scanner_refresh_waits_for_delayed_overlay_before_acquiring_original_quote() {
        use std::sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        };
        let overlay_finished = Arc::new(AtomicBool::new(false));
        let quote_calls = Arc::new(AtomicUsize::new(0));
        let started = tokio::time::Instant::now();
        let overlay_flag = overlay_finished.clone();
        let quote_flag = overlay_finished.clone();
        let quote_counter = quote_calls.clone();
        let (overlay, quote) = acquire_scanner_quotes_after_overlay(
            async move {
                tokio::time::sleep(std::time::Duration::from_secs(12)).await;
                overlay_flag.store(true, Ordering::SeqCst);
                "TEST_CODE_original_flow"
            },
            || true,
            move || {
                quote_counter.fetch_add(1, Ordering::SeqCst);
                assert!(quote_flag.load(Ordering::SeqCst));
                Ok(("TEST_CODE_original_quote", tokio::time::Instant::now()))
            },
        )
        .await;
        let (source, acquired_at) = quote.expect("fresh original quote");
        assert_eq!(overlay, "TEST_CODE_original_flow");
        assert_eq!(source, "TEST_CODE_original_quote");
        assert!(acquired_at >= started + std::time::Duration::from_secs(12));
        assert_eq!(quote_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn scanner_refresh_source_failure_is_preserved_without_retry_or_old_quote() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let quote_calls = Arc::new(AtomicUsize::new(0));
        let quote_counter = quote_calls.clone();
        let (overlay, quote): (_, Result<&str, String>) = acquire_scanner_quotes_after_overlay(
            async { "TEST_CODE_original_flow" },
            || true,
            move || {
                quote_counter.fetch_add(1, Ordering::SeqCst);
                Err("TEST_CODE_original_quote_unavailable".into())
            },
        )
        .await;
        assert_eq!(overlay, "TEST_CODE_original_flow");
        assert_eq!(quote.unwrap_err(), "TEST_CODE_original_quote_unavailable");
        assert_eq!(quote_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn scanner_refresh_missing_overlay_keeps_missing_flow_and_acquires_quote_once() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let quote_calls = Arc::new(AtomicUsize::new(0));
        let quote_counter = quote_calls.clone();
        let (overlay, quote) = acquire_scanner_quotes_after_overlay(
            async { std::collections::HashMap::<String, f64>::new() },
            || true,
            move || {
                quote_counter.fetch_add(1, Ordering::SeqCst);
                Ok("TEST_CODE_original_quote_without_flow")
            },
        )
        .await;
        assert!(overlay.is_empty());
        assert_eq!(quote.unwrap(), "TEST_CODE_original_quote_without_flow");
        assert_eq!(quote_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn scanner_refresh_closed_or_crossed_session_discards_even_fresh_quote() {
        use std::sync::{
            atomic::{AtomicI64, AtomicUsize, Ordering},
            Arc,
        };
        let tick_date = chrono::NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        for (started, returned, expected_calls) in [
            ("2026-10-08T03:29:59Z", "2026-10-08T03:30:02Z", 1),
            ("2026-10-08T06:59:59Z", "2026-10-08T07:00:02Z", 1),
            ("2026-10-08T04:00:00Z", "2026-10-08T04:00:02Z", 0),
            ("2026-10-09T01:30:00Z", "2026-10-09T01:30:02Z", 0),
        ] {
            let started = chrono::DateTime::parse_from_rfc3339(started)
                .unwrap()
                .with_timezone(&chrono::Utc);
            let returned = chrono::DateTime::parse_from_rfc3339(returned)
                .unwrap()
                .with_timezone(&chrono::Utc);
            let clock = Arc::new(AtomicI64::new(started.timestamp()));
            let window_clock = clock.clone();
            let source_clock = clock.clone();
            let quote_calls = Arc::new(AtomicUsize::new(0));
            let quote_counter = quote_calls.clone();
            let (_, quote) = acquire_scanner_quotes_after_overlay(
                async { () },
                move || {
                    scanner_window_open_at(
                        chrono::DateTime::from_timestamp(window_clock.load(Ordering::SeqCst), 0)
                            .unwrap(),
                        tick_date,
                    )
                },
                move || {
                    quote_counter.fetch_add(1, Ordering::SeqCst);
                    source_clock.store(returned.timestamp(), Ordering::SeqCst);
                    Ok(returned)
                },
            )
            .await;
            assert_eq!(quote.unwrap_err(), "scanner_continuous_session_closed");
            assert_eq!(quote_calls.load(Ordering::SeqCst), expected_calls);
        }
    }

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
        let inputs: IntradayMarketInputs = acquire_intraday_market_inputs(
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
        let inputs: IntradayMarketInputs = acquire_intraday_market_inputs(
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
        let resolved: ResolvedIntradayMarketInputs =
            resolve_intraday_market_inputs(Err("TEST_CODE join failed".to_string()));
        let plan = resolved.consumer_plan();

        assert!(resolved.limit_stocks.is_none());
        assert!(resolved.position_quotes.is_none());
        assert!(resolved.limit_error.is_none());
        assert!(resolved.position_error.is_none());
        assert_eq!(
            resolved.task_error.as_deref(),
            Some("TEST_CODE join failed")
        );
        assert!(!plan.use_limit_data);
        assert!(!plan.use_position_data);
        assert!(plan.run_independent_jobs);
    }

    #[test]
    fn scanner_quote_original_observation_types_survive_source_matrix_without_projection() {
        // Pure acquisition plumbing values, not Gateway admission factories.
        // Different source types cannot be accidentally interchanged here.
        struct DatePoolMarker(&'static str);
        struct QuoteBatchMarker(&'static str, Vec<&'static str>);
        for (limit_ok, position_ok) in [(true, true), (true, false), (false, true), (false, false)]
        {
            let position_calls = Cell::new(0);
            let inputs = acquire_intraday_market_inputs(
                || {
                    if limit_ok {
                        Ok(DatePoolMarker("TEST_CODE_original_date_pool_receipt"))
                    } else {
                        Err("TEST_CODE date source unavailable".into())
                    }
                },
                || {
                    position_calls.set(position_calls.get() + 1);
                    if position_ok {
                        Ok(QuoteBatchMarker(
                            "TEST_CODE_original_quote_batch",
                            vec!["TEST_CODE_000001", "TEST_CODE_600000"],
                        ))
                    } else {
                        Err("TEST_CODE native quote source unavailable".into())
                    }
                },
            );
            assert_eq!(position_calls.get(), 1);
            let resolved = resolve_intraday_market_inputs(Ok(inputs));
            assert_eq!(resolved.limit_stocks.is_some(), limit_ok);
            assert_eq!(resolved.position_quotes.is_some(), position_ok);
            if let Some(pool) = resolved.limit_stocks.as_ref() {
                assert_eq!(pool.0, "TEST_CODE_original_date_pool_receipt");
            }
            if let Some(batch) = resolved.position_quotes.as_ref() {
                assert_eq!(batch.0, "TEST_CODE_original_quote_batch");
                assert_eq!(batch.1, ["TEST_CODE_000001", "TEST_CODE_600000"]);
            }
            assert_eq!(resolved.limit_error.is_some(), !limit_ok);
            assert_eq!(resolved.position_error.is_some(), !position_ok);
            assert!(resolved.consumer_plan().run_independent_jobs);
        }
    }
}
