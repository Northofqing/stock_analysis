//! BR-196 V2 closed template-family manifest.
//!
//! This module owns lifecycle/coverage validation only.  Fixture rendering,
//! governance smoke and the opt-in transport are deliberately separate phases
//! so a manifest failure cannot touch a database, sink or external process.

use crate::notify::PushKind;
use crate::presentation_registry::ProductionPresentationDescriptor;
use chrono::{TimeZone, Timelike};
use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub(super) const MANIFEST_VERSION: &str = "BR196_V2";
const FIXED_DISABLED_REASON: &str = "template_contract_not_live_admitted";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Activation {
    NotAdmittedInManifestVersion { manifest_version: &'static str },
    NewsFlashProcessCapability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Lifecycle {
    Active,
    Disabled {
        reason_code: &'static str,
        activation: Activation,
    },
    Retired {
        reason_code: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TemplateFamily {
    pub family_key: &'static str,
    pub template_id: Option<&'static str>,
    pub push_kind: PushKind,
    pub lifecycle: Lifecycle,
    pub producer_seam_id: Option<&'static str>,
    pub renderer_or_assembler_seam_id: Option<&'static str>,
    pub has_fixture_builder: bool,
    pub manifest_version: &'static str,
    pub ordinal: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LifecycleCounts {
    pub active: usize,
    pub disabled: usize,
    pub retired: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ValidatedManifest {
    pub families: Vec<TemplateFamily>,
    pub family_counts: LifecycleCounts,
    pub push_kind_counts: LifecycleCounts,
    pub manifest_sha256: String,
    pub news_capability_generation: u64,
    pub news_capability_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NewsFlashProcessCapabilitySnapshot {
    pub generation: u64,
    pub selection_v2_enabled: bool,
    pub selection_v2_activation_receipt_sha256: String,
    pub registered_feed_set_sha256: String,
    pub registered_feed_count: usize,
    pub capability_sha256: String,
}

static CURRENT_NEWS_PROCESS_CAPABILITY: Lazy<Mutex<Option<NewsFlashProcessCapabilitySnapshot>>> =
    Lazy::new(|| Mutex::new(None));

impl NewsFlashProcessCapabilitySnapshot {
    pub(super) fn new(
        generation: u64,
        selection_v2_enabled: bool,
        selection_v2_activation_receipt_sha256: impl Into<String>,
        registered_feed_set_sha256: impl Into<String>,
        registered_feed_count: usize,
    ) -> Result<Self, String> {
        let selection_hash = selection_v2_activation_receipt_sha256.into();
        let feed_hash = registered_feed_set_sha256.into();
        if generation == 0 {
            return Err("BR-196 NewsFlash capability generation must be nonzero".to_string());
        }
        validate_sha256("selection_v2_activation_receipt_sha256", &selection_hash)?;
        validate_sha256("registered_feed_set_sha256", &feed_hash)?;
        let canonical = format!(
            "version={MANIFEST_VERSION}\ngeneration={generation}\nselection_v2_enabled={selection_v2_enabled}\nselection_receipt_sha256={selection_hash}\nregistered_feed_set_sha256={feed_hash}\nregistered_feed_count={registered_feed_count}\n"
        );
        let capability_sha256 = sha256_domain(
            "stock_analysis.br196.news_flash_process_capability.v1",
            canonical.as_bytes(),
        );
        Ok(Self {
            generation,
            selection_v2_enabled,
            selection_v2_activation_receipt_sha256: selection_hash,
            registered_feed_set_sha256: feed_hash,
            registered_feed_count,
            capability_sha256,
        })
    }

    fn news_lifecycle(&self) -> Lifecycle {
        let reason_code = match (self.selection_v2_enabled, self.registered_feed_count > 0) {
            (true, true) => return Lifecycle::Active,
            (false, false) => {
                "selection_v2_activation_not_released_and_global_news_pipeline_not_initialized"
            }
            (false, true) => "selection_v2_activation_not_released",
            (true, false) => "global_news_pipeline_not_initialized",
        };
        Lifecycle::Disabled {
            reason_code,
            activation: Activation::NewsFlashProcessCapability,
        }
    }

    pub(super) fn require_unchanged(&self, current: Option<&Self>) -> Result<(), String> {
        let current = current
            .ok_or_else(|| "BR-196 NewsFlash capability missing at batch use-site".to_string())?;
        if current.generation < self.generation {
            return Err("BR-196 NewsFlash capability generation rolled back".to_string());
        }
        if current.generation != self.generation
            || current.capability_sha256 != self.capability_sha256
        {
            return Err("BR-196 NewsFlash capability changed during invocation".to_string());
        }
        Ok(())
    }
}

pub(super) fn capture_news_process_capability(
    selection_v2_enabled: bool,
    registered_feed_count: usize,
    registered_feed_set_sha256: impl Into<String>,
) -> Result<NewsFlashProcessCapabilitySnapshot, String> {
    static GENERATION: AtomicU64 = AtomicU64::new(1);
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst);
    let selection_receipt = sha256_domain(
        "stock_analysis.br196.selection_v2_activation_receipt.v1",
        format!("selection_v2_enabled={selection_v2_enabled}").as_bytes(),
    );
    let snapshot = NewsFlashProcessCapabilitySnapshot::new(
        generation,
        selection_v2_enabled,
        selection_receipt,
        registered_feed_set_sha256,
        registered_feed_count,
    )?;
    *CURRENT_NEWS_PROCESS_CAPABILITY
        .lock()
        .map_err(|_| "BR-196 NewsFlash capability owner lock poisoned".to_string())? =
        Some(snapshot.clone());
    Ok(snapshot)
}

pub(super) fn current_news_process_capability() -> Option<NewsFlashProcessCapabilitySnapshot> {
    CURRENT_NEWS_PROCESS_CAPABILITY
        .lock()
        .ok()
        .and_then(|snapshot| snapshot.clone())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GovernanceSmokeDisposition {
    pub family_key: &'static str,
    pub push_kind: PushKind,
    pub outcome: crate::notify::PushOutcome,
}

/// Invocation-scoped governance clock for the BR-196 exact-six smoke.
///
/// This value is deliberately passed through the call stack. It is never
/// installed in process-wide environment, global or thread-local state, so an
/// acceptance run cannot relax ordinary test or production quiet-hour policy.
#[derive(Debug)]
pub(super) struct GovernanceSmokeContext {
    now: chrono::DateTime<chrono::Local>,
}

#[derive(Debug)]
pub(super) struct GovernanceSmokeDispatch<'context> {
    context: &'context GovernanceSmokeContext,
    family_key: &'static str,
    push_kind: PushKind,
    code: Option<TestSecurityIdentity>,
}

const GOVERNANCE_SMOKE_IDENTITIES: [(&str, PushKind); 6] = [
    ("D-01-news-to-idea", PushKind::NewsToIdea),
    ("I-02-news-catalyst", PushKind::NewsCatalyst),
    ("P-01-preopen-news-hot", PushKind::PreopenNewsHot),
    ("T-11-auction-volume", PushKind::AuctionVolume),
    ("R-03-industry-chain", PushKind::IndustryChain),
    ("A-10-catalyst-review", PushKind::CatalystReview),
];

impl GovernanceSmokeContext {
    pub(super) fn for_review_date(review_date: chrono::NaiveDate) -> Result<Self, String> {
        if stock_analysis::risk::env_guard::current_env()
            != stock_analysis::risk::env_guard::TradingEnv::Test
        {
            return Err("BR-196 governance smoke context requires Test environment".to_string());
        }
        let local_noon = review_date
            .and_hms_opt(12, 0, 0)
            .ok_or_else(|| "BR-196 governance smoke review date is invalid".to_string())?;
        let now = chrono::Local
            .from_local_datetime(&local_noon)
            .single()
            .ok_or_else(|| "BR-196 governance smoke noon is not a unique local time".to_string())?;
        if now.offset().local_minus_utc() != 8 * 60 * 60 {
            return Err("BR-196 governance smoke requires Asia/Shanghai +08:00".to_string());
        }
        Ok(Self { now })
    }

    pub(super) fn dispatch(
        &self,
        family_key: &'static str,
        push_kind: PushKind,
        code: Option<&str>,
    ) -> Result<GovernanceSmokeDispatch<'_>, String> {
        validate_governance_smoke_identity(family_key, push_kind)?;
        let code = code.map(TestSecurityIdentity::parse).transpose()?;
        Ok(GovernanceSmokeDispatch {
            context: self,
            family_key,
            push_kind,
            code,
        })
    }
}

impl GovernanceSmokeDispatch<'_> {
    pub(super) fn push_kind(&self) -> PushKind {
        self.push_kind
    }

    pub(super) fn code(&self) -> Option<&str> {
        self.code.as_ref().map(TestSecurityIdentity::as_str)
    }

    pub(super) fn governance_now(&self) -> chrono::DateTime<chrono::Local> {
        self.context.now
    }

    pub(super) fn validate_for_use(&self) -> Result<(), String> {
        if stock_analysis::risk::env_guard::current_env()
            != stock_analysis::risk::env_guard::TradingEnv::Test
        {
            return Err("BR-196 governance smoke dispatch requires Test environment".to_string());
        }
        validate_governance_smoke_identity(self.family_key, self.push_kind)?;
        if self.context.now.hour() != 12
            || self.context.now.minute() != 0
            || self.context.now.second() != 0
            || self.context.now.offset().local_minus_utc() != 8 * 60 * 60
        {
            return Err("BR-196 governance smoke clock contract drift".to_string());
        }
        Ok(())
    }
}

fn validate_governance_smoke_identity(family_key: &str, push_kind: PushKind) -> Result<(), String> {
    if GOVERNANCE_SMOKE_IDENTITIES.contains(&(family_key, push_kind)) {
        Ok(())
    } else {
        Err("BR-196 governance smoke tuple is not admitted".to_string())
    }
}

pub(super) fn validate_governance_smoke(
    observed: &[GovernanceSmokeDisposition],
) -> Result<(), String> {
    if observed.len() != GOVERNANCE_SMOKE_IDENTITIES.len() {
        return Err(format!(
            "BR-196 governance smoke cardinality mismatch: {}",
            observed.len()
        ));
    }
    let expected = GOVERNANCE_SMOKE_IDENTITIES
        .into_iter()
        .collect::<HashSet<_>>();
    let actual = observed
        .iter()
        .map(|item| (item.family_key, item.push_kind))
        .collect::<HashSet<_>>();
    if actual.len() != observed.len() || actual != expected {
        return Err("BR-196 governance smoke identity multiset mismatch".to_string());
    }
    if let Some(failed) = observed
        .iter()
        .find(|item| item.outcome != crate::notify::PushOutcome::Pushed)
    {
        return Err(format!(
            "BR-196 governance smoke non-Pushed family={} outcome={:?}",
            failed.family_key, failed.outcome
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct TestSecurityIdentity(String);

impl TestSecurityIdentity {
    pub(super) fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty()
            || value.trim() != value
            || !value.starts_with("TEST_CODE_")
            || value.chars().any(char::is_whitespace)
        {
            return Err("BR-196 security identity is not canonical TEST_CODE".to_string());
        }
        let suffix = &value["TEST_CODE_".len()..];
        if suffix.is_empty() || suffix.chars().all(|ch| ch.is_ascii_digit()) && suffix.len() == 6 {
            return Err("BR-196 production-like six-digit security identity rejected".to_string());
        }
        if !suffix
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Err("BR-196 security identity contains an alias separator".to_string());
        }
        Ok(Self(value))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

type PresentationTuple = (&'static str, PushKind, &'static str, &'static str);

// This inventory is the test manifest authority.  The production registry
// below intentionally duplicates the canonical tuples instead of deriving
// them, allowing either side to drift and the bijection test to catch it.
const ACTIVE_PRESENTATIONS: [PresentationTuple; 52] = [
    (
        "T-01-account-mode",
        PushKind::AccountMode,
        "account_mode_hook",
        "render_account_mode",
    ),
    (
        "T-02-data-mode",
        PushKind::DataMode,
        "data_mode_hook",
        "render_data_mode",
    ),
    (
        "T-02-data-mode-reminder",
        PushKind::DataMode,
        "data_mode_scheduler",
        "render_data_mode_reminder",
    ),
    (
        "T-03-holding-plan",
        PushKind::HoldingPlan,
        "holding_plan_dispatcher",
        "render_holding_plan",
    ),
    (
        "T-04-holding-event",
        PushKind::HoldingEvent,
        "holding_event_dispatcher",
        "render_holding_event",
    ),
    (
        "T-05-t0-advice",
        PushKind::T0Advice,
        "t0_dispatcher",
        "render_t0_advice",
    ),
    (
        "T-06-t0-forbid",
        PushKind::T0Advice,
        "t0_dispatcher",
        "render_t0_forbid",
    ),
    (
        "T-07-candidate-triggered",
        PushKind::CandidateTriggered,
        "candidate_dispatcher",
        "render_candidate_triggered",
    ),
    (
        "T-08-candidate-invalidated",
        PushKind::CandidateInvalidated,
        "candidate_dispatcher",
        "render_candidate_invalidated",
    ),
    (
        "T-09-forbidden-ops",
        PushKind::ForbiddenOps,
        "holding_risk_dispatcher",
        "render_forbidden_ops",
    ),
    (
        "P-05-virtual-watch",
        PushKind::VirtualWatch,
        "virtual_watch_dispatcher",
        "render_virtual_watch",
    ),
    (
        "T-10-paper-trade",
        PushKind::PaperTrade,
        "paper_trade_dispatcher",
        "render_paper_trade",
    ),
    (
        "T-11-auction-volume",
        PushKind::AuctionVolume,
        "auction_volume_dispatcher",
        "render_auction_volume",
    ),
    (
        "T-12-close-call",
        PushKind::CloseCall,
        "close_call_dispatcher",
        "render_close_call",
    ),
    (
        "I-09-sector-top",
        PushKind::SectorTop,
        "sector_top_dispatcher",
        "render_sector_top",
    ),
    (
        "T-13-turnover-top",
        PushKind::TurnoverTop,
        "turnover_top_dispatcher",
        "render_turnover_top",
    ),
    (
        "R-01-daily-report",
        PushKind::DailyReport,
        "daily_report_dispatcher",
        "render_daily_report",
    ),
    (
        "R-02-review-market",
        PushKind::ReviewMarket,
        "review_market_dispatcher",
        "render_review_market",
    ),
    (
        "R-03-industry-chain",
        PushKind::IndustryChain,
        "industry_chain_review_dispatcher",
        "render_industry_chain",
    ),
    (
        "R-04-review-lhb-gateway",
        PushKind::ReviewLhb,
        "review_lhb_gateway_dispatcher",
        "render_review_lhb_gateway",
    ),
    (
        "R-05-review-signal",
        PushKind::ReviewSignal,
        "review_signal_dispatcher",
        "render_review_signal",
    ),
    (
        "R-06-review-failure",
        PushKind::ReviewFailure,
        "review_failure_dispatcher",
        "render_review_failure",
    ),
    (
        "R-07-tomorrow-watch",
        PushKind::TomorrowWatch,
        "tomorrow_watch_dispatcher",
        "render_tomorrow_watch",
    ),
    (
        "R-09-provider-top-n",
        PushKind::ReviewProviderTopN,
        "provider_top_n_dispatcher",
        "render_r09_provider_top_n",
    ),
    (
        "R-11-position-review",
        PushKind::PositionReview,
        "position_review_dispatcher",
        "render_position_review",
    ),
    (
        "A-02-auction-repush",
        PushKind::AuctionRepush,
        "auction_repush_dispatcher",
        "render_auction_repush",
    ),
    (
        "P-05-candidate-board",
        PushKind::CandidateBoard,
        "candidate_board_dispatcher",
        "format_candidate_board",
    ),
    (
        "A-11-ipo-catalyst",
        PushKind::IpoCatalyst,
        "ipo_catalyst_dispatcher",
        "render_ipo_catalyst",
    ),
    (
        "P-01-preopen-news-hot",
        PushKind::PreopenNewsHot,
        "preopen_news_dispatcher",
        "render_preopen_news_hot",
    ),
    (
        "I-01-intraday-market",
        PushKind::IntradayMarket,
        "intraday_market_dispatcher",
        "render_intraday_market",
    ),
    (
        "I-02-news-catalyst",
        PushKind::NewsCatalyst,
        "news_catalyst_dispatcher",
        "render_news_catalyst",
    ),
    (
        "I-09-sector-anomaly",
        PushKind::SectorAnomaly,
        "sector_anomaly_dispatcher",
        "render_sector_anomaly",
    ),
    (
        "D-01-news-to-idea",
        PushKind::NewsToIdea,
        "news_to_idea_dispatcher",
        "render_news_to_idea",
    ),
    (
        "A-10-catalyst-review",
        PushKind::CatalystReview,
        "catalyst_review_dispatcher",
        "render_catalyst_review",
    ),
    (
        "I-03-industry-chain-intraday",
        PushKind::IndustryChainIntraday,
        "industry_chain_intraday_dispatcher",
        "render_industry_chain_intraday",
    ),
    (
        "T-14-post-fixed-price-order",
        PushKind::PostFixedPriceOrder,
        "post_fixed_price_dispatcher",
        "render_post_fixed_price_order",
    ),
    (
        "T-15-post-fixed-price-fill",
        PushKind::PostFixedPriceFill,
        "post_fixed_price_dispatcher",
        "render_post_fixed_price_fill",
    ),
    (
        "T-16-st-price-limit-changed",
        PushKind::StPriceLimitChanged,
        "st_price_limit_dispatcher",
        "render_st_price_limit_changed",
    ),
    (
        "T-17-etf-closing-call-auction",
        PushKind::EtfClosingCallAuction,
        "etf_closing_call_dispatcher",
        "render_etf_closing_call_auction",
    ),
    (
        "BR-033-block-trade-confirm",
        PushKind::BlockTradeIntradayConfirm,
        "block_trade_dispatcher",
        "render_block_trade_intraday_confirm",
    ),
    (
        "BR-034-block-trade-range",
        PushKind::BlockTradePriceRange,
        "block_trade_dispatcher",
        "render_block_trade_price_range",
    ),
    (
        "A-01-paper-review",
        PushKind::PaperReview,
        "paper_review_dispatcher",
        "render_paper_review",
    ),
    (
        "R-08-public-event-calendar",
        PushKind::EventCalendar,
        "dispatch_r08_event_calendar_outcome",
        "render_r08_public_calendar",
    ),
    (
        "L-01-limit-boards-first",
        PushKind::LimitBoards,
        "monitor_limit_board_producer",
        "assemble_limit_boards_first",
    ),
    (
        "L-02-limit-boards-second",
        PushKind::LimitBoards,
        "monitor_limit_board_producer",
        "assemble_limit_boards_second",
    ),
    (
        "L-03-limit-boards-third-plus",
        PushKind::LimitBoards,
        "monitor_limit_board_producer",
        "assemble_limit_boards_third_plus",
    ),
    (
        "S-01-announcement",
        PushKind::Announcement,
        "v17_source_dispatcher",
        "v17_sources_render_message_announcement",
    ),
    (
        "S-02-policy-hit",
        PushKind::PolicyHit,
        "v17_source_dispatcher",
        "v17_sources_render_message_policy_hit",
    ),
    (
        "S-03-earnings-beat",
        PushKind::EarningsBeat,
        "v17_source_dispatcher",
        "v17_sources_render_message_earnings_beat",
    ),
    (
        "S-04-earnings-miss",
        PushKind::EarningsMiss,
        "v17_source_dispatcher",
        "v17_sources_render_message_earnings_miss",
    ),
    (
        "S-05-analyst-upgrade",
        PushKind::AnalystUpgrade,
        "v17_source_dispatcher",
        "v17_sources_render_message_analyst_upgrade",
    ),
    (
        "S-06-market-action-alert",
        PushKind::MarketActionAlert,
        "v17_source_dispatcher",
        "v17_sources_render_message_market_action_alert",
    ),
];

const NEWS_PRESENTATIONS: [PresentationTuple; 2] = [
    (
        "N-01-news-flash-critical",
        PushKind::NewsFlashCritical,
        "news_flash_critical_dispatcher",
        "assemble_news_flash_critical",
    ),
    (
        "N-02-news-flash-aggregated",
        PushKind::NewsFlashAggregated,
        "news_flash_aggregate_dispatcher",
        "assemble_news_flash_aggregated",
    ),
];

const FIXED_DISABLED_KINDS: [PushKind; 11] = [
    PushKind::FundInflow,
    PushKind::FactorIC,
    PushKind::SectorTier,
    PushKind::CapitalVerify,
    PushKind::WeeklySOP,
    PushKind::StockPick,
    PushKind::CandidateBoard,
    PushKind::NewsRanked,
    PushKind::IpoListingApproval,
    PushKind::IpoProspectus,
    PushKind::IpoCatalyst,
];

const ALL_PUSH_KINDS: [PushKind; 59] = [
    PushKind::HoldingEvent,
    PushKind::DailyReport,
    PushKind::Announcement,
    PushKind::AuctionVolume,
    PushKind::VirtualWatch,
    PushKind::LimitBoards,
    PushKind::SectorTop,
    PushKind::FundInflow,
    PushKind::AuctionRepush,
    PushKind::FactorIC,
    PushKind::SectorTier,
    PushKind::CapitalVerify,
    PushKind::WeeklySOP,
    PushKind::StockPick,
    PushKind::IndustryChain,
    PushKind::TurnoverTop,
    PushKind::CandidateBoard,
    PushKind::NewsRanked,
    PushKind::AccountMode,
    PushKind::DataMode,
    PushKind::HoldingPlan,
    PushKind::T0Advice,
    PushKind::CandidateTriggered,
    PushKind::ForbiddenOps,
    PushKind::PaperTrade,
    PushKind::CloseCall,
    PushKind::ReviewMarket,
    PushKind::ReviewLhb,
    PushKind::ReviewSignal,
    PushKind::ReviewFailure,
    PushKind::TomorrowWatch,
    PushKind::EventCalendar,
    PushKind::ReviewProviderTopN,
    PushKind::PositionReview,
    PushKind::PreopenNewsHot,
    PushKind::IntradayMarket,
    PushKind::NewsCatalyst,
    PushKind::SectorAnomaly,
    PushKind::NewsToIdea,
    PushKind::CatalystReview,
    PushKind::IndustryChainIntraday,
    PushKind::PostFixedPriceOrder,
    PushKind::PostFixedPriceFill,
    PushKind::StPriceLimitChanged,
    PushKind::EtfClosingCallAuction,
    PushKind::BlockTradeIntradayConfirm,
    PushKind::BlockTradePriceRange,
    PushKind::PaperReview,
    PushKind::CandidateInvalidated,
    PushKind::IpoListingApproval,
    PushKind::IpoProspectus,
    PushKind::IpoCatalyst,
    PushKind::PolicyHit,
    PushKind::EarningsBeat,
    PushKind::EarningsMiss,
    PushKind::AnalystUpgrade,
    PushKind::MarketActionAlert,
    PushKind::NewsFlashCritical,
    PushKind::NewsFlashAggregated,
];

pub(super) fn build_validated_manifest(
    news: &NewsFlashProcessCapabilitySnapshot,
) -> Result<ValidatedManifest, String> {
    let families = build_manifest(news);
    validate_manifest(families, news)
}

pub(super) fn build_active_catalog(
    date: &str,
    hhmm: &str,
    manifest: &ValidatedManifest,
) -> Result<Vec<crate::push_templates::TestTemplatePreview>, String> {
    let mut by_id = crate::push_templates::build_test_template_catalog(date, hhmm)?
        .into_iter()
        .map(|preview| (preview.template_id, preview))
        .collect::<HashMap<_, _>>();
    if manifest.families.iter().any(|row| {
        row.family_key == "N-01-news-flash-critical" && matches!(row.lifecycle, Lifecycle::Active)
    }) {
        let identity = TestSecurityIdentity::parse("TEST_CODE_NEWS_FLASH_ALPHA")?;
        by_id.insert(
            "N-01-news-flash-critical",
            crate::push_templates::TestTemplatePreview {
                template_id: "N-01-news-flash-critical",
                text: crate::news_aggregator_init::assemble_news_flash_critical(
                    hhmm,
                    "TEST_CODE 事件",
                    &format!("TEST_CODE 新闻 {}", identity.as_str()),
                    90,
                    80,
                    1,
                    10,
                ),
            },
        );
        by_id.insert(
            "N-02-news-flash-aggregated",
            crate::push_templates::TestTemplatePreview {
                template_id: "N-02-news-flash-aggregated",
                text: crate::news_aggregator_init::assemble_news_flash_aggregated(
                    hhmm,
                    &[format!("TEST_CODE 聚合新闻 {}", identity.as_str())],
                )?,
            },
        );
    }
    let mut ordered = Vec::with_capacity(manifest.family_counts.active);
    for family in manifest
        .families
        .iter()
        .filter(|family| matches!(family.lifecycle, Lifecycle::Active))
    {
        let template_id = family.template_id.ok_or_else(|| {
            format!(
                "BR-196 Active family missing template: {}",
                family.family_key
            )
        })?;
        let preview = by_id.remove(template_id).ok_or_else(|| {
            format!("BR-196 Active family missing rendered preview: {template_id}")
        })?;
        ordered.push(preview);
    }
    if !by_id.is_empty() {
        return Err(format!(
            "BR-196 renderer emitted non-Active families: {:?}",
            by_id.keys().collect::<Vec<_>>()
        ));
    }
    if ordered.len() != manifest.family_counts.active {
        return Err("BR-196 rendered family count differs from Active manifest".to_string());
    }
    Ok(ordered)
}

fn build_manifest(news: &NewsFlashProcessCapabilitySnapshot) -> Vec<TemplateFamily> {
    let mut rows = Vec::with_capacity(64);
    for &(family_key, push_kind, producer, renderer) in &ACTIVE_PRESENTATIONS {
        rows.push(presentation_family(
            rows.len(),
            family_key,
            push_kind,
            Lifecycle::Active,
            producer,
            renderer,
        ));
    }
    let news_lifecycle = news.news_lifecycle();
    for &(family_key, push_kind, producer, renderer) in &NEWS_PRESENTATIONS {
        rows.push(presentation_family(
            rows.len(),
            family_key,
            push_kind,
            news_lifecycle.clone(),
            producer,
            renderer,
        ));
    }
    for (index, push_kind) in FIXED_DISABLED_KINDS.into_iter().enumerate() {
        rows.push(TemplateFamily {
            family_key: [
                "M-01-fund-inflow",
                "M-02-factor-ic",
                "M-03-sector-tier",
                "M-04-capital-verify",
                "M-05-weekly-sop",
                "M-06-stock-pick",
                "M-07-candidate-board",
                "M-08-news-ranked",
                "M-09-ipo-listing-approval",
                "M-10-ipo-prospectus",
                "M-11-ipo-catalyst",
            ][index],
            template_id: None,
            push_kind,
            lifecycle: Lifecycle::Disabled {
                reason_code: FIXED_DISABLED_REASON,
                activation: Activation::NotAdmittedInManifestVersion {
                    manifest_version: MANIFEST_VERSION,
                },
            },
            producer_seam_id: None,
            renderer_or_assembler_seam_id: None,
            has_fixture_builder: false,
            manifest_version: MANIFEST_VERSION,
            ordinal: rows.len(),
        });
    }
    for (family_key, template_id, push_kind, reason_code) in [
        (
            "R-04-review-lhb-legacy",
            Some("R-04-review-lhb-legacy"),
            PushKind::ReviewLhb,
            "superseded_by_gateway_renderer",
        ),
        (
            "R-08-event-calendar",
            Some("R-08-event-calendar"),
            PushKind::EventCalendar,
            "superseded_by_public_source_only_renderer",
        ),
        (
            "X-01-auction-repush",
            None,
            PushKind::AuctionRepush,
            "production_call_deleted_v13_10_1",
        ),
    ] {
        rows.push(TemplateFamily {
            family_key,
            template_id,
            push_kind,
            lifecycle: Lifecycle::Retired { reason_code },
            producer_seam_id: None,
            renderer_or_assembler_seam_id: None,
            has_fixture_builder: false,
            manifest_version: MANIFEST_VERSION,
            ordinal: rows.len(),
        });
    }
    rows
}

fn presentation_family(
    ordinal: usize,
    family_key: &'static str,
    push_kind: PushKind,
    lifecycle: Lifecycle,
    producer_seam_id: &'static str,
    renderer_or_assembler_seam_id: &'static str,
) -> TemplateFamily {
    TemplateFamily {
        family_key,
        template_id: Some(family_key),
        push_kind,
        lifecycle,
        producer_seam_id: Some(producer_seam_id),
        renderer_or_assembler_seam_id: Some(renderer_or_assembler_seam_id),
        has_fixture_builder: true,
        manifest_version: MANIFEST_VERSION,
        ordinal,
    }
}

fn validate_manifest(
    families: Vec<TemplateFamily>,
    news: &NewsFlashProcessCapabilitySnapshot,
) -> Result<ValidatedManifest, String> {
    if families.len() != 68 {
        return Err(format!("BR-196 family total drift: {}", families.len()));
    }
    let mut family_keys = HashSet::new();
    let mut template_ids = HashSet::new();
    for (expected_ordinal, row) in families.iter().enumerate() {
        if row.ordinal != expected_ordinal || row.manifest_version != MANIFEST_VERSION {
            return Err("BR-196 manifest ordinal/version drift".to_string());
        }
        if row.family_key.trim().is_empty() || !family_keys.insert(row.family_key) {
            return Err("BR-196 family key missing or duplicated".to_string());
        }
        if let Some(template_id) = row.template_id {
            if template_id.trim().is_empty() || !template_ids.insert(template_id) {
                return Err("BR-196 template id missing or duplicated".to_string());
            }
        }
        match &row.lifecycle {
            Lifecycle::Active => validate_presentation_row(row)?,
            Lifecycle::Disabled {
                reason_code,
                activation,
            } => match activation {
                Activation::NewsFlashProcessCapability => {
                    if *reason_code != news.news_disabled_reason().unwrap_or_default() {
                        return Err("BR-196 NewsFlash disabled reason drift".to_string());
                    }
                    validate_presentation_row(row)?;
                }
                Activation::NotAdmittedInManifestVersion { manifest_version } => {
                    if *reason_code != FIXED_DISABLED_REASON
                        || *manifest_version != MANIFEST_VERSION
                        || row.template_id.is_some()
                        || row.producer_seam_id.is_some()
                        || row.renderer_or_assembler_seam_id.is_some()
                        || row.has_fixture_builder
                    {
                        return Err("BR-196 fixed Disabled row contract drift".to_string());
                    }
                }
            },
            Lifecycle::Retired { reason_code } => {
                if reason_code.trim().is_empty()
                    || row.producer_seam_id.is_some()
                    || row.renderer_or_assembler_seam_id.is_some()
                    || row.has_fixture_builder
                {
                    return Err("BR-196 Retired row has a production presentation".to_string());
                }
            }
        }
    }
    validate_descriptor_bijection(&families, crate::presentation_registry::descriptors())?;
    let family_counts = count_family_lifecycle(&families);
    let push_kind_counts = project_push_kind_lifecycle(&families)?;
    let expected = if news.selection_v2_enabled && news.registered_feed_count > 0 {
        (
            LifecycleCounts {
                active: 54,
                disabled: 11,
                retired: 3,
                total: 68,
            },
            LifecycleCounts {
                active: 50,
                disabled: 9,
                retired: 0,
                total: 59,
            },
        )
    } else {
        (
            LifecycleCounts {
                active: 52,
                disabled: 13,
                retired: 3,
                total: 68,
            },
            LifecycleCounts {
                active: 48,
                disabled: 11,
                retired: 0,
                total: 59,
            },
        )
    };
    if (family_counts, push_kind_counts) != expected {
        return Err(format!(
            "BR-196 lifecycle matrix drift: family={family_counts:?} kind={push_kind_counts:?}"
        ));
    }
    let canonical = families
        .iter()
        .map(|row| {
            format!(
                "{}|{}|{:?}|{:?}|{}|{}|{}",
                row.ordinal,
                row.family_key,
                row.push_kind,
                row.lifecycle,
                row.producer_seam_id.unwrap_or(""),
                row.renderer_or_assembler_seam_id.unwrap_or(""),
                row.template_id.unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(ValidatedManifest {
        families,
        family_counts,
        push_kind_counts,
        manifest_sha256: sha256_domain(
            "stock_analysis.br196.template_manifest.v2",
            canonical.as_bytes(),
        ),
        news_capability_generation: news.generation,
        news_capability_sha256: news.capability_sha256.clone(),
    })
}

impl NewsFlashProcessCapabilitySnapshot {
    fn news_disabled_reason(&self) -> Option<&'static str> {
        match (self.selection_v2_enabled, self.registered_feed_count > 0) {
            (true, true) => None,
            (false, false) => Some(
                "selection_v2_activation_not_released_and_global_news_pipeline_not_initialized",
            ),
            (false, true) => Some("selection_v2_activation_not_released"),
            (true, false) => Some("global_news_pipeline_not_initialized"),
        }
    }
}

fn validate_presentation_row(row: &TemplateFamily) -> Result<(), String> {
    if row.template_id.is_none()
        || row.producer_seam_id.is_none_or(str::is_empty)
        || row.renderer_or_assembler_seam_id.is_none_or(str::is_empty)
        || !row.has_fixture_builder
    {
        return Err(format!(
            "BR-196 presentation row incomplete: {}",
            row.family_key
        ));
    }
    Ok(())
}

fn validate_descriptor_bijection(
    families: &[TemplateFamily],
    descriptors: &[ProductionPresentationDescriptor],
) -> Result<(), String> {
    if descriptors.len() != 54 {
        return Err("BR-196 production descriptor count must be 54".to_string());
    }
    let descriptor_set = descriptors.iter().copied().collect::<HashSet<_>>();
    if descriptor_set.len() != descriptors.len() {
        return Err("BR-196 duplicate production presentation descriptor".to_string());
    }
    let manifest_set = families
        .iter()
        .filter(|row| {
            matches!(row.lifecycle, Lifecycle::Active)
                || matches!(
                    row.lifecycle,
                    Lifecycle::Disabled {
                        activation: Activation::NewsFlashProcessCapability,
                        ..
                    }
                )
        })
        .map(|row| ProductionPresentationDescriptor {
            family_key: row.family_key,
            push_kind: row.push_kind,
            producer_seam_id: row.producer_seam_id.unwrap_or_default(),
            renderer_or_assembler_seam_id: row.renderer_or_assembler_seam_id.unwrap_or_default(),
        })
        .collect::<HashSet<_>>();
    if manifest_set != descriptor_set {
        return Err("BR-196 manifest/production descriptor bijection failed".to_string());
    }
    Ok(())
}

fn count_family_lifecycle(families: &[TemplateFamily]) -> LifecycleCounts {
    let mut counts = LifecycleCounts {
        active: 0,
        disabled: 0,
        retired: 0,
        total: families.len(),
    };
    for family in families {
        match family.lifecycle {
            Lifecycle::Active => counts.active += 1,
            Lifecycle::Disabled { .. } => counts.disabled += 1,
            Lifecycle::Retired { .. } => counts.retired += 1,
        }
    }
    counts
}

fn project_push_kind_lifecycle(families: &[TemplateFamily]) -> Result<LifecycleCounts, String> {
    let mut projected: HashMap<PushKind, LifecycleCounts> = HashMap::new();
    for row in families {
        let counts = projected.entry(row.push_kind).or_insert(LifecycleCounts {
            active: 0,
            disabled: 0,
            retired: 0,
            total: 0,
        });
        counts.total += 1;
        match row.lifecycle {
            Lifecycle::Active => counts.active += 1,
            Lifecycle::Disabled { .. } => counts.disabled += 1,
            Lifecycle::Retired { .. } => counts.retired += 1,
        }
    }
    let all = ALL_PUSH_KINDS.into_iter().collect::<HashSet<_>>();
    if all.len() != 59 || projected.len() != 59 || projected.keys().any(|kind| !all.contains(kind))
    {
        return Err("BR-196 PushKind inventory is not an exact 59-kind cover".to_string());
    }
    let mut result = LifecycleCounts {
        active: 0,
        disabled: 0,
        retired: 0,
        total: 59,
    };
    for counts in projected.values() {
        if counts.active > 0 {
            result.active += 1;
        } else if counts.disabled > 0 {
            result.disabled += 1;
        } else {
            result.retired += 1;
        }
    }
    Ok(result)
}

fn validate_sha256(field: &str, value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("BR-196 {field} is not a SHA-256 digest"));
    }
    Ok(())
}

fn sha256_domain(domain: &str, payload: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(payload);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(selection: bool, feeds: usize) -> NewsFlashProcessCapabilitySnapshot {
        NewsFlashProcessCapabilitySnapshot::new(7, selection, "1".repeat(64), "2".repeat(64), feeds)
            .unwrap()
    }

    #[test]
    fn br196_default_and_active_newsflash_matrices_are_exact() {
        let default = build_validated_manifest(&snapshot(false, 0)).unwrap();
        assert_eq!(
            default.family_counts,
            LifecycleCounts {
                active: 52,
                disabled: 13,
                retired: 3,
                total: 68
            }
        );
        assert_eq!(
            default.push_kind_counts,
            LifecycleCounts {
                active: 48,
                disabled: 11,
                retired: 0,
                total: 59
            }
        );

        let active = build_validated_manifest(&snapshot(true, 7)).unwrap();
        assert_eq!(
            active.family_counts,
            LifecycleCounts {
                active: 54,
                disabled: 11,
                retired: 3,
                total: 68
            }
        );
        assert_eq!(
            active.push_kind_counts,
            LifecycleCounts {
                active: 50,
                disabled: 9,
                retired: 0,
                total: 59
            }
        );
        assert_eq!(
            default
                .families
                .iter()
                .map(|row| row.ordinal)
                .collect::<Vec<_>>(),
            active
                .families
                .iter()
                .map(|row| row.ordinal)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn br196_newsflash_truth_table_has_exact_reasons() {
        let cases = [
            (
                false,
                0,
                Some(
                    "selection_v2_activation_not_released_and_global_news_pipeline_not_initialized",
                ),
            ),
            (false, 1, Some("selection_v2_activation_not_released")),
            (true, 0, Some("global_news_pipeline_not_initialized")),
            (true, 1, None),
        ];
        for (selection, feeds, expected_reason) in cases {
            let manifest = build_validated_manifest(&snapshot(selection, feeds)).unwrap();
            let news = manifest
                .families
                .iter()
                .filter(|row| row.family_key.starts_with("N-0"))
                .collect::<Vec<_>>();
            assert_eq!(news.len(), 2);
            for row in news {
                match (&row.lifecycle, expected_reason) {
                    (Lifecycle::Active, None) => {}
                    (
                        Lifecycle::Disabled {
                            reason_code,
                            activation: Activation::NewsFlashProcessCapability,
                        },
                        Some(expected),
                    ) => assert_eq!(*reason_code, expected),
                    other => panic!("unexpected NewsFlash lifecycle: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn br196_descriptor_registry_is_independent_exact_bijection() {
        let manifest = build_manifest(&snapshot(false, 0));
        assert_eq!(crate::presentation_registry::descriptors().len(), 54);
        validate_descriptor_bijection(&manifest, crate::presentation_registry::descriptors())
            .unwrap();

        let mut mutated = crate::presentation_registry::descriptors().to_vec();
        mutated[0].renderer_or_assembler_seam_id = "TEST_CODE_MUTATED_SEAM";
        assert!(validate_descriptor_bijection(&manifest, &mutated).is_err());
        mutated.pop();
        assert!(validate_descriptor_bijection(&manifest, &mutated).is_err());
    }

    #[test]
    fn br196_fixed_disabled_and_retired_rows_have_no_descriptor_or_fixture() {
        let manifest = build_validated_manifest(&snapshot(false, 0)).unwrap();
        let fixed = manifest
            .families
            .iter()
            .filter(|row| {
                matches!(
                    row.lifecycle,
                    Lifecycle::Disabled {
                        activation: Activation::NotAdmittedInManifestVersion { .. },
                        ..
                    }
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(fixed.len(), 11);
        assert!(fixed.iter().all(|row| row.template_id.is_none()
            && !row.has_fixture_builder
            && row.producer_seam_id.is_none()
            && row.renderer_or_assembler_seam_id.is_none()));
        let retired = manifest
            .families
            .iter()
            .filter(|row| matches!(row.lifecycle, Lifecycle::Retired { .. }))
            .collect::<Vec<_>>();
        assert_eq!(retired.len(), 3);
        assert!(retired.iter().all(|row| !row.has_fixture_builder
            && row.producer_seam_id.is_none()
            && row.renderer_or_assembler_seam_id.is_none()));
    }

    #[test]
    fn br196_test_security_identity_rejects_production_and_aliases() {
        assert!(TestSecurityIdentity::parse("TEST_CODE_ALPHA_01").is_ok());
        for invalid in [
            "",
            "600000",
            " TEST_CODE_ALPHA",
            "TEST_CODE_600000",
            "TEST_CODE_ALPHA-BETA",
            "TEST_CODE_ALPHA BETA",
        ] {
            assert!(
                TestSecurityIdentity::parse(invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn br196_news_capability_snapshot_rejects_drift_and_rollback() {
        let frozen = snapshot(true, 7);
        frozen.require_unchanged(Some(&frozen)).unwrap();
        assert!(frozen.require_unchanged(None).is_err());
        let mut rollback = frozen.clone();
        rollback.generation -= 1;
        assert!(frozen.require_unchanged(Some(&rollback)).is_err());
        let mut same_generation_different_hash = frozen.clone();
        same_generation_different_hash.capability_sha256 = "f".repeat(64);
        assert!(frozen
            .require_unchanged(Some(&same_generation_different_hash))
            .is_err());
    }

    #[test]
    fn br196_governance_smoke_requires_exact_six_pushed_tuples() {
        let valid = GOVERNANCE_SMOKE_IDENTITIES
            .into_iter()
            .map(|(family_key, push_kind)| GovernanceSmokeDisposition {
                family_key,
                push_kind,
                outcome: crate::notify::PushOutcome::Pushed,
            })
            .collect::<Vec<_>>();
        validate_governance_smoke(&valid).unwrap();

        let mut denied = valid.clone();
        denied[0].outcome = crate::notify::PushOutcome::Denied("TEST_CODE".to_string());
        assert!(validate_governance_smoke(&denied).is_err());
        let mut duplicate = valid.clone();
        duplicate[0].family_key = duplicate[1].family_key;
        duplicate[0].push_kind = duplicate[1].push_kind;
        assert!(validate_governance_smoke(&duplicate).is_err());
        assert!(validate_governance_smoke(&valid[..5]).is_err());
    }
}
