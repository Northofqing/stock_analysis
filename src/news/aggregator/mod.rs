//! v15.3 Phase D1: NewsAggregator — 统一 12 个 NewsFeed 的抓取 + 去重 + 调度
//!
//! 设计要点:
//! - NewsFeed trait 是所有数据源 (flash/active/policy/earnings/market_action/analyst) 的统一接口
//! - 复用 `MarketEvent` (signal/market_event.rs) 作为内部事件结构 (已含 simhash 去重字段)
//! - 单实例全局 `OnceCell<NewsAggregator>`, 12 个 feed 注册在启动时
//!
//! 复用现有: 不重建 NewsEvent, 不重建 dedup, 全部依赖 MarketEvent::simhash

pub mod feed;

use crate::signal::market_event::{EventType, MarketEvent};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::sync::Arc;

/// 新闻源类型 (用于 dispatcher 多源共振 + 影响打分加权)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Flash,
    ActiveSearch,
    Policy,
    Earnings,
    MarketAction,
    AnalystView,
}

impl SourceKind {
    pub fn label(&self) -> &'static str {
        match self {
            SourceKind::Flash => "flash",
            SourceKind::ActiveSearch => "active_search",
            SourceKind::Policy => "policy",
            SourceKind::Earnings => "earnings",
            SourceKind::MarketAction => "market_action",
            SourceKind::AnalystView => "analyst_view",
        }
    }
}

/// NewsFeed trait — 所有数据源统一接口
#[async_trait]
pub trait NewsFeed: Send + Sync {
    fn name(&self) -> &str;
    fn source_kind(&self) -> SourceKind;
    async fn fetch(&self, limit: usize) -> Result<Vec<MarketEvent>>;
}

/// NewsAggregator — 多源收敛 + simhash 去重
pub struct NewsAggregator {
    feeds: Vec<Arc<dyn NewsFeed>>,
    seen_simhash: std::sync::Mutex<HashSet<u64>>,
}

impl NewsAggregator {
    pub fn new(feeds: Vec<Arc<dyn NewsFeed>>) -> Self {
        Self {
            feeds,
            seen_simhash: std::sync::Mutex::new(HashSet::new()),
        }
    }

    /// 拉所有 feed + 按 simhash 去重 (返回新增的)
    pub async fn tick(&self, per_feed_limit: usize) -> Vec<MarketEvent> {
        let mut all_events: Vec<MarketEvent> = Vec::new();
        for feed in &self.feeds {
            match feed.fetch(per_feed_limit).await {
                Ok(events) => all_events.extend(events),
                Err(e) => log::warn!("[NewsAggregator] feed {} 失败: {}", feed.name(), e),
            }
        }
        // simhash 去重 (std::sync::Mutex 返回 Result, poison 时继续)
        let mut seen = match self.seen_simhash.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        all_events.retain(|e| {
            let h = e.simhash;
            if seen.contains(&h) {
                false
            } else {
                seen.insert(h);
                true
            }
        });
        // 按时间倒序
        all_events.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at));
        all_events
    }

    pub fn feed_count(&self) -> usize {
        self.feeds.len()
    }
}

/// 全局单例 (V14Stack::global() 风格)
static GLOBAL_AGGREGATOR: once_cell::sync::OnceCell<Arc<NewsAggregator>> =
    once_cell::sync::OnceCell::new();

pub fn set_global(agg: Arc<NewsAggregator>) {
    if GLOBAL_AGGREGATOR.set(agg).is_err() {
        log::warn!("[NewsAggregator] set_global 重复, 忽略");
    }
}

pub fn global() -> Option<Arc<NewsAggregator>> {
    GLOBAL_AGGREGATOR.get().cloned()
}

/// 辅助: MarketEvent 构造
pub fn build_market_event(
    event_type: EventType,
    source_kind: SourceKind,
    title: String,
    subject: String,
    object: String,
    direction: crate::signal::market_event::Direction,
) -> MarketEvent {
    let now: DateTime<Utc> = Utc::now();
    let simhash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        title.hash(&mut h);
        h.finish()
    };
    MarketEvent {
        event_id: format!("{}-{:x}", source_kind.label(), simhash),
        simhash,
        full_title: title,
        event_type,
        subject,
        object: Some(object),
        direction,
        strength: 50,
        certainty: 50,
        chains: vec![],
        occurred_at: now.with_timezone(&chrono::Local),
        provenance: vec![crate::signal::market_event::SourceRef {
            provider: source_kind.label().to_string(),
            url: None,
            fetched_at: now.with_timezone(&chrono::Local),
        }],
        ai_degraded: false,
        stale: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::market_event::{Direction, EventType};

    struct MockFeed {
        name: String,
        events: Vec<MarketEvent>,
    }

    #[async_trait]
    impl NewsFeed for MockFeed {
        fn name(&self) -> &str {
            &self.name
        }
        fn source_kind(&self) -> SourceKind {
            SourceKind::Flash
        }
        async fn fetch(&self, _limit: usize) -> Result<Vec<MarketEvent>> {
            Ok(self.events.clone())
        }
    }

    #[tokio::test]
    async fn test_aggregator_dedups_by_simhash() {
        let evt = build_market_event(
            EventType::Policy,
            SourceKind::Policy,
            "长鑫存储递交招股说明书".into(),
            "长鑫存储".into(),
            "兆易创新".into(),
            Direction::Bull,
        );
        let feed1: Arc<dyn NewsFeed> = Arc::new(MockFeed {
            name: "feed1".into(),
            events: vec![evt.clone()],
        });
        let feed2: Arc<dyn NewsFeed> = Arc::new(MockFeed {
            name: "feed2".into(),
            events: vec![evt.clone()],
        });
        let agg = NewsAggregator::new(vec![feed1, feed2]);
        let first = agg.tick(10).await;
        assert_eq!(first.len(), 1, "2 feeds same event → 1 dedup");
        let second = agg.tick(10).await;
        assert_eq!(second.len(), 0, "second tick 不返回已见");
    }

    #[test]
    fn test_build_market_event_basic() {
        let e = build_market_event(
            EventType::Policy,
            SourceKind::Policy,
            "测试".into(),
            "X".into(),
            "Y".into(),
            Direction::Bull,
        );
        assert_eq!(e.event_type, EventType::Policy);
        assert_eq!(e.provenance[0].provider, "policy");
    }
}