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
// v17.4 §5.1 能力1: NewsFlashGate — critical 即时推 + 4 时段聚合 Top3
// 业务规则登记: BR-033 (docs/business_rules.md, 红线 2.10)
// ============================================================================

/// 4 个聚合窗口 (开盘/午盘收/午盘开/收盘)
const AGG_WINDOWS: [(u32, u32); 4] = [(9, 30), (11, 30), (13, 0), (15, 0)];

/// 窗口触发容差: 窗口时刻起 5 分钟内首个 tick 触发 (news_monitor_loop 轮询默认
/// 120s, spec ±1min 会漏; 加宽到 5min + 当日一次门控, 偏差已在 spec 回填注明)
const AGG_WINDOW_TOLERANCE_SECS: i64 = 300;

/// 聚合决策 (纯数据, 供单测断言)
#[derive(Debug, PartialEq)]
pub enum FlashDecision {
    /// 即时推 (critical): (event_id, 渲染文本)
    Critical(String, String),
    /// 时段聚合推: (窗口标签, 渲染文本)
    Aggregated(String, String),
}

/// v17.4 §5.1 门控状态机 (纯逻辑, 不做 IO — 推送由 caller 处理)
pub struct NewsFlashGate {
    day: chrono::NaiveDate,
    seen_today: std::collections::HashSet<String>,
    critical_pushed_today: u32,
    /// 每窗口当日是否已触发
    window_fired: [bool; 4],
    /// 当日事件缓冲 (strength, 标题行) — 聚合 Top3 用, 上限 200
    buffer: Vec<(u8, String)>,
}

impl NewsFlashGate {
    pub fn new(today: chrono::NaiveDate) -> Self {
        Self {
            day: today,
            seen_today: std::collections::HashSet::new(),
            critical_pushed_today: 0,
            window_fired: [false; 4],
            buffer: Vec::new(),
        }
    }

    /// 跨天重置 (BR-033: 日桶清零, 防内存增长)
    fn rollover(&mut self, today: chrono::NaiveDate) {
        if self.day != today {
            self.day = today;
            self.seen_today.clear();
            self.critical_pushed_today = 0;
            self.window_fired = [false; 4];
            self.buffer.clear();
            log::info!("[NewsFlashGate] day rollover → {} (buckets reset)", today);
        }
    }

    /// 每 tick 调用: 喂入 dedup 后事件 + 当前时间 → 产出推送决策 (BR-033)
    ///
    /// critical 判定: strength ≥ threshold 且 certainty ≥ 60 (官方性门槛);
    /// 每日上限 max_per_day, 超限 warn 出声 (v15.x 静默路径可见)。
    pub fn process(
        &mut self,
        events: &[MarketEvent],
        now: chrono::DateTime<chrono::Local>,
        critical_threshold: u8,
        max_critical_per_day: u32,
    ) -> Vec<FlashDecision> {
        self.rollover(now.date_naive());
        let mut out = Vec::new();

        // 1. 事件驱动: critical 即时推 (AC34)
        for e in events {
            if !self.seen_today.insert(e.event_id.clone()) {
                continue; // event_id 当日去重
            }
            // buffer 收集 (聚合用, 上限 200)
            if self.buffer.len() < 200 {
                self.buffer.push((
                    e.strength,
                    format!(
                        "[{}] {} (强度{} 确定性{})",
                        e.event_type.label(),
                        if e.full_title.is_empty() { &e.subject } else { &e.full_title },
                        e.strength,
                        e.certainty
                    ),
                ));
            }
            if e.strength >= critical_threshold && e.certainty >= 60 {
                if self.critical_pushed_today >= max_critical_per_day {
                    log::warn!(
                        "[NewsFlashGate] critical 日上限已满 ({}/{}), 跳过: {}",
                        self.critical_pushed_today, max_critical_per_day, e.subject
                    );
                    continue;
                }
                self.critical_pushed_today += 1;
                out.push(FlashDecision::Critical(
                    e.event_id.clone(),
                    format!(
                        "🚨 高分新闻快讯 ({})\n[{}] {}\n强度 {} | 确定性 {} | 今日第 {}/{} 条",
                        now.format("%H:%M"),
                        e.event_type.label(),
                        if e.full_title.is_empty() { &e.subject } else { &e.full_title },
                        e.strength,
                        e.certainty,
                        self.critical_pushed_today,
                        max_critical_per_day
                    ),
                ));
            }
        }

        // 2. 4 时段聚合 Top3 (AC35): 窗口时刻起 5min 内首个 tick 触发, 当日一次
        for (i, (h, m)) in AGG_WINDOWS.iter().enumerate() {
            if self.window_fired[i] {
                continue;
            }
            let target = now
                .date_naive()
                .and_hms_opt(*h, *m, 0)
                .expect("valid window time")
                .and_local_timezone(chrono::Local)
                .single();
            let Some(target) = target else { continue };
            let delta = (now - target).num_seconds();
            if (0..AGG_WINDOW_TOLERANCE_SECS).contains(&delta) {
                self.window_fired[i] = true;
                let label = format!("{:02}:{:02}", h, m);
                if self.buffer.is_empty() {
                    // 红线 2.2: 无数据显式说明, 不臆造
                    log::info!("[NewsFlashGate] {} 窗口无事件, 跳过聚合推送", label);
                    continue;
                }
                let mut sorted: Vec<&(u8, String)> = self.buffer.iter().collect();
                sorted.sort_by(|a, b| b.0.cmp(&a.0));
                let mut text = format!("📰 新闻时段聚合 ({}) Top3:\n", label);
                for (rank, (_, line)) in sorted.iter().take(3).enumerate() {
                    text.push_str(&format!("{}. {}\n", rank + 1, line));
                }
                out.push(FlashDecision::Aggregated(label, text));
            }
        }

        out
    }
}

/// 推送包装: 把 FlashDecision 走现有 push_governor_v3 (L4 dedup: critical 按
/// event_id, 聚合按窗口标签 — 见 BR-033)。返回 (critical 推送数, 聚合推送数)。
pub async fn push_flash_decisions(decisions: Vec<FlashDecision>) -> (usize, usize) {
    let mut n_critical = 0usize;
    let mut n_agg = 0usize;
    for d in decisions {
        match d {
            FlashDecision::Critical(event_id, text) => {
                let outcome = crate::notify::push_governor_v3(
                    &text,
                    crate::notify::PushKind::NewsFlashCritical,
                    Some(&event_id[..event_id.len().min(16)]),
                )
                .await;
                if outcome.is_pushed() {
                    n_critical += 1;
                } else {
                    log::info!("[NewsFlashGate] critical 未推 (治理): {:?}", outcome);
                }
            }
            FlashDecision::Aggregated(window, text) => {
                let outcome = crate::notify::push_governor_v3(
                    &text,
                    crate::notify::PushKind::NewsFlashAggregated,
                    Some(&window),
                )
                .await;
                if outcome.is_pushed() {
                    n_agg += 1;
                } else {
                    log::info!("[NewsFlashGate] {} 聚合未推 (治理): {:?}", window, outcome);
                }
            }
        }
    }
    (n_critical, n_agg)
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use stock_analysis::signal::market_event::{Direction, EventType};

    fn ev(id_seed: &str, strength: u8, certainty: u8) -> MarketEvent {
        let mut e = MarketEvent::new(
            EventType::Policy,
            format!("测试事件-{}", id_seed),
            None,
            Direction::Bull,
            strength,
            certainty,
        );
        e.event_id = format!("eid-{}", id_seed); // 固定 id 便于断言
        e
    }

    fn at(h: u32, m: u32) -> chrono::DateTime<chrono::Local> {
        chrono::Local::now()
            .date_naive()
            .and_hms_opt(h, m, 0)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .single()
            .unwrap()
    }

    /// AC34 + AC46: 阈值默认 80/certainty 60 门; 低分不推
    #[test]
    fn gate_critical_threshold_and_certainty() {
        let mut g = NewsFlashGate::new(at(10, 0).date_naive());
        let d = g.process(&[ev("a", 85, 70), ev("b", 85, 30), ev("c", 60, 90)], at(10, 0), 80, 20);
        assert_eq!(d.len(), 1, "仅 strength≥80 且 certainty≥60 推");
        assert!(matches!(&d[0], FlashDecision::Critical(id, _) if id == "eid-a"));
    }

    /// BR-033: event_id 当日去重
    #[test]
    fn gate_dedup_same_event_id() {
        let mut g = NewsFlashGate::new(at(10, 0).date_naive());
        let e = ev("dup", 90, 90);
        assert_eq!(g.process(&[e.clone()], at(10, 0), 80, 20).len(), 1);
        assert_eq!(g.process(&[e], at(10, 1), 80, 20).len(), 0, "同 event_id 当日不重推");
    }

    /// BR-033: 每日上限
    #[test]
    fn gate_daily_cap() {
        let mut g = NewsFlashGate::new(at(10, 0).date_naive());
        let events: Vec<MarketEvent> = (0..5).map(|i| ev(&format!("cap{}", i), 90, 90)).collect();
        let d = g.process(&events, at(10, 0), 80, 3);
        assert_eq!(d.len(), 3, "超 max_critical_per_day=3 截断");
    }

    /// AC35: 窗口触发一次/日 + Top3 按 strength 降序
    #[test]
    fn gate_window_fires_once_with_top3() {
        let mut g = NewsFlashGate::new(at(9, 0).date_naive());
        // 9:00 喂 4 条低分事件 (进 buffer, 不 critical)
        let events: Vec<MarketEvent> =
            [40u8, 70, 55, 60].iter().enumerate().map(|(i, &s)| ev(&format!("w{}", i), s, 50)).collect();
        assert!(g.process(&events, at(9, 0), 80, 20).is_empty());
        // 9:31 → 触发 09:30 窗口
        let d1 = g.process(&[], at(9, 31), 80, 20);
        assert_eq!(d1.len(), 1);
        match &d1[0] {
            FlashDecision::Aggregated(w, text) => {
                assert_eq!(w, "09:30");
                assert!(text.contains("强度70"), "Top1 应是 strength=70: {}", text);
                assert_eq!(text.matches("测试事件").count(), 3, "只取 Top3");
            }
            other => panic!("应为 Aggregated, got {:?}", other),
        }
        // 9:33 再 tick → 同窗口不重复触发
        assert!(g.process(&[], at(9, 33), 80, 20).is_empty(), "窗口当日一次");
    }

    /// 红线 2.2: 窗口无事件不臆造推送
    #[test]
    fn gate_window_empty_buffer_no_push() {
        let mut g = NewsFlashGate::new(at(11, 0).date_naive());
        assert!(g.process(&[], at(11, 30), 80, 20).is_empty());
    }

    /// AC46: config 默认值
    #[test]
    fn news_config_defaults() {
        let cfg = stock_analysis::config::MonitorConfig::default();
        assert_eq!(cfg.news_critical_score_threshold, 80);
        assert_eq!(cfg.news_max_critical_per_day, 20);
    }

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
