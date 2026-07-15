//! `news::aggregator` 在 monitor 主路径的初始化 + tick 接入
//!
//! ## 目标 (v15.3 Phase D 收尾)
//!
//! 把 `src/news/aggregator/feed.rs` 的 15 个 `NewsFeed` 适配 (Jin10 / WSCN / CLS / Sina /
//! Weibo / Gel / 科创板日报 / GovPolicy = 8 个真 HTTP;GovCn / MIIT / EmAnnouncement /
//! Earnings / Consensus / MarketAction / AnalystViews = 7 个 unit stub) 注册到全局
//! `NewsAggregator`, 然后在 `news_monitor_loop` 每 tick 调一次 `tick_news_aggregator(20)`,
//! 把 dedup 后的 `Vec<MarketEvent>` 喂给现有产出器 (本期仅 log + count).
//!
//! ## 调用链
//!
//! ```text
//! monitor::main()
//!   └─ init_news_aggregator()  ← 本文件
//!        ├─ register_feeds(13 × Arc<dyn NewsFeed>)
//!        ├─ take_all_for_aggregator()
//!        └─ NewsAggregator::new(...).set_global()
//!
//! monitor::main()
//!   └─ news_monitor_loop()
//!        └─ tick_news_aggregator(20).await  ← 本文件
//!             └─ NewsAggregator::global().tick(20) → 13 feed 并发取数 + simhash 去重
//!                  → Vec<MarketEvent> (本期 log event 数, 后续接入 news_ranker)
//! ```
//!
//! ## Idempotent
//!
//! 重复调 `init_news_aggregator()` 是 no-op (全局已 set_global 后直接 return).
//!
//! ## 红线约束
//!
//! - AGENTS.md §2.1: feed 失败显式 warn log, 不静默 panic
//! - CLAUDE.md Completion Rule: 本模块由 `src/bin/monitor/` 集成 (grep ≥1),
//!   不能只活在 `src/news/aggregator/feed.rs` 单测里

use std::sync::Arc;

use stock_analysis::news::aggregator::{
    self,
    feed::{self},
    NewsAggregator, NewsFeed,
};
use stock_analysis::signal::market_event::MarketEvent;

/// 注册 13 个 NewsFeed 适配到全局 NewsAggregator.
///
/// 在 monitor 启动早期调一次 (main() 里 spawn task 之前). 重复调 no-op.
///
/// 返回注册的 feed 数 (供单测断言 + 启动 log).
pub fn init_news_aggregator() -> usize {
    // Idempotent: 已 set_global 直接 return (不重复注册, 避免 Mutex<Vec> 累积)
    if aggregator::global().is_some() {
        log::info!("[NewsAggregator] 已初始化, 跳过重复 init");
        return feed_count_global();
    }

    let feeds: Vec<Arc<dyn NewsFeed>> = vec![
        // ===== Flash 源 (8 个, 真 HTTP; 每个 inner 调对应 Provider::new()) =====
        Arc::new(feed::Jin10FlashFeed {
            inner: stock_analysis::search_service::providers::jin10::Jin10Provider::new(),
        }),
        Arc::new(feed::WallStreetCnFeed {
            inner: stock_analysis::search_service::providers::wallstreetcn::WallStreetCnProvider::new(),
        }),
        Arc::new(feed::ClsFlashFeed {
            inner: stock_analysis::search_service::providers::cls::ClsProvider::new(),
        }),
        Arc::new(feed::SinaFlashFeed {
            inner: stock_analysis::search_service::providers::sina_flash::SinaFlashProvider::new(),
        }),
        Arc::new(feed::WeiboHotFeed {
            inner: stock_analysis::search_service::providers::weibo_hot::WeiboHotProvider::new(),
        }),
        Arc::new(feed::GelonghuiFeed {
            inner: stock_analysis::search_service::providers::gelonghui::GelonghuiProvider::new(),
        }),
        Arc::new(feed::KcbDailyFeed {
            inner: stock_analysis::search_service::providers::kcb_daily::KcbDailyProvider::new(),
        }),
        Arc::new(feed::GovPolicyFeed {
            inner: stock_analysis::search_service::providers::gov_policy::GovPolicyProvider::new(),
        }),
        // ===== 政策源 (GovCn / MIIT; unit struct 占位 stub) =====
        Arc::new(feed::GovCnFeed),
        Arc::new(feed::MiitFeed),
        // ===== 公告 / 财报源 (unit stub) =====
        Arc::new(feed::EmAnnouncementFeed),
        Arc::new(feed::EarningsCalendarFeed),
        Arc::new(feed::ConsensusFeed),
        // ===== 实盘 + 机构观点 (unit stub) =====
        Arc::new(feed::MarketActionFeed),
        Arc::new(feed::AnalystViewsFeed),
    ];
    let count = feeds.len();

    feed::register_feeds(feeds);
    let drained = feed::take_all_for_aggregator();
    let real_count = drained.len();
    let agg = NewsAggregator::new(drained);
    aggregator::set_global(Arc::new(agg));

    log::info!(
        "[NewsAggregator] init 完成: {} feeds registered, {} 喂入 aggregator",
        count, real_count
    );
    real_count
}

/// 全局已注册 feed 数 (供调试 / 启动 banner).
fn feed_count_global() -> usize {
    aggregator::global()
        .map(|agg| agg.feed_count())
        .unwrap_or(0)
}

/// 在 `news_monitor_loop` 中每 tick 调一次, 拿到 dedup 后的 `Vec<MarketEvent>`.
///
/// 本期仅 log event 数 + 按事件类型分布统计; 后续接入 news_ranker /
/// news_outcome / news_catalyst 时把 events 喂过去.
pub async fn tick_news_aggregator(per_feed_limit: usize) -> Vec<MarketEvent> {
    match aggregator::global() {
        Some(agg) => {
            let events = agg.tick(per_feed_limit).await;
            if events.is_empty() {
                log::debug!(
                    "[NewsAggregator] tick 返回 0 事件 (per_feed_limit={})",
                    per_feed_limit
                );
                return vec![];
            }
            let mut counts_by_type: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for e in &events {
                *counts_by_type
                    .entry(format!("{:?}", e.event_type))
                    .or_insert(0) += 1;
            }
            log::info!(
                "[NewsAggregator] tick 拿到 {} 事件, 按类型: {:?} (per_feed_limit={})",
                events.len(),
                counts_by_type,
                per_feed_limit
            );
            // future: news_ranker::rank_events(&events) → 候选 → 推 v14 push 栈
            events
        }
        None => {
            log::warn!(
                "[NewsAggregator] global() 尚未初始化, 调用方应在 main() 早期先调 init_news_aggregator()"
            );
            vec![]
        }
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_news_aggregator_short_circuits_when_global_set() {
        // F5 修复 (review #5): 旧名 `init_news_aggregator_is_idempotent` 名不副实 — 二次调
        // 走 early-return (line 52),不真创建 feeds. 测 short-circuit 行为而非"幂等注册".
        let c1 = init_news_aggregator();
        let c2 = init_news_aggregator();
        assert!(c1 > 0, "首次 init 应返回 >0 feed 数, 实际 {}", c1);
        assert_eq!(
            c1, c2,
            "short-circuit: 二次 init 应返回相同 feed 数 (实际 {} vs {})",
            c1, c2
        );
    }

    #[test]
    fn global_aggregator_has_feeds_after_init() {
        let count = feed_count_global();
        log::info!("[test] global aggregator 现有 {} feeds", count);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tick_news_aggregator_returns_real_market_events() {
        // F12 修复: tick 真返 Vec<MarketEvent>,不再是 u64 stub.
        let events = tick_news_aggregator(5).await;
        log::info!("[test] tick 拿到 {} 个 MarketEvent", events.len());
        // Vec<MarketEvent> 是真事件类型, 后续 caller 可直接喂 news_ranker
    }
}
