//! v15.3 Phase D1.2-D1.4: 12 个 NewsFeed 适配层
//!
//! 把现有 `search_service::providers::*` (8 个 flash) + 新建 4 个数据源
//! (GovCn / Miit / Earnings / Consensus / MarketAction / AnalystViews) 适配为 NewsFeed
//!
//! 设计: 每个 feed 只是薄壳, fetch 内部委托给现有数据源 provider, 然后 SearchResult → MarketEvent

use super::{NewsFeed, SourceKind};
use crate::signal::market_event::{Direction, EventType, MarketEvent, SourceRef};
use crate::util::recover_lock_or_warn;
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use chrono::{Local, Utc};
use std::sync::{Arc, Mutex};

/// 把任意 `SearchResult` (现有 search_service 类型) 转成 MarketEvent
fn search_result_to_event(
    r: &crate::search_service::SearchResult,
    source_kind: SourceKind,
    event_type: EventType,
) -> MarketEvent {
    let now = Utc::now();
    let simhash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        r.title.hash(&mut h);
        r.url.hash(&mut h);
        h.finish()
    };
    let dir = match r.sentiment {
        crate::search_service::Sentiment::Positive => Direction::Bull,
        crate::search_service::Sentiment::Negative => Direction::Bear,
        _ => Direction::Neutral,
    };
    let importance = (r.importance as u32).min(10) as u8;
    let simhash_str = format!("{:x}", simhash);
    let _ = simhash_str;
    MarketEvent {
        event_id: format!("{}-{:x}", source_kind.label(), simhash),
        simhash,
        full_title: r.title.clone(),
        event_type,
        subject: r.source.clone(),
        object: Some(r.title.clone()),
        direction: dir,
        strength: importance.saturating_mul(10),
        certainty: 60,
        chains: vec![],
        occurred_at: now.with_timezone(&Local),
        provenance: vec![SourceRef {
            provider: source_kind.label().to_string(),
            url: if r.url.is_empty() {
                None
            } else {
                Some(r.url.clone())
            },
            fetched_at: now.with_timezone(&Local),
        }],
        ai_degraded: false,
        stale: false,
    }
}

// ============================================================================
// 8 flash wraps — 全部用 SearchResult 接口
// ============================================================================

pub struct Jin10FlashFeed {
    pub inner: crate::search_service::providers::jin10::Jin10Provider,
}
#[async_trait]
impl NewsFeed for Jin10FlashFeed {
    fn name(&self) -> &str {
        "jin10_flash"
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::Flash
    }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self
            .inner
            .fetch_flash_news(limit, false)
            .await
            .context("jin10_flash fetch failed")?;
        Ok(v.iter()
            .map(|r| search_result_to_event(r, SourceKind::Flash, EventType::Other))
            .collect())
    }
}

pub struct WallStreetCnFeed {
    pub inner: crate::search_service::providers::wallstreetcn::WallStreetCnProvider,
}
#[async_trait]
impl NewsFeed for WallStreetCnFeed {
    fn name(&self) -> &str {
        "wallstreetcn_flash"
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::Flash
    }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self
            .inner
            .fetch_live_news(limit)
            .await
            .context("wallstreetcn_flash fetch failed")?;
        Ok(v.iter()
            .map(|r| search_result_to_event(r, SourceKind::Flash, EventType::Other))
            .collect())
    }
}

pub struct ClsFlashFeed {
    pub inner: crate::search_service::providers::cls::ClsProvider,
}
#[async_trait]
impl NewsFeed for ClsFlashFeed {
    fn name(&self) -> &str {
        "cls_flash"
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::Flash
    }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self
            .inner
            .fetch_live_news(limit)
            .await
            .context("cls_flash fetch failed")?;
        Ok(v.iter()
            .map(|r| search_result_to_event(r, SourceKind::Flash, EventType::Other))
            .collect())
    }
}

pub struct SinaFlashFeed {
    pub inner: crate::search_service::providers::sina_flash::SinaFlashProvider,
}
#[async_trait]
impl NewsFeed for SinaFlashFeed {
    fn name(&self) -> &str {
        "sina_flash"
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::Flash
    }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self.inner.fetch_flash_news(limit).await;
        Ok(v.iter()
            .map(|r| search_result_to_event(r, SourceKind::Flash, EventType::Other))
            .collect())
    }
}

pub struct WeiboHotFeed {
    pub inner: crate::search_service::providers::weibo_hot::WeiboHotProvider,
}
#[async_trait]
impl NewsFeed for WeiboHotFeed {
    fn name(&self) -> &str {
        "weibo_hot"
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::Flash
    }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self
            .inner
            .fetch_hot_search(limit)
            .await
            .context("weibo_hot fetch failed")?;
        Ok(v.iter()
            .map(|r| search_result_to_event(r, SourceKind::Flash, EventType::Other))
            .collect())
    }
}

pub struct GelonghuiFeed {
    pub inner: crate::search_service::providers::gelonghui::GelonghuiProvider,
}
#[async_trait]
impl NewsFeed for GelonghuiFeed {
    fn name(&self) -> &str {
        "gelonghui"
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::Flash
    }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self
            .inner
            .fetch_live(limit)
            .await
            .context("gelonghui fetch failed")?;
        Ok(v.iter()
            .map(|r| search_result_to_event(r, SourceKind::Flash, EventType::Other))
            .collect())
    }
}

pub struct KcbDailyFeed {
    pub inner: crate::search_service::providers::kcb_daily::KcbDailyProvider,
}
#[async_trait]
impl NewsFeed for KcbDailyFeed {
    fn name(&self) -> &str {
        "kcb_daily"
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::ActiveSearch
    }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self
            .inner
            .fetch_latest(limit)
            .await
            .context("kcb_daily fetch failed")?;
        Ok(v.iter()
            .map(|r| search_result_to_event(r, SourceKind::ActiveSearch, EventType::Other))
            .collect())
    }
}

pub struct GovPolicyFeed {
    pub inner: crate::search_service::providers::gov_policy::GovPolicyProvider,
}
#[async_trait]
impl NewsFeed for GovPolicyFeed {
    fn name(&self) -> &str {
        "gov_policy"
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::Policy
    }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self
            .inner
            .fetch_latest(limit)
            .await
            .context("gov_policy fetch failed")?;
        Ok(v.iter()
            .map(|r| search_result_to_event(r, SourceKind::Policy, EventType::Policy))
            .collect())
    }
}

// ============================================================================
// 未实现的政策 feed：不进入生产注册表，误调用显式 unavailable
// ============================================================================

pub struct GovCnFeed;
#[async_trait]
impl NewsFeed for GovCnFeed {
    fn name(&self) -> &str {
        "gov_cn_yaowen"
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::Policy
    }
    async fn fetch(&self, _limit: usize) -> Result<Vec<MarketEvent>> {
        bail!("gov_cn_yaowen unavailable: parser not implemented")
    }
}

pub struct MiitFeed;
#[async_trait]
impl NewsFeed for MiitFeed {
    fn name(&self) -> &str {
        "miit_policy"
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::Policy
    }
    async fn fetch(&self, _limit: usize) -> Result<Vec<MarketEvent>> {
        bail!("miit_policy unavailable: parser not implemented")
    }
}

// ============================================================================
// D1.3: 3 个财报 / 公告 / 共识 feed
// ============================================================================

pub struct EmAnnouncementFeed;
#[async_trait]
impl NewsFeed for EmAnnouncementFeed {
    fn name(&self) -> &str {
        "em_announcement"
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::Earnings
    }
    async fn fetch(&self, _limit: usize) -> Result<Vec<MarketEvent>> {
        let anns = crate::data_provider::announcement::fetch_announcements(None)
            .await
            .context("em_announcement fetch failed")?;
        if anns.is_empty() {
            log::info!("[EmAnnouncementFeed] no announcements this cycle");
            return Ok(vec![]);
        }
        let now = Utc::now().with_timezone(&Local);
        Ok(anns
            .iter()
            .map(|a| {
                let simhash = {
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut h = DefaultHasher::new();
                    a.code.hash(&mut h);
                    a.title.hash(&mut h);
                    h.finish()
                };
                MarketEvent {
                    event_id: format!("earnings-{:x}", simhash),
                    simhash,
                    full_title: a.title.clone(),
                    event_type: EventType::Announcement, // truthful category
                    subject: a.code.clone(),
                    object: Some(a.code.clone()),
                    direction: Direction::Neutral,
                    strength: 70,
                    certainty: 80,
                    chains: vec![],
                    occurred_at: now,
                    provenance: vec![SourceRef {
                        provider: "em_announcement".to_string(),
                        url: a.url.clone(),
                        fetched_at: now,
                    }],
                    ai_degraded: false,
                    stale: false,
                }
            })
            .collect())
    }
}

pub struct EarningsCalendarFeed;
#[async_trait]
impl NewsFeed for EarningsCalendarFeed {
    fn name(&self) -> &str {
        "earnings_calendar"
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::Earnings
    }
    async fn fetch(&self, _limit: usize) -> Result<Vec<MarketEvent>> {
        bail!("earnings_calendar unavailable: polling source not implemented")
    }
}

pub struct ConsensusFeed;
#[async_trait]
impl NewsFeed for ConsensusFeed {
    fn name(&self) -> &str {
        "consensus"
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::AnalystView
    }
    async fn fetch(&self, _limit: usize) -> Result<Vec<MarketEvent>> {
        bail!("consensus unavailable: polling source not implemented")
    }
}

// ============================================================================
// D1.4: MarketActionFeed + AnalystViewsFeed (主动触发，禁止按轮询源调用)
// ============================================================================

pub struct MarketActionFeed;
#[async_trait]
impl NewsFeed for MarketActionFeed {
    fn name(&self) -> &str {
        "market_action"
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::MarketAction
    }
    async fn fetch(&self, _limit: usize) -> Result<Vec<MarketEvent>> {
        bail!("market_action is push-driven and cannot be polled")
    }
}

pub struct AnalystViewsFeed;
#[async_trait]
impl NewsFeed for AnalystViewsFeed {
    fn name(&self) -> &str {
        "analyst_views"
    }
    fn source_kind(&self) -> SourceKind {
        SourceKind::AnalystView
    }
    async fn fetch(&self, _limit: usize) -> Result<Vec<MarketEvent>> {
        bail!("analyst_views is push-driven and cannot be polled")
    }
}

// ============================================================================
// 全局注册 (D1.5 wire)
// ============================================================================

pub type RegisteredFeeds = std::sync::Arc<Mutex<Vec<Arc<dyn NewsFeed>>>>;
static ALL_FEEDS: once_cell::sync::OnceCell<RegisteredFeeds> = once_cell::sync::OnceCell::new();

pub fn all_feeds() -> Option<RegisteredFeeds> {
    ALL_FEEDS.get().cloned()
}

pub fn register_feeds(feeds: Vec<Arc<dyn NewsFeed>>) {
    let g = ALL_FEEDS.get_or_init(|| std::sync::Arc::new(Mutex::new(Vec::new())));
    let mut g = recover_lock_or_warn("news::aggregator::register_feeds", g.lock());
    for f in feeds {
        g.push(f);
    }
}

/// 一次性取出已注册 feeds → 喂给 NewsAggregator
pub fn take_all_for_aggregator() -> Vec<Arc<dyn NewsFeed>> {
    match ALL_FEEDS.get() {
        Some(arc) => match arc.lock() {
            Ok(mut g) => std::mem::take(&mut *g),
            Err(p) => {
                log::warn!("[feed::take] lock poisoned, take inner");
                let mut inner = p.into_inner();
                std::mem::take(&mut *inner)
            }
        },
        None => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_result_adapter_preserves_direction_identity_url_and_strength_bounds() {
        let cases = [
            (
                crate::search_service::Sentiment::Positive,
                Direction::Bull,
                SourceKind::Policy,
                EventType::Policy,
            ),
            (
                crate::search_service::Sentiment::Negative,
                Direction::Bear,
                SourceKind::Earnings,
                EventType::Announcement,
            ),
            (
                crate::search_service::Sentiment::Neutral,
                Direction::Neutral,
                SourceKind::ActiveSearch,
                EventType::Other,
            ),
            (
                crate::search_service::Sentiment::Unknown,
                Direction::Neutral,
                SourceKind::Flash,
                EventType::Other,
            ),
        ];

        for (sentiment, direction, source_kind, event_type) in cases {
            let mut result = crate::search_service::SearchResult::new(
                "TEST_CODE 测试事件".to_string(),
                "测试来源证据".to_string(),
                "https://example.invalid/TEST_CODE".to_string(),
                "测试提供方".to_string(),
            );
            result.sentiment = sentiment;
            result.importance = 99;
            let event = search_result_to_event(&result, source_kind, event_type);
            assert_eq!(event.direction, direction);
            assert_eq!(event.event_type, event_type);
            assert_eq!(event.subject, "测试提供方");
            assert_eq!(event.object.as_deref(), Some("TEST_CODE 测试事件"));
            assert_eq!(event.strength, 100);
            assert_eq!(event.certainty, 60);
            assert!(event.event_id.starts_with(source_kind.label()));
            assert_eq!(event.provenance[0].provider, source_kind.label());
            assert_eq!(
                event.provenance[0].url.as_deref(),
                Some("https://example.invalid/TEST_CODE")
            );
            assert!(!event.ai_degraded);
            assert!(!event.stale);

            let repeat = search_result_to_event(&result, source_kind, event_type);
            assert_eq!(repeat.simhash, event.simhash);
            result.url.clear();
            let without_url = search_result_to_event(&result, source_kind, EventType::Other);
            assert_eq!(without_url.provenance[0].url, None);
        }
    }

    #[test]
    fn real_feed_wrappers_report_their_registered_identity_without_network_access() {
        let feeds: Vec<(Box<dyn NewsFeed>, &str, SourceKind)> = vec![
            (
                Box::new(Jin10FlashFeed {
                    inner: crate::search_service::providers::jin10::Jin10Provider::new(),
                }),
                "jin10_flash",
                SourceKind::Flash,
            ),
            (
                Box::new(WallStreetCnFeed {
                    inner:
                        crate::search_service::providers::wallstreetcn::WallStreetCnProvider::new(),
                }),
                "wallstreetcn_flash",
                SourceKind::Flash,
            ),
            (
                Box::new(ClsFlashFeed {
                    inner: crate::search_service::providers::cls::ClsProvider::new(),
                }),
                "cls_flash",
                SourceKind::Flash,
            ),
            (
                Box::new(SinaFlashFeed {
                    inner: crate::search_service::providers::sina_flash::SinaFlashProvider::new(),
                }),
                "sina_flash",
                SourceKind::Flash,
            ),
            (
                Box::new(WeiboHotFeed {
                    inner: crate::search_service::providers::weibo_hot::WeiboHotProvider::new(),
                }),
                "weibo_hot",
                SourceKind::Flash,
            ),
            (
                Box::new(GelonghuiFeed {
                    inner: crate::search_service::providers::gelonghui::GelonghuiProvider::new(),
                }),
                "gelonghui",
                SourceKind::Flash,
            ),
            (
                Box::new(KcbDailyFeed {
                    inner: crate::search_service::providers::kcb_daily::KcbDailyProvider::new(),
                }),
                "kcb_daily",
                SourceKind::ActiveSearch,
            ),
            (
                Box::new(GovPolicyFeed {
                    inner: crate::search_service::providers::gov_policy::GovPolicyProvider::new(),
                }),
                "gov_policy",
                SourceKind::Policy,
            ),
        ];
        for (feed, name, source_kind) in feeds {
            assert_eq!(feed.name(), name);
            assert_eq!(feed.source_kind(), source_kind);
        }
    }

    #[tokio::test]
    async fn unimplemented_and_push_driven_feeds_fail_explicitly() {
        let feeds: Vec<Box<dyn NewsFeed>> = vec![
            Box::new(GovCnFeed),
            Box::new(MiitFeed),
            Box::new(EarningsCalendarFeed),
            Box::new(ConsensusFeed),
            Box::new(MarketActionFeed),
            Box::new(AnalystViewsFeed),
        ];

        for feed in feeds {
            assert!(matches!(
                feed.source_kind(),
                SourceKind::Policy
                    | SourceKind::Earnings
                    | SourceKind::AnalystView
                    | SourceKind::MarketAction
            ));
            let result = feed.fetch(10).await;
            assert!(
                result.is_err(),
                "{} must not masquerade as an empty successful polling source",
                feed.name()
            );
        }
    }
}
