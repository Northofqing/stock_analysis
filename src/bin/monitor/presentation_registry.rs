//! Production-owned presentation descriptor registry (BR-196).
//!
//! Tokens can only be minted by exact tuple lookup.  Presentation dispatchers
//! carry the token to the notification gateway; test manifest code can inspect
//! descriptors but cannot construct tokens or descriptors.

use crate::notify::PushKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ProductionPresentationDescriptor {
    pub family_key: &'static str,
    pub push_kind: PushKind,
    pub producer_seam_id: &'static str,
    pub renderer_or_assembler_seam_id: &'static str,
}

#[derive(Debug)]
pub(super) struct ProductionPresentationToken {
    descriptor: &'static ProductionPresentationDescriptor,
}

impl ProductionPresentationToken {
    pub(super) fn descriptor(&self) -> &'static ProductionPresentationDescriptor {
        self.descriptor
    }
}

const fn descriptor(
    family_key: &'static str,
    push_kind: PushKind,
    producer_seam_id: &'static str,
    renderer_or_assembler_seam_id: &'static str,
) -> ProductionPresentationDescriptor {
    ProductionPresentationDescriptor {
        family_key,
        push_kind,
        producer_seam_id,
        renderer_or_assembler_seam_id,
    }
}

const PRODUCTION_PRESENTATION_DESCRIPTORS: [ProductionPresentationDescriptor; 58] = [
    descriptor(
        "T-01-account-mode",
        PushKind::AccountMode,
        "account_mode_hook",
        "render_account_mode",
    ),
    descriptor(
        "T-02-data-mode",
        PushKind::DataMode,
        "data_mode_hook",
        "render_data_mode",
    ),
    descriptor(
        "T-02-data-mode-reminder",
        PushKind::DataMode,
        "data_mode_scheduler",
        "render_data_mode_reminder",
    ),
    descriptor(
        "T-03-holding-plan",
        PushKind::HoldingPlan,
        "holding_plan_dispatcher",
        "render_holding_plan",
    ),
    descriptor(
        "T-04-holding-event",
        PushKind::HoldingEvent,
        "holding_event_dispatcher",
        "render_holding_event",
    ),
    descriptor(
        // 2026-08-07: 盘中指标告警 counted 接线 (BR-192 收尾) —
        // detector 12 类指标 (涨停突破/主力突袭/量比爆发/炸板等) 走 counted
        // binding 投递, 与 T-04 (持仓紧急风险) 共用 HoldingEvent kind 但独立
        // producer/renderer seam (BR-196 精确 tuple)。
        "T-04B-intraday-alert",
        PushKind::HoldingEvent,
        "intraday_alert_dispatcher",
        "render_intraday_alert",
    ),
    descriptor(
        // 交付物 A (2026-08-20): 虚拟盘绩效归因日推 — 15:05 归因闭环推送,
        // 文本由 performance::report::render_summary 生成并经透传模板进入投递链
        "A-12-attribution-daily",
        PushKind::AttributionDaily,
        "attribution_daily_dispatcher",
        "render_attribution_daily",
    ),
    descriptor(
        "T-05-t0-advice",
        PushKind::T0Advice,
        "t0_dispatcher",
        "render_t0_advice",
    ),
    descriptor(
        "T-06-t0-forbid",
        PushKind::T0Advice,
        "t0_dispatcher",
        "render_t0_forbid",
    ),
    descriptor(
        "T-07-candidate-triggered",
        PushKind::CandidateTriggered,
        "candidate_dispatcher",
        "render_candidate_triggered",
    ),
    descriptor(
        "T-08-candidate-invalidated",
        PushKind::CandidateInvalidated,
        "candidate_dispatcher",
        "render_candidate_invalidated",
    ),
    descriptor(
        "T-09-forbidden-ops",
        PushKind::ForbiddenOps,
        "holding_risk_dispatcher",
        "render_forbidden_ops",
    ),
    descriptor(
        "P-05-virtual-watch",
        PushKind::VirtualWatch,
        "virtual_watch_dispatcher",
        "render_virtual_watch",
    ),
    descriptor(
        "T-10-paper-trade",
        PushKind::PaperTrade,
        "paper_trade_dispatcher",
        "render_paper_trade",
    ),
    descriptor(
        "T-11-auction-volume",
        PushKind::AuctionVolume,
        "auction_volume_dispatcher",
        "render_auction_volume",
    ),
    descriptor(
        "T-12-close-call",
        PushKind::CloseCall,
        "close_call_dispatcher",
        "render_close_call",
    ),
    descriptor(
        "I-09-sector-top",
        PushKind::SectorTop,
        "sector_top_dispatcher",
        "render_sector_top",
    ),
    descriptor(
        "T-13-turnover-top",
        PushKind::TurnoverTop,
        "turnover_top_dispatcher",
        "render_turnover_top",
    ),
    descriptor(
        "R-01-daily-report",
        PushKind::DailyReport,
        "daily_report_dispatcher",
        "render_daily_report",
    ),
    descriptor(
        "R-02-review-market",
        PushKind::ReviewMarket,
        "review_market_dispatcher",
        "render_review_market",
    ),
    descriptor(
        "R-03-industry-chain",
        PushKind::IndustryChain,
        "industry_chain_review_dispatcher",
        "render_industry_chain",
    ),
    descriptor(
        "R-04-review-lhb-gateway",
        PushKind::ReviewLhb,
        "review_lhb_gateway_dispatcher",
        "render_review_lhb_gateway",
    ),
    descriptor(
        "R-05-review-signal",
        PushKind::ReviewSignal,
        "review_signal_dispatcher",
        "render_review_signal",
    ),
    descriptor(
        "R-06-review-failure",
        PushKind::ReviewFailure,
        "review_failure_dispatcher",
        "render_review_failure",
    ),
    descriptor(
        "R-07-tomorrow-watch",
        PushKind::TomorrowWatch,
        "tomorrow_watch_dispatcher",
        "render_tomorrow_watch",
    ),
    descriptor(
        "R-09-provider-top-n",
        PushKind::ReviewProviderTopN,
        "provider_top_n_dispatcher",
        "render_r09_provider_top_n",
    ),
    descriptor(
        "A-11-ipo-catalyst",
        PushKind::IpoCatalyst,
        "ipo_catalyst_dispatcher",
        "render_ipo_catalyst",
    ),
    descriptor(
        "P-05-candidate-board",
        PushKind::CandidateBoard,
        "candidate_board_dispatcher",
        "format_candidate_board",
    ),
    descriptor(
        "A-02-auction-repush",
        PushKind::AuctionRepush,
        "auction_repush_dispatcher",
        "render_auction_repush",
    ),
    descriptor(
        "R-11-position-review",
        PushKind::PositionReview,
        "position_review_dispatcher",
        "render_position_review",
    ),
    descriptor(
        "R-12-backtest-review",
        PushKind::ReviewBacktest,
        "review_backtest_dispatcher",
        "render_r12_backtest",
    ),
    descriptor(
        "T1-watch-tracking",
        PushKind::WatchlistTracking,
        "watchlist_tracking_dispatcher",
        "render_watchlist_tracking",
    ),
    descriptor(
        "P-01-preopen-news-hot",
        PushKind::PreopenNewsHot,
        "preopen_news_dispatcher",
        "render_preopen_news_hot",
    ),
    descriptor(
        "I-01-intraday-market",
        PushKind::IntradayMarket,
        "intraday_market_dispatcher",
        "render_intraday_market",
    ),
    descriptor(
        "I-02-news-catalyst",
        PushKind::NewsCatalyst,
        "news_catalyst_dispatcher",
        "render_news_catalyst",
    ),
    descriptor(
        "I-09-sector-anomaly",
        PushKind::SectorAnomaly,
        "sector_anomaly_dispatcher",
        "render_sector_anomaly",
    ),
    descriptor(
        "D-01-news-to-idea",
        PushKind::NewsToIdea,
        "news_to_idea_dispatcher",
        "render_news_to_idea",
    ),
    descriptor(
        "A-10-catalyst-review",
        PushKind::CatalystReview,
        "catalyst_review_dispatcher",
        "render_catalyst_review",
    ),
    descriptor(
        "I-03-industry-chain-intraday",
        PushKind::IndustryChainIntraday,
        "industry_chain_intraday_dispatcher",
        "render_industry_chain_intraday",
    ),
    descriptor(
        "T-14-post-fixed-price-order",
        PushKind::PostFixedPriceOrder,
        "post_fixed_price_dispatcher",
        "render_post_fixed_price_order",
    ),
    descriptor(
        "T-15-post-fixed-price-fill",
        PushKind::PostFixedPriceFill,
        "post_fixed_price_dispatcher",
        "render_post_fixed_price_fill",
    ),
    descriptor(
        "T-16-st-price-limit-changed",
        PushKind::StPriceLimitChanged,
        "st_price_limit_dispatcher",
        "render_st_price_limit_changed",
    ),
    descriptor(
        "T-17-etf-closing-call-auction",
        PushKind::EtfClosingCallAuction,
        "etf_closing_call_dispatcher",
        "render_etf_closing_call_auction",
    ),
    descriptor(
        "BR-033-block-trade-confirm",
        PushKind::BlockTradeIntradayConfirm,
        "block_trade_dispatcher",
        "render_block_trade_intraday_confirm",
    ),
    descriptor(
        "BR-034-block-trade-range",
        PushKind::BlockTradePriceRange,
        "block_trade_dispatcher",
        "render_block_trade_price_range",
    ),
    descriptor(
        "A-01-paper-review",
        PushKind::PaperReview,
        "paper_review_dispatcher",
        "render_paper_review",
    ),
    descriptor(
        "R-08-public-event-calendar",
        PushKind::EventCalendar,
        "dispatch_r08_event_calendar_outcome",
        "render_r08_public_calendar",
    ),
    descriptor(
        "L-01-limit-boards-first",
        PushKind::LimitBoards,
        "monitor_limit_board_producer",
        "assemble_limit_boards_first",
    ),
    descriptor(
        "L-02-limit-boards-second",
        PushKind::LimitBoards,
        "monitor_limit_board_producer",
        "assemble_limit_boards_second",
    ),
    descriptor(
        "L-03-limit-boards-third-plus",
        PushKind::LimitBoards,
        "monitor_limit_board_producer",
        "assemble_limit_boards_third_plus",
    ),
    descriptor(
        "S-01-announcement",
        PushKind::Announcement,
        "v17_source_dispatcher",
        "v17_sources_render_message_announcement",
    ),
    descriptor(
        "S-02-policy-hit",
        PushKind::PolicyHit,
        "v17_source_dispatcher",
        "v17_sources_render_message_policy_hit",
    ),
    descriptor(
        "S-03-earnings-beat",
        PushKind::EarningsBeat,
        "v17_source_dispatcher",
        "v17_sources_render_message_earnings_beat",
    ),
    descriptor(
        "S-04-earnings-miss",
        PushKind::EarningsMiss,
        "v17_source_dispatcher",
        "v17_sources_render_message_earnings_miss",
    ),
    descriptor(
        "S-05-analyst-upgrade",
        PushKind::AnalystUpgrade,
        "v17_source_dispatcher",
        "v17_sources_render_message_analyst_upgrade",
    ),
    descriptor(
        "S-06-market-action-alert",
        PushKind::MarketActionAlert,
        "v17_source_dispatcher",
        "v17_sources_render_message_market_action_alert",
    ),
    descriptor(
        "N-01-news-flash-critical",
        PushKind::NewsFlashCritical,
        "news_flash_critical_dispatcher",
        "assemble_news_flash_critical",
    ),
    descriptor(
        "N-02-news-flash-aggregated",
        PushKind::NewsFlashAggregated,
        "news_flash_aggregate_dispatcher",
        "assemble_news_flash_aggregated",
    ),
];

pub(super) fn descriptors() -> &'static [ProductionPresentationDescriptor; 58] {
    &PRODUCTION_PRESENTATION_DESCRIPTORS
}

pub(super) fn acquire_token(
    family_key: &str,
    push_kind: PushKind,
    producer_seam_id: &str,
    renderer_or_assembler_seam_id: &str,
) -> Result<ProductionPresentationToken, String> {
    let descriptor = PRODUCTION_PRESENTATION_DESCRIPTORS
        .iter()
        .find(|descriptor| {
            descriptor.family_key == family_key
                && descriptor.push_kind == push_kind
                && descriptor.producer_seam_id == producer_seam_id
                && descriptor.renderer_or_assembler_seam_id == renderer_or_assembler_seam_id
        })
        .ok_or_else(|| "BR-196 unknown production presentation tuple".to_string())?;
    Ok(ProductionPresentationToken { descriptor })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn br196_production_token_requires_exact_registered_tuple() {
        let token = acquire_token(
            "L-01-limit-boards-first",
            PushKind::LimitBoards,
            "monitor_limit_board_producer",
            "assemble_limit_boards_first",
        )
        .unwrap();
        assert_eq!(token.descriptor().family_key, "L-01-limit-boards-first");
        assert!(acquire_token(
            "L-01-limit-boards-first",
            PushKind::LimitBoards,
            "monitor_limit_board_producer",
            "assemble_limit_boards_second",
        )
        .is_err());
        assert!(acquire_token(
            "R-04-review-lhb-legacy",
            PushKind::ReviewLhb,
            "legacy",
            "render_review_lhb",
        )
        .is_err());
    }
}
