//! v15.3 Phase D1.2-D1.4: 12 个 NewsFeed 适配层
//!
//! 把现有 `search_service::providers::*` (8 个 flash) + 新建 4 个数据源
//! (GovCn / Miit / Earnings / Consensus / MarketAction / AnalystViews) 适配为 NewsFeed
//!
//! 设计: 每个 feed 只是薄壳, fetch 内部委托给现有数据源 provider, 然后 SearchResult → MarketEvent

use super::{NewsFeed, SourceKind};
use crate::signal::market_event::{Direction, EventType, MarketEvent, SourceRef};
use crate::util::recover_lock_or_warn;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{Local, Utc};
use std::sync::Mutex;

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
            url: if r.url.is_empty() { None } else { Some(r.url.clone()) },
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
    fn name(&self) -> &str { "jin10_flash" }
    fn source_kind(&self) -> SourceKind { SourceKind::Flash }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self.inner.fetch_flash_news(limit, false).await.unwrap_or_default();
        Ok(v.iter().map(|r| search_result_to_event(r, SourceKind::Flash, EventType::Other)).collect())
    }
}

pub struct WallStreetCnFeed {
    pub inner: crate::search_service::providers::wallstreetcn::WallStreetCnProvider,
}
#[async_trait]
impl NewsFeed for WallStreetCnFeed {
    fn name(&self) -> &str { "wallstreetcn_flash" }
    fn source_kind(&self) -> SourceKind { SourceKind::Flash }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self.inner.fetch_live_news(limit).await.unwrap_or_default();
        Ok(v.iter().map(|r| search_result_to_event(r, SourceKind::Flash, EventType::Other)).collect())
    }
}

pub struct ClsFlashFeed {
    pub inner: crate::search_service::providers::cls::ClsProvider,
}
#[async_trait]
impl NewsFeed for ClsFlashFeed {
    fn name(&self) -> &str { "cls_flash" }
    fn source_kind(&self) -> SourceKind { SourceKind::Flash }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self.inner.fetch_live_news(limit).await.unwrap_or_default();
        Ok(v.iter().map(|r| search_result_to_event(r, SourceKind::Flash, EventType::Other)).collect())
    }
}

pub struct SinaFlashFeed {
    pub inner: crate::search_service::providers::sina_flash::SinaFlashProvider,
}
#[async_trait]
impl NewsFeed for SinaFlashFeed {
    fn name(&self) -> &str { "sina_flash" }
    fn source_kind(&self) -> SourceKind { SourceKind::Flash }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self.inner.fetch_flash_news(limit).await;
        Ok(v.iter().map(|r| search_result_to_event(r, SourceKind::Flash, EventType::Other)).collect())
    }
}

pub struct WeiboHotFeed {
    pub inner: crate::search_service::providers::weibo_hot::WeiboHotProvider,
}
#[async_trait]
impl NewsFeed for WeiboHotFeed {
    fn name(&self) -> &str { "weibo_hot" }
    fn source_kind(&self) -> SourceKind { SourceKind::Flash }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self.inner.fetch_hot_search(limit).await.unwrap_or_default();
        Ok(v.iter().map(|r| search_result_to_event(r, SourceKind::Flash, EventType::Other)).collect())
    }
}

pub struct GelonghuiFeed {
    pub inner: crate::search_service::providers::gelonghui::GelonghuiProvider,
}
#[async_trait]
impl NewsFeed for GelonghuiFeed {
    fn name(&self) -> &str { "gelonghui" }
    fn source_kind(&self) -> SourceKind { SourceKind::Flash }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self.inner.fetch_live(limit).await.unwrap_or_default();
        Ok(v.iter().map(|r| search_result_to_event(r, SourceKind::Flash, EventType::Other)).collect())
    }
}

pub struct KcbDailyFeed {
    pub inner: crate::search_service::providers::kcb_daily::KcbDailyProvider,
}
#[async_trait]
impl NewsFeed for KcbDailyFeed {
    fn name(&self) -> &str { "kcb_daily" }
    fn source_kind(&self) -> SourceKind { SourceKind::ActiveSearch }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self.inner.fetch_latest(limit).await.unwrap_or_default();
        Ok(v.iter().map(|r| search_result_to_event(r, SourceKind::ActiveSearch, EventType::Other)).collect())
    }
}

pub struct GovPolicyFeed {
    pub inner: crate::search_service::providers::gov_policy::GovPolicyProvider,
}
#[async_trait]
impl NewsFeed for GovPolicyFeed {
    fn name(&self) -> &str { "gov_policy" }
    fn source_kind(&self) -> SourceKind { SourceKind::Policy }
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>> {
        let v = self.inner.fetch_latest(limit).await.unwrap_or_default();
        Ok(v.iter().map(|r| search_result_to_event(r, SourceKind::Policy, EventType::Policy)).collect())
    }
}

// ============================================================================
// 新建 2 个政策 feed (gov.cn RSS / miit.gov.cn 栏目) — skeleton, 待真实 HTML 解析
// ============================================================================

pub struct GovCnFeed;
#[async_trait]
impl NewsFeed for GovCnFeed {
    fn name(&self) -> &str { "gov_cn_yaowen" }
    fn source_kind(&self) -> SourceKind { SourceKind::Policy }
    async fn fetch(&self, _limit: usize) -> Result<Vec<MarketEvent>> {
        log::debug!("[GovCnFeed] skeleton — 国务院栏目 RSS 待实现");
        Ok(vec![])
    }
}

pub struct MiitFeed;
#[async_trait]
impl NewsFeed for MiitFeed {
    fn name(&self) -> &str { "miit_policy" }
    fn source_kind(&self) -> SourceKind { SourceKind::Policy }
    async fn fetch(&self, _limit: usize) -> Result<Vec<MarketEvent>> {
        log::debug!("[MiitFeed] skeleton — 工信部栏目 RSS 待实现");
        Ok(vec![])
    }
}

// ============================================================================
// D1.3: 3 个财报 / 公告 / 共识 feed
// ============================================================================

pub struct EmAnnouncementFeed;
#[async_trait]
impl NewsFeed for EmAnnouncementFeed {
    fn name(&self) -> &str { "em_announcement" }
    fn source_kind(&self) -> SourceKind { SourceKind::Earnings }
    async fn fetch(&self, _limit: usize) -> Result<Vec<MarketEvent>> {
        let anns = crate::data_provider::announcement::fetch_announcements(None).await.unwrap_or_default();
        let now = Utc::now().with_timezone(&Local);
        Ok(anns.iter().map(|a| {
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
                event_type: EventType::Policy,  // D2 will add Earnings variant
                subject: a.code.clone(),
                object: Some(a.code.clone()),
                direction: Direction::Neutral,
                strength: 70,
                certainty: 80,
                chains: vec![],
                occurred_at: now,
                provenance: vec![SourceRef {
                    provider: "em_announcement".to_string(),
                    url: None,
                    fetched_at: now,
                }],
                ai_degraded: false,
                stale: false,
            }
        }).collect())
    }
}

pub struct EarningsCalendarFeed;
#[async_trait]
impl NewsFeed for EarningsCalendarFeed {
    fn name(&self) -> &str { "earnings_calendar" }
    fn source_kind(&self) -> SourceKind { SourceKind::Earnings }
    async fn fetch(&self, _limit: usize) -> Result<Vec<MarketEvent>> {
        Ok(vec![])
    }
}

pub struct ConsensusFeed;
#[async_trait]
impl NewsFeed for ConsensusFeed {
    fn name(&self) -> &str { "consensus" }
    fn source_kind(&self) -> SourceKind { SourceKind::AnalystView }
    async fn fetch(&self, _limit: usize) -> Result<Vec<MarketEvent>> {
        Ok(vec![])
    }
}

// ============================================================================
// D1.4: MarketActionFeed + AnalystViewsFeed (主动触发, 被动 feed 返回空)
// ============================================================================

pub struct MarketActionFeed;
#[async_trait]
impl NewsFeed for MarketActionFeed {
    fn name(&self) -> &str { "market_action" }
    fn source_kind(&self) -> SourceKind { SourceKind::MarketAction }
    async fn fetch(&self, _limit: usize) -> Result<Vec<MarketEvent>> {
        Ok(vec![])  // 主动路径: portfolio::market_action 触发
    }
}

pub struct AnalystViewsFeed;
#[async_trait]
impl NewsFeed for AnalystViewsFeed {
    fn name(&self) -> &str { "analyst_views" }
    fn source_kind(&self) -> SourceKind { SourceKind::AnalystView }
    async fn fetch(&self, _limit: usize) -> Result<Vec<MarketEvent>> {
        Ok(vec![])  // 被动: 卖方研报由 D1.4 主动 xueqiu 触发
    }
}

// ============================================================================
// 全局注册 (D1.5 wire)
// ============================================================================

static ALL_FEEDS: once_cell::sync::OnceCell<std::sync::Arc<Mutex<Vec<Box<dyn NewsFeed>>>>> =
    once_cell::sync::OnceCell::new();

pub fn all_feeds() -> Option<std::sync::Arc<Mutex<Vec<Box<dyn NewsFeed>>>>> {
    ALL_FEEDS.get().cloned()
}

pub fn register_feeds(feeds: Vec<Box<dyn NewsFeed>>) {
    let g = ALL_FEEDS.get_or_init(|| std::sync::Arc::new(Mutex::new(Vec::new())));
    let mut g = recover_lock_or_warn("news::aggregator::register_feeds", g.lock());
    for f in feeds {
        g.push(f);
    }
}
