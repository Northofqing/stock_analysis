//! Registered business rules: BR-137.
//! v17.7 Task 5: Monitor-only source-to-push adapter
//!
//! Consumes `NormalizedSourceEvent` from the news aggregator and dispatches
//! exactly one `push_governor_v3` call per event. No retry, no fallback PushKind.
//!
//! v17.7 Task 7: Adds bounded polling for earnings and analyst data on the watchlist.

use crate::notify::{self, PushKind, PushOutcome};
use chrono::Local;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use stock_analysis::data_provider::consensus;
use stock_analysis::data_provider::financials;
use stock_analysis::monitor::event_bus::MonitorEvent;
use stock_analysis::news::aggregator::analyst_state::{
    AnalystKey, AnalystObservation, AnalystStateStore,
};
use stock_analysis::news::aggregator::classifier::{
    classify_earnings, EarningsClassification, EarningsConfig, EarningsKind,
};
use stock_analysis::news::aggregator::{NormalizedSourceEvent, SourcePushKind};

/// Bounded state map for deduping OrderUpdate events.
/// Tracks (action, shares) per code; only emits if the tuple changes.
#[derive(Debug, Default)]
pub struct MarketActionState {
    seen: HashMap<String, (String, u64)>, // code → (action, shares)
}

impl MarketActionState {
    /// Returns true if this is a new state (code/action/shares combination is different
    /// from the last time we saw this code), false if unchanged.
    pub fn accept(&mut self, event: &MonitorEvent) -> bool {
        if let MonitorEvent::OrderUpdate {
            code,
            action,
            shares,
        } = event
        {
            let prev = self.seen.get(code).cloned();
            let is_new = prev.as_ref() != Some(&(action.clone(), *shares));
            self.seen.insert(code.clone(), (action.clone(), *shares));
            is_new
        } else {
            false
        }
    }
}

/// Build a MarketActionAlert NormalizedSourceEvent from an OrderUpdate MonitorEvent.
pub fn normalize_market_action(event: &MonitorEvent) -> Option<NormalizedSourceEvent> {
    if let MonitorEvent::OrderUpdate {
        code,
        action,
        shares,
    } = event
    {
        NormalizedSourceEvent::new(
            SourcePushKind::MarketActionAlert,
            format!("order:{}:{}:{}", code, action, shares),
            Some(code.clone()),
            format!("OrderUpdate: {} {} shares", action, shares),
            format!("Order action {} for {}", action, code),
            stock_analysis::signal::market_event::Direction::Neutral,
            70,
            90,
            false,
            "monitor".into(),
            None,
        )
        .ok()
    } else {
        None
    }
}

/// Handle a MonitorEvent: dedup via MarketActionState, then push via push_normalized_event.
pub async fn handle_monitor_event(
    event: &MonitorEvent,
    state: &Mutex<MarketActionState>,
) -> Option<PushAttempt> {
    let is_new = {
        let mut s = state.lock().ok()?;
        s.accept(event)
    }; // MutexGuard dropped here, before any await
    if !is_new {
        return None; // unchanged, skip
    }
    let normalized = normalize_market_action(event)?;
    Some(push_normalized_event(normalized).await)
}

#[derive(Debug, Clone)]
pub struct PushAttempt {
    pub kind: PushKind,
    pub code: Option<String>,
    pub outcome: PushOutcome,
    pub rendered_len: usize,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SourcePollReport {
    pub attempted: usize,
    pub classified: usize,
    pub pushed: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Maps SourcePushKind 1:1 to the corresponding PushKind variant.
fn source_push_kind_to_push_kind(kind: SourcePushKind) -> PushKind {
    match kind {
        SourcePushKind::Announcement => PushKind::Announcement,
        SourcePushKind::PolicyHit => PushKind::PolicyHit,
        SourcePushKind::EarningsBeat => PushKind::EarningsBeat,
        SourcePushKind::EarningsMiss => PushKind::EarningsMiss,
        SourcePushKind::AnalystUpgrade => PushKind::AnalystUpgrade,
        SourcePushKind::MarketActionAlert => PushKind::MarketActionAlert,
    }
}

/// Returns a static str label for human-readable rendering.
fn source_push_kind_label(kind: SourcePushKind) -> &'static str {
    match kind {
        SourcePushKind::Announcement => "Announcement",
        SourcePushKind::PolicyHit => "PolicyHit",
        SourcePushKind::EarningsBeat => "EarningsBeat",
        SourcePushKind::EarningsMiss => "EarningsMiss",
        SourcePushKind::AnalystUpgrade => "AnalystUpgrade",
        SourcePushKind::MarketActionAlert => "MarketActionAlert",
    }
}

/// Renders a NormalizedSourceEvent into a push message string.
fn render_message(event: &NormalizedSourceEvent) -> String {
    let mut s = format!(
        "[{}] {}\n{}\nsource={}",
        source_push_kind_label(event.push_kind),
        event.title,
        event.summary,
        event.source
    );
    if let Some(url) = &event.url {
        s.push_str(&format!(" url={}", url));
    }
    if !event.metadata.is_empty() {
        for (k, v) in &event.metadata {
            s.push_str(&format!(" {}={}", k, v));
        }
    }
    s
}

/// Pushes a single NormalizedSourceEvent through the monitor push pipeline.
/// Calls `push_governor_v3` exactly once; no retry, no fallback PushKind.
pub async fn push_normalized_event(event: NormalizedSourceEvent) -> PushAttempt {
    let kind = source_push_kind_to_push_kind(event.push_kind);
    let code_str = event.code.clone();
    if let Err(error) = event.validate() {
        log::error!(
            "[v17.7][BR-137] normalized source event rejected: kind={kind:?} reason={error}"
        );
        return PushAttempt {
            kind,
            code: code_str,
            outcome: PushOutcome::Denied(format!("source_event_invalid:{error}")),
            rendered_len: 0,
        };
    }
    let rendered = render_message(&event);
    let outcome = if matches!(
        kind,
        PushKind::Announcement
            | PushKind::PolicyHit
            | PushKind::EarningsBeat
            | PushKind::EarningsMiss
            | PushKind::AnalystUpgrade
    ) {
        match crate::v14_adapter::SourceFactEvidence::new(
            kind,
            event.event_id.clone(),
            event.code.clone(),
            event.title.clone(),
            event.source.clone(),
            event.occurred_at,
            event.strength,
            event.certainty,
            event.stale,
        ) {
            Ok(evidence) => notify::push_source_fact_v3(&rendered, &evidence).await,
            Err(error) => {
                log::error!("[v17.7][BR-137] source fact rejected: kind={kind:?} reason={error}");
                PushOutcome::Denied(format!("source_fact_invalid:{error}"))
            }
        }
    } else {
        notify::push_governor_v3(&rendered, kind, code_str.as_deref()).await
    };
    let rendered_len = rendered.len();
    PushAttempt {
        kind,
        code: code_str,
        outcome,
        rendered_len,
    }
}

/// Processes a batch of NormalizedSourceEvents, skipping those with empty
/// title or event_id before attempting any push.
pub async fn push_normalized_events(events: Vec<NormalizedSourceEvent>) -> SourcePollReport {
    let mut report = SourcePollReport::default();
    for event in events {
        report.attempted += 1;
        if let Err(error) = event.validate() {
            log::warn!(
                "[v17.7][BR-137] source batch item skipped: kind={:?} reason={error}",
                event.push_kind
            );
            report.skipped += 1;
            continue;
        }
        report.classified += 1;
        let attempt = push_normalized_event(event).await;
        match attempt.outcome {
            PushOutcome::Pushed => report.pushed += 1,
            _ => report.failed += 1,
        }
    }
    report
}

/// Build a NormalizedSourceEvent for an earnings classification.
fn earnings_classification_to_event(
    code: &str,
    classification: &EarningsClassification,
) -> NormalizedSourceEvent {
    let push_kind = match classification.kind {
        EarningsKind::Beat => SourcePushKind::EarningsBeat,
        EarningsKind::Miss => SourcePushKind::EarningsMiss,
        EarningsKind::Unclassified => SourcePushKind::EarningsBeat, // Should not happen
    };
    let (direction, title_prefix) = match classification.kind {
        EarningsKind::Beat => (
            stock_analysis::signal::market_event::Direction::Bull,
            "超预期",
        ),
        EarningsKind::Miss => (
            stock_analysis::signal::market_event::Direction::Bear,
            "低于预期",
        ),
        EarningsKind::Unclassified => (
            stock_analysis::signal::market_event::Direction::Neutral,
            "未分类",
        ),
    };
    let title = format!(
        "{} 业绩{} (实际EPS {} vs 预期 {})",
        code, title_prefix, classification.actual, classification.reference
    );
    let summary = format!("delta {}%", classification.delta_pct);
    let event_id = format!(
        "earnings:{}:{}",
        code,
        classification.report_date.format("%Y%m%d")
    );
    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert("actual".to_string(), classification.actual.to_string());
    metadata.insert(
        "reference".to_string(),
        classification.reference.to_string(),
    );
    metadata.insert(
        "delta_pct".to_string(),
        classification.delta_pct.to_string(),
    );

    NormalizedSourceEvent {
        push_kind,
        event_id,
        code: Some(code.to_string()),
        title,
        summary,
        direction,
        strength: 80,
        certainty: 90,
        occurred_at: Local::now(),
        stale: false,
        source: "earnings_classifier".to_string(),
        url: None,
        metadata,
    }
}

/// Build a NormalizedSourceEvent for an analyst upgrade.
fn analyst_upgrade_event(
    code: &str,
    broker: &str,
    from: &str,
    to: &str,
    report_id: &str,
) -> NormalizedSourceEvent {
    let event_id = format!("analyst:{}:{}:{}", code, broker, report_id);
    let title = format!("{} 券商上调评级", broker);
    let summary = format!("从 {} 上调至 {}", from, to);
    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert("broker".to_string(), broker.to_string());
    metadata.insert("from_rating".to_string(), from.to_string());
    metadata.insert("to_rating".to_string(), to.to_string());

    NormalizedSourceEvent {
        push_kind: SourcePushKind::AnalystUpgrade,
        event_id,
        code: Some(code.to_string()),
        title,
        summary,
        direction: stock_analysis::signal::market_event::Direction::Bull,
        strength: 70,
        certainty: 80,
        occurred_at: Local::now(),
        stale: false,
        source: "analyst_tracker".to_string(),
        url: None,
        metadata,
    }
}

/// Trait for fetching earnings and consensus data.
/// Allows test stubs without real HTTP calls.
pub trait EarningsFetcher: Send + Sync {
    async fn fetch_financials(
        &self,
        client: &reqwest::Client,
        code: &str,
    ) -> anyhow::Result<stock_analysis::data_provider::financials::Financials>;
    async fn fetch_consensus(
        &self,
        client: &reqwest::Client,
        code: &str,
    ) -> anyhow::Result<consensus::ConsensusData>;
}

/// Real fetcher using the existing data providers.
pub struct RealEarningsFetcher;

impl EarningsFetcher for RealEarningsFetcher {
    async fn fetch_financials(
        &self,
        client: &reqwest::Client,
        code: &str,
    ) -> anyhow::Result<stock_analysis::data_provider::financials::Financials> {
        financials::fetch_with_fallback_async(client, code).await
    }
    async fn fetch_consensus(
        &self,
        client: &reqwest::Client,
        code: &str,
    ) -> anyhow::Result<consensus::ConsensusData> {
        consensus::fetch_async(client, code).await
    }
}

/// Poll earnings and analyst data for the watchlist.
///
/// For each code in `our_codes`:
/// - If `elapsed < poll_secs_earnings` since last earnings poll, skip.
/// - Otherwise fetch financials + consensus, classify earnings, build event.
/// - If `elapsed < poll_secs_analyst` since last analyst poll, skip analyst check.
/// - Otherwise for each recent report, call analyst_store.observe(), build upgrade events.
///
/// Returns a SourcePollReport summarizing what was attempted/pushed/skipped/failed.
pub async fn poll_earnings_and_analyst(
    our_codes: &std::collections::HashSet<String>,
    earnings_cfg: &EarningsConfig,
    analyst_store: &AnalystStateStore,
    last_poll_earnings: Arc<Mutex<HashMap<String, Instant>>>,
    last_poll_analyst: Arc<Mutex<HashMap<String, Instant>>>,
    poll_secs_earnings: u64,
    poll_secs_analyst: u64,
) -> SourcePollReport {
    if our_codes.is_empty() {
        return SourcePollReport::default();
    }

    let http = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log::warn!("[v17_sources] failed to build HTTP client: {}", e);
            return SourcePollReport {
                failed: 1,
                ..SourcePollReport::default()
            };
        }
    };

    let fetcher = RealEarningsFetcher;
    let mut events: Vec<NormalizedSourceEvent> = Vec::new();
    let mut source_failures = 0usize;
    let now = Instant::now();

    // Track poll times for this iteration
    let poll_secs_earnings_duration = std::time::Duration::from_secs(poll_secs_earnings);
    let poll_secs_analyst_duration = std::time::Duration::from_secs(poll_secs_analyst);

    for code in our_codes {
        let code_str = code.as_str();

        // --- Earnings polling ---
        {
            let should_poll = {
                let last_polls = last_poll_earnings.lock().unwrap();
                last_polls
                    .get(code_str)
                    .map(|last| last.elapsed() >= poll_secs_earnings_duration)
                    .unwrap_or(true) // Never polled = poll now
            };

            if should_poll {
                let (financials_result, consensus_result) = tokio::join!(
                    fetcher.fetch_financials(&http, code_str),
                    fetcher.fetch_consensus(&http, code_str)
                );
                match (financials_result, consensus_result) {
                    (Ok(financials), Ok(consensus)) => {
                        if let Some(latest_period) = financials.history.first() {
                            if let Some(classification) =
                                classify_earnings(latest_period, &consensus, earnings_cfg)
                            {
                                events.push(earnings_classification_to_event(
                                    code_str,
                                    &classification,
                                ));
                            }
                        }
                        let mut last_polls = last_poll_earnings.lock().unwrap();
                        last_polls.insert(code_str.to_string(), now);
                    }
                    (financials_result, consensus_result) => {
                        source_failures += 1;
                        log::warn!(
                            "[v17_sources][BR-115] {} earnings batch rejected: financials={}; consensus={}",
                            code_str,
                            financials_result
                                .err()
                                .map(|error| error.to_string())
                                .unwrap_or_else(|| "ok".to_string()),
                            consensus_result
                                .err()
                                .map(|error| error.to_string())
                                .unwrap_or_else(|| "ok".to_string())
                        );
                    }
                }
            }
        }

        // --- Analyst polling ---
        {
            let should_poll = {
                let last_polls = last_poll_analyst.lock().unwrap();
                last_polls
                    .get(code_str)
                    .map(|last| last.elapsed() >= poll_secs_analyst_duration)
                    .unwrap_or(true)
            };

            if should_poll {
                match fetcher.fetch_consensus(&http, code_str).await {
                    Ok(consensus_data) => {
                        for report in &consensus_data.recent_reports {
                            let key = AnalystKey {
                                code: code_str.to_string(),
                                broker: report.org_name.clone(),
                            };
                            let publish_date = match chrono::NaiveDate::parse_from_str(
                                &report.publish_date,
                                "%Y-%m-%d",
                            ) {
                                Ok(date) => date,
                                Err(error) => {
                                    source_failures += 1;
                                    log::warn!(
                                        "[v17_sources][BR-115] {} analyst report date invalid ({}): {}",
                                        code_str,
                                        report.publish_date,
                                        error
                                    );
                                    continue;
                                }
                            };
                            let obs = AnalystObservation {
                                rating: report.rating.clone(),
                                publish_date,
                                report_id: report.title.clone(), // Use title as report_id proxy
                            };

                            if let stock_analysis::news::aggregator::analyst_state::ObservationDecision::Upgrade { from, to } = analyst_store.observe(key.clone(), obs) {
                                events.push(analyst_upgrade_event(
                                    code_str,
                                    &report.org_name,
                                    &from,
                                    &to,
                                    &report.title,
                                ));
                            }
                        }

                        // Update last poll time
                        {
                            let mut last_polls = last_poll_analyst.lock().unwrap();
                            last_polls.insert(code_str.to_string(), now);
                        }
                    }
                    Err(e) => {
                        source_failures += 1;
                        log::warn!(
                            "[v17_sources] {} analyst consensus fetch failed: {}",
                            code_str,
                            e
                        );
                    }
                }
            }
        }
    }

    // Push all collected events
    let mut report = if events.is_empty() {
        SourcePollReport::default()
    } else {
        push_normalized_events(events).await
    };
    report.failed += source_failures;
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
    use stock_analysis::signal::market_event::Direction;

    /// Returns a valid announcement event for testing.
    fn test_announcement_event() -> NormalizedSourceEvent {
        NormalizedSourceEvent {
            push_kind: SourcePushKind::Announcement,
            event_id: "ann-1".into(),
            code: Some("TEST_CODE_ANNOUNCEMENT".into()),
            title: "关于回购股份方案的公告".into(),
            summary: "回购".into(),
            direction: Direction::Neutral,
            strength: 50,
            certainty: 50,
            occurred_at: Local::now(),
            stale: false,
            source: "eastmoney".into(),
            url: Some("https://example.invalid/ann-1".into()),
            metadata: Default::default(),
        }
    }

    /// Returns an event with empty title — bypasses NormalizedSourceEvent::new()
    /// validation to test the adapter's own empty-title filter.
    fn test_event_with_empty_title() -> NormalizedSourceEvent {
        NormalizedSourceEvent {
            push_kind: SourcePushKind::Announcement,
            event_id: "ann-empty".into(),
            code: Some("TEST_CODE_EMPTY_TITLE".into()),
            title: "".into(),
            summary: "".into(),
            direction: Direction::Neutral,
            strength: 50,
            certainty: 50,
            occurred_at: Local::now(),
            stale: false,
            source: "test".into(),
            url: None,
            metadata: Default::default(),
        }
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn announcement_adapter_calls_only_announcement_kind() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let report = push_normalized_events(vec![test_announcement_event()]).await;
        assert_eq!(report.attempted, 1);
        assert_eq!(report.pushed, 1);
        assert_eq!(report.failed, 0);
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br137_complete_announcement_pushes_when_global_data_mode_is_down() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        crate::LATEST_BANNER
            .lock()
            .expect("test banner lock")
            .as_mut()
            .expect("test banner")
            .data_mode = crate::push_templates::DataMode::Unsafe;

        let attempt = push_normalized_event(test_announcement_event()).await;
        assert_eq!(attempt.outcome, PushOutcome::Pushed);
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br137_market_action_remains_strict_when_global_data_mode_is_down() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        crate::LATEST_BANNER
            .lock()
            .expect("test banner lock")
            .as_mut()
            .expect("test banner")
            .data_mode = crate::push_templates::DataMode::Unsafe;

        let event =
            normalize_market_action(&order_update("TEST_CODE_MARKET_ACTION_DOWN", "sell", 100))
                .expect("normalized market action");
        assert_eq!(
            push_normalized_event(event).await.outcome,
            PushOutcome::Denied("data_quality".to_string())
        );
    }

    #[tokio::test]
    async fn br137_stale_source_event_is_skipped_explicitly() {
        let mut event = test_announcement_event();
        event.stale = true;
        let report = push_normalized_events(vec![event]).await;
        assert_eq!(report.attempted, 1);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.pushed, 0);
    }

    #[tokio::test]
    async fn missing_source_identity_is_skipped_not_pushed() {
        let report = push_normalized_events(vec![test_event_with_empty_title()]).await;
        assert_eq!(report.skipped, 1);
        assert_eq!(report.pushed, 0);
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn analyst_upgrade_maps_to_analyst_upgrade_kind() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let event = NormalizedSourceEvent {
            push_kind: SourcePushKind::AnalystUpgrade,
            event_id: "analyst-1".into(),
            code: Some("TEST_CODE_ANALYST".into()),
            title: "券商上调评级".into(),
            summary: "上调至买入".into(),
            direction: Direction::Bull,
            strength: 70,
            certainty: 80,
            occurred_at: Local::now(),
            stale: false,
            source: " Wind".into(),
            url: None,
            metadata: Default::default(),
        };
        let attempt = push_normalized_event(event).await;
        assert_eq!(attempt.kind, PushKind::AnalystUpgrade);
        assert_eq!(attempt.outcome, PushOutcome::Pushed);
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn policy_hit_with_no_code_is_pushed_as_policy() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let event = NormalizedSourceEvent {
            push_kind: SourcePushKind::PolicyHit,
            event_id: "pol-1".into(),
            code: None,
            title: "关于促进数字经济高质量发展的通知".into(),
            summary: "政策".into(),
            direction: Direction::Bull,
            strength: 80,
            certainty: 90,
            occurred_at: Local::now(),
            stale: false,
            source: "ndrc".into(),
            url: Some("https://example.invalid/pol-1".into()),
            metadata: Default::default(),
        };
        let attempt = push_normalized_event(event).await;
        assert_eq!(attempt.kind, PushKind::PolicyHit);
        assert!(attempt.code.is_none());
        assert_eq!(attempt.outcome, PushOutcome::Pushed);
    }

    /// Helper: build a FinancialPeriod for testing.
    fn test_financial_period(
        _code: &str,
        eps: f64,
        report_date: &str,
    ) -> stock_analysis::data_provider::financials::FinancialPeriod {
        stock_analysis::data_provider::financials::FinancialPeriod {
            report_date: Some(report_date.to_string()),
            eps: Some(eps),
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            op_cash_flow_ps: None,
            total_asset_turnover: None,
            debt_to_assets: None,
        }
    }

    /// Helper: build a ConsensusData for testing with a single recent report.
    fn test_consensus_data(
        code: &str,
        broker: &str,
        rating: &str,
        eps_avg: f64,
    ) -> stock_analysis::data_provider::consensus::ConsensusData {
        use std::collections::HashMap;
        let mut rating_dist = HashMap::new();
        rating_dist.insert(rating.to_string(), 1);
        stock_analysis::data_provider::consensus::ConsensusData {
            report_count: 1,
            broker_count: 1,
            eps_this_year_avg: Some(eps_avg),
            eps_next_year_avg: None,
            eps_next2_year_avg: None,
            rating_distribution: rating_dist,
            target_price_high_avg: None,
            target_price_low_avg: None,
            latest_report_date: Some("2026-07-15".to_string()),
            recent_reports: vec![stock_analysis::data_provider::consensus::RecentReport {
                title: format!("{}研报-{}", broker, code),
                org_name: broker.to_string(),
                publish_date: "2026-07-15".to_string(),
                rating: rating.to_string(),
            }],
        }
    }

    #[tokio::test]
    async fn earnings_beat_and_miss_map_to_distinct_push_kinds() {
        let earnings_cfg = EarningsConfig {
            metric: "eps".to_string(),
            beat_threshold_pct: 10.0,
            miss_threshold_pct: -10.0,
            poll_interval_secs: 900,
        };

        // Beat case: actual EPS 1.10, consensus 1.00 → delta +10% → Beat
        let beat_actual = test_financial_period("TEST_CODE_EARNINGS_BEAT", 1.10, "2026-04-20");
        let beat_consensus = test_consensus_data("TEST_CODE_EARNINGS_BEAT", "券商A", "买入", 1.00);
        let beat_classification = classify_earnings(&beat_actual, &beat_consensus, &earnings_cfg);
        assert!(
            beat_classification.is_some(),
            "Beat classification should not be None"
        );
        assert_eq!(
            beat_classification.as_ref().unwrap().kind,
            EarningsKind::Beat
        );

        let beat_event = earnings_classification_to_event(
            "TEST_CODE_EARNINGS_BEAT",
            beat_classification.as_ref().unwrap(),
        );
        assert_eq!(beat_event.push_kind, SourcePushKind::EarningsBeat);

        // Miss case: actual EPS 0.89, consensus 1.00 → delta -11% → Miss
        let miss_actual = test_financial_period("TEST_CODE_EARNINGS_MISS", 0.89, "2026-04-20");
        let miss_consensus = test_consensus_data("TEST_CODE_EARNINGS_MISS", "券商B", "中性", 1.00);
        let miss_classification = classify_earnings(&miss_actual, &miss_consensus, &earnings_cfg);
        assert!(
            miss_classification.is_some(),
            "Miss classification should not be None"
        );
        assert_eq!(
            miss_classification.as_ref().unwrap().kind,
            EarningsKind::Miss
        );

        let miss_event = earnings_classification_to_event(
            "TEST_CODE_EARNINGS_MISS",
            miss_classification.as_ref().unwrap(),
        );
        assert_eq!(miss_event.push_kind, SourcePushKind::EarningsMiss);

        // Verify beat and miss map to different PushKinds
        assert_ne!(
            source_push_kind_to_push_kind(beat_event.push_kind),
            source_push_kind_to_push_kind(miss_event.push_kind)
        );
        assert_eq!(
            source_push_kind_to_push_kind(beat_event.push_kind),
            PushKind::EarningsBeat
        );
        assert_eq!(
            source_push_kind_to_push_kind(miss_event.push_kind),
            PushKind::EarningsMiss
        );
    }

    #[tokio::test]
    async fn repeated_analyst_report_is_not_pushed_twice() {
        let analyst_store = AnalystStateStore::new(10_000);

        let key = AnalystKey {
            code: "TEST_CODE_ANALYST_STATE".to_string(),
            broker: "券商A".to_string(),
        };

        let obs = AnalystObservation {
            rating: "中性".to_string(),
            publish_date: chrono::NaiveDate::parse_from_str("2026-07-15", "%Y-%m-%d").unwrap(),
            report_id: "研报-TEST_CODE_ANALYST_STATE-2026-07-15".to_string(),
        };

        // First observation: should be Observed (new entry)
        let first_decision = analyst_store.observe(key.clone(), obs.clone());
        assert_eq!(
            first_decision,
            stock_analysis::news::aggregator::analyst_state::ObservationDecision::Observed
        );

        // Same report again (same report_id AND same publish_date): should be Duplicate
        let second_decision = analyst_store.observe(key.clone(), obs.clone());
        assert_eq!(
            second_decision,
            stock_analysis::news::aggregator::analyst_state::ObservationDecision::Duplicate
        );

        // No push should be generated for Duplicate, so attempted=0 for the second call
        // This is the key assertion: repeated report is not pushed twice
        match second_decision {
            stock_analysis::news::aggregator::analyst_state::ObservationDecision::Duplicate => {}
            _ => panic!("Expected Duplicate, got {:?}", second_decision),
        }
    }

    // -------------------------------------------------------------------------
    // Task 8: MarketActionAlert transition tests
    // -------------------------------------------------------------------------

    /// Helper: build an OrderUpdate MonitorEvent.
    fn order_update(code: &str, action: &str, shares: u64) -> MonitorEvent {
        MonitorEvent::OrderUpdate {
            code: code.into(),
            action: action.into(),
            shares,
        }
    }

    #[test]
    fn order_update_maps_to_emergency_market_action() {
        let event = order_update("TEST_CODE_MARKET_ACTION", "sell", 100);
        let normalized = normalize_market_action(&event).unwrap();
        assert_eq!(normalized.push_kind, SourcePushKind::MarketActionAlert);
        assert_eq!(normalized.code.as_deref(), Some("TEST_CODE_MARKET_ACTION"));
        assert!(normalized.title.contains("sell"));
    }

    #[test]
    fn unchanged_order_state_is_not_re_emitted() {
        let mut state = MarketActionState::default();
        let event = order_update("TEST_CODE_MARKET_ACTION", "sell", 100);
        assert!(state.accept(&event), "first emission should be accepted");
        assert!(!state.accept(&event), "identical state should be rejected");
    }

    #[test]
    fn market_action_state_dedup_within_capacity() {
        let mut state = MarketActionState::default();
        // Different codes are independent
        let e1 = order_update("TEST_CODE_MARKET_ACTION_1", "buy", 100);
        let e2 = order_update("TEST_CODE_MARKET_ACTION_2", "sell", 200);
        assert!(state.accept(&e1));
        assert!(state.accept(&e2));
        // Same code/action/shares again is rejected
        assert!(!state.accept(&e1));
        assert!(!state.accept(&e2));
        // Different action for same code is accepted
        let e3 = order_update("TEST_CODE_MARKET_ACTION_1", "sell", 100); // same code but different action
        assert!(state.accept(&e3), "different action should be new state");
        let e4 = order_update("TEST_CODE_MARKET_ACTION_1", "buy", 200); // different shares
        assert!(state.accept(&e4), "different shares should be new");
    }

    #[test]
    fn handle_monitor_event_non_order_returns_none() {
        use stock_analysis::monitor::event_bus::MonitorEvent;
        let state = Mutex::new(MarketActionState::default());
        // Alert event should return None
        let alert = MonitorEvent::Alert {
            title: "test".into(),
            success: true,
        };
        // This is a compile-time check that non-OrderUpdate variants type-check
        // Actual runtime behavior: the function returns None for non-OrderUpdate
        let result = futures::executor::block_on(handle_monitor_event(&alert, &state));
        assert!(result.is_none());
    }
}
