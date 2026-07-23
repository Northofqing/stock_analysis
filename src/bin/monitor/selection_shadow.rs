//! BR-155/BR-156/BR-157 monitor adapter for event-scoped shadow selection.

use stock_analysis::calendar::{self, MarketSession};
use stock_analysis::news::aggregator::NewsAggregationBatch;
use stock_analysis::selection::magic_tdx::SelectionMarketWindow;
use stock_analysis::selection::outcome::{
    settle_due_outcomes as settle_selection_outcomes, OutcomeSettlementSummary,
};
use stock_analysis::selection::pipeline::{
    evaluate_market_events, SelectionContext, SelectionEventBatch, SelectionRunOutcome,
};

const ENABLE_ENV: &str = "STOCK_ANALYSIS_SELECTION_SHADOW_ENABLE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillSwitchError {
    value: String,
}

impl std::fmt::Display for KillSwitchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{ENABLE_ENV} must be one of 1/true/0/false, got {:?}",
            self.value
        )
    }
}

pub fn parse_selection_shadow_enable(value: Option<&str>) -> Result<bool, KillSwitchError> {
    match value.map(str::trim) {
        None => Ok(true),
        Some("1" | "true") => Ok(true),
        Some("0" | "false") => Ok(false),
        Some(value) => Err(KillSwitchError {
            value: value.to_owned(),
        }),
    }
}

pub async fn evaluate_news_batch(batch: NewsAggregationBatch) {
    let enabled = match selection_shadow_enabled() {
        Ok(enabled) => enabled,
        Err(error) => {
            log::error!(
                "[selection-shadow][BR-155] disabled reason={} error={error}",
                error.reason_code()
            );
            return;
        }
    };
    if !enabled {
        log::debug!("[selection-shadow][BR-155] disabled by operator");
        return;
    }

    let batch = match SelectionEventBatch::try_from(batch) {
        Ok(batch) => batch,
        Err(error) => {
            log::warn!(
                "[selection-shadow][BR-155] input unavailable reason_code={} retryable={}",
                error.code(),
                error.retryable()
            );
            return;
        }
    };
    let now = chrono::Local::now();
    let context = selection_context(now, calendar::current_session());
    match evaluate_market_events(batch, context).await {
        SelectionRunOutcome::Completed(summary) => log::info!(
            "[selection-shadow][BR-155][BR-156][BR-157] completed events={} admitted={} rejected={} pending={}",
            summary.evaluated_events,
            summary.admitted_candidates,
            summary.rejected_candidates,
            summary.pending_events
        ),
        SelectionRunOutcome::VerifiedEmpty(summary) => log::info!(
            "[selection-shadow][BR-155] verified_empty events={} sources={}",
            summary.evaluated_events,
            summary.source_count
        ),
        SelectionRunOutcome::Unavailable(unavailable) => log::warn!(
            "[selection-shadow][BR-155] unavailable reason_code={} retryable={}",
            unavailable.reason_code,
            unavailable.retryable
        ),
    }
}

#[derive(Debug)]
pub struct ShadowSettlementError {
    reason_code: &'static str,
    message: String,
}

impl ShadowSettlementError {
    pub fn reason_code(&self) -> &'static str {
        self.reason_code
    }
}

impl std::fmt::Display for ShadowSettlementError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub async fn settle_due_outcomes(
    now: chrono::DateTime<chrono::Local>,
) -> Result<OutcomeSettlementSummary, ShadowSettlementError> {
    // The kill switch stops only new candidate evaluation. Already-visible
    // immutable candidates must still receive their T0/D+1 outcomes.
    settle_selection_outcomes(now)
        .await
        .map_err(|error| ShadowSettlementError {
            reason_code: error.reason_code(),
            message: error.to_string(),
        })
}

fn selection_shadow_enabled() -> Result<bool, ShadowSettlementError> {
    let value = match std::env::var(ENABLE_ENV) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => {
            return Err(ShadowSettlementError {
                reason_code: "kill_switch_unreadable",
                message: format!("{ENABLE_ENV} is unreadable: {error}"),
            });
        }
    };
    parse_selection_shadow_enable(value.as_deref()).map_err(|error| ShadowSettlementError {
        reason_code: "invalid_kill_switch",
        message: error.to_string(),
    })
}

fn selection_context(
    now: chrono::DateTime<chrono::Local>,
    session: MarketSession,
) -> SelectionContext {
    let today = now.date_naive();
    let (window, expected_latest_settled_date) = match session {
        MarketSession::Auction
        | MarketSession::Morning
        | MarketSession::LunchBreak
        | MarketSession::Afternoon => (
            SelectionMarketWindow::Intraday,
            calendar::prev_trading_day(today),
        ),
        MarketSession::AfterHours if calendar::is_trading_day(today) => {
            (SelectionMarketWindow::PostClose, today)
        }
        MarketSession::AfterHours | MarketSession::Closed => (
            SelectionMarketWindow::PostClose,
            calendar::prev_trading_day(today),
        ),
    };
    SelectionContext::new(window, now, expected_latest_settled_date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn invalid_kill_switch_fails_closed() {
        assert!(parse_selection_shadow_enable(Some("invalid")).is_err());
        assert_eq!(parse_selection_shadow_enable(None), Ok(true));
        assert_eq!(parse_selection_shadow_enable(Some("0")), Ok(false));
    }

    #[test]
    fn shadow_adapter_has_no_delivery_or_execution_capability() {
        let source = include_str!("selection_shadow.rs");
        for (prefix, suffix) in [
            ("push_", "wechat"),
            ("Sink", "Router"),
            ("Trading", "Bus"),
            ("paper_", "trades"),
            ("place_", "order"),
        ] {
            let forbidden = format!("{prefix}{suffix}");
            assert!(
                !source.contains(&forbidden),
                "forbidden capability: {forbidden}"
            );
        }
    }

    #[test]
    fn existing_news_governance_precedes_shadow_evaluation() {
        let source = include_str!("main.rs");
        let governance = source
            .find("push_flash_decisions(decisions).await")
            .expect("existing news governance call");
        let selection = source
            .find("selection_shadow::evaluate_news_batch(news_batch).await")
            .expect("selection shadow call");
        assert!(governance < selection);
    }

    #[test]
    fn existing_post_session_scheduler_is_the_only_outcome_owner() {
        let source = include_str!("main.rs");
        let production = source
            .split("mod tests_post_session_review_scheduler")
            .next()
            .expect("production source precedes tests");
        assert_eq!(
            production
                .matches("selection_shadow::settle_due_outcomes(now).await")
                .count(),
            1
        );
        let scheduler = production
            .split("async fn post_session_review_scheduler()")
            .nth(1)
            .expect("post-session scheduler")
            .split("fn spawn_post_session_review_scheduler")
            .next()
            .expect("scheduler body");
        assert!(scheduler.contains("selection_shadow::settle_due_outcomes(now).await"));
    }

    #[test]
    fn market_window_never_claims_unsettled_intraday_daily_bar() {
        let now = chrono::Local
            .with_ymd_and_hms(2026, 7, 23, 10, 0, 0)
            .single()
            .expect("test time");
        let context = selection_context(now, MarketSession::Morning);
        assert_eq!(context.window, SelectionMarketWindow::Intraday);
        assert!(context.expected_latest_settled_date < now.date_naive());

        let after_close = selection_context(now, MarketSession::AfterHours);
        assert_eq!(after_close.window, SelectionMarketWindow::PostClose);
        assert_eq!(after_close.expected_latest_settled_date, now.date_naive());
    }
}
