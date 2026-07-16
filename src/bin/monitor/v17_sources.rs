//! v17.7 Task 5: Monitor-only source-to-push adapter
//!
//! Consumes `NormalizedSourceEvent` from the news aggregator and dispatches
//! exactly one `push_governor_v3` call per event. No retry, no fallback PushKind.

use crate::notify::{self, PushKind, PushOutcome};
use stock_analysis::news::aggregator::{NormalizedSourceEvent, SourcePushKind};

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
    let rendered = render_message(&event);
    let outcome = notify::push_governor_v3(&rendered, kind, code_str.as_deref()).await;
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
        if event.title.is_empty() || event.event_id.is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use stock_analysis::signal::market_event::Direction;
    use chrono::Local;

    /// Returns a valid announcement event for testing.
    fn test_announcement_event() -> NormalizedSourceEvent {
        NormalizedSourceEvent {
            push_kind: SourcePushKind::Announcement,
            event_id: "ann-1".into(),
            code: Some("600519".into()),
            title: "关于回购股份方案的公告".into(),
            summary: "回购".into(),
            direction: Direction::Neutral,
            strength: 50,
            certainty: 50,
            occurred_at: Local::now(),
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
            code: Some("600519".into()),
            title: "".into(),
            summary: "".into(),
            direction: Direction::Neutral,
            strength: 50,
            certainty: 50,
            occurred_at: Local::now(),
            source: "test".into(),
            url: None,
            metadata: Default::default(),
        }
    }

    #[tokio::test]
    async fn announcement_adapter_calls_only_announcement_kind() {
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        let report = push_normalized_events(vec![test_announcement_event()]).await;
        assert_eq!(report.attempted, 1);
        assert_eq!(report.pushed, 1);
        assert_eq!(report.failed, 0);
        std::env::remove_var("V10_DRY_RUN_PUSH");
    }

    #[tokio::test]
    async fn missing_source_identity_is_skipped_not_pushed() {
        let report = push_normalized_events(vec![test_event_with_empty_title()]).await;
        assert_eq!(report.skipped, 1);
        assert_eq!(report.pushed, 0);
    }

    #[tokio::test]
    async fn analyst_upgrade_maps_to_analyst_upgrade_kind() {
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        let event = NormalizedSourceEvent {
            push_kind: SourcePushKind::AnalystUpgrade,
            event_id: "analyst-1".into(),
            code: Some("000858".into()),
            title: "券商上调评级".into(),
            summary: "上调至买入".into(),
            direction: Direction::Bull,
            strength: 70,
            certainty: 80,
            occurred_at: Local::now(),
            source: " Wind".into(),
            url: None,
            metadata: Default::default(),
        };
        let attempt = push_normalized_event(event).await;
        assert_eq!(attempt.kind, PushKind::AnalystUpgrade);
        assert_eq!(attempt.outcome, PushOutcome::Pushed);
        std::env::remove_var("V10_DRY_RUN_PUSH");
    }

    #[tokio::test]
    async fn policy_hit_with_no_code_is_pushed_as_policy() {
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
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
            source: "ndrc".into(),
            url: Some("https://example.invalid/pol-1".into()),
            metadata: Default::default(),
        };
        let attempt = push_normalized_event(event).await;
        assert_eq!(attempt.kind, PushKind::PolicyHit);
        assert!(attempt.code.is_none());
        assert_eq!(attempt.outcome, PushOutcome::Pushed);
        std::env::remove_var("V10_DRY_RUN_PUSH");
    }
}
