//! v16.4 Commit 2 — Strategy trait 抽象 (替代 v16.3 8 enum 硬编码).
//!
//! 设计 (v16.3 doc §3.3): 替代 `opportunity::virtual_reason::VirtualReason` 8 enum,
//!                          用 trait + 8 impl struct, 让 v16.3 8 个 strategy 各自独立,
//!                          支持 v16.4 第 4 步过滤 (FeatureBuilder + ScoreCalculator +
//!                          DecisionPolicy) 拆分层.
//!
//! v16.4 Commit 2 注: 8 impl 文件拆独立, 不动 v16.3 evaluate_candidate 评分表.
//!
//! BR-098: 8 个实现只消费结构化真实指标；生产 `intraday_monitor` 直接调用
//! `Strategy::score`，缺失/坏指标返回 `None`，不再按 push_kind 使用固定分数。

use crate::bus::StrategyId;

#[derive(Debug, Clone)]
pub struct StrategyInput {
    pub code: String,
    pub push_price: f64,
    pub metric_json: String,
    pub push_kind: String,
    pub now: chrono::DateTime<chrono::Local>,
}

#[derive(Debug, Clone)]
pub struct StrategyOutput {
    pub score: f64,
    pub reason: String,
    pub virtual_reason: String,
}

pub trait Strategy: Send + Sync {
    fn id(&self) -> StrategyId;
    fn virtual_reason(&self) -> &'static str;
    fn score(&self, input: &StrategyInput) -> Option<StrategyOutput>;
    fn description(&self) -> &'static str;
    fn is_active(&self) -> bool {
        true
    }
}

pub fn register_all() {
    use crate::registry::StrategyRegistry;
    let r = StrategyRegistry::global();
    let all: Vec<Box<dyn Strategy>> = vec![
        Box::new(NewsCatalystStrategy),
        Box::new(AuctionAnomalyStrategy),
        Box::new(MainNetInflowStrategy),
        Box::new(SectorLeaderStrategy),
        Box::new(BreakoutStrategy),
        Box::new(VolumeSurgeStrategy),
        Box::new(LLMSelectStrategy),
        Box::new(MomentumStrategy),
    ];
    for s in all {
        r.register(
            s.virtual_reason(),
            "v1",
            s.description(),
            s.virtual_reason(),
        );
    }
}

pub mod _helpers;
pub mod auction_anomaly;
pub mod breakout;
pub mod llm_select;
pub mod main_net_inflow;
pub mod momentum;
pub mod news_catalyst;
pub mod sector_leader;
pub mod volume_surge;

pub use auction_anomaly::AuctionAnomalyStrategy;
pub use breakout::BreakoutStrategy;
pub use llm_select::LLMSelectStrategy;
pub use main_net_inflow::MainNetInflowStrategy;
pub use momentum::MomentumStrategy;
pub use news_catalyst::NewsCatalystStrategy;
pub use sector_leader::SectorLeaderStrategy;
pub use volume_surge::VolumeSurgeStrategy;

/// Fix review #1 (HIGH): Strategy::id() 稳定
///
/// 用 `OnceLock<StrategyId>` 缓存, 首次调生成, 后续返回同 id.
/// 8 strategy impl 全用此 macro, 避免每次 new_strategy_id 返新 id (counter++).
#[macro_export]
macro_rules! impl_strategy_id {
    ($struct:ident, $name:expr) => {
        fn id(&self) -> $crate::bus::StrategyId {
            use ::std::sync::OnceLock;
            use $crate::bus::new_strategy_id;
            static CACHED: OnceLock<$crate::bus::StrategyId> = OnceLock::new();
            CACHED.get_or_init(|| new_strategy_id($name, "v1")).clone()
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_input(kind: &str, vol: f64) -> StrategyInput {
        StrategyInput {
            code: "TEST_CODE_000001".to_string(),
            push_price: 10.0,
            // Fix v16.4 完整化: chg + 0.1 (momentum/sector_leader 拒 chg<=0)
            metric_json: serde_json::json!({"vol_ratio": vol, "push_subkind": "Test", "price_chg_pct": 0.1, "sector": "AI"}).to_string(),
            push_kind: kind.to_string(),
            now: chrono::Local::now(),
        }
    }

    #[test]
    fn news_catalyst_scores_7() {
        let s = NewsCatalystStrategy;
        let out = s.score(&make_input("D-01", 0.0)).expect("should score");
        assert_eq!(out.score, 7.0);
    }

    #[test]
    fn momentum_scores_8_highest() {
        let s = MomentumStrategy;
        // v16.4 完整化: 8.0 base + chg 0.1 * 0.2 = 8.02
        let out = s.score(&make_input("Momentum", 8.0)).expect("should score");
        assert!(
            out.score >= 8.0 && out.score <= 9.0,
            "Momentum 应 8.0-9.0 实际 {}",
            out.score
        );
    }

    #[test]
    fn auction_volume_low_vol_skipped() {
        let s = AuctionAnomalyStrategy;
        let out = s.score(&make_input("P-02", 2.0));
        assert!(out.is_none());
    }

    #[test]
    fn auction_volume_high_vol_passes() {
        let s = AuctionAnomalyStrategy;
        let out = s.score(&make_input("P-02", 8.0)).expect("should score");
        assert!(out.score > 0.0);
    }

    #[test]
    fn volume_surge_scores_6_5() {
        let s = VolumeSurgeStrategy;
        // v16.4 完整化: 6.5 base + (vol - 2.0) * 0.15 = 6.5 (vol=0 不拒, vol<2 拒... 实际 vol=0 时 score 6.5)
        // vol=0 时公式 (0-2)*0.15 = -0.3, max(0, 6.5-0.3) = 6.2, 但 score = 6.5 - 0 = 6.5
        // 实际: 6.5 + (0-2)*0.15 = 6.2 (负数), .min(1.5) 限制 min 0, max 0 → 6.5 - 0.3 = 6.2
        // 改: 传 vol=3.0 (≥2 拒阈值), 6.5 + (3-2)*0.15 = 6.65
        let out = s.score(&make_input("P-02", 3.0)).expect("should score");
        assert!(
            out.score >= 6.5 && out.score <= 8.0,
            "VolumeSurge 应 6.5-8.0 实际 {}",
            out.score
        );
    }

    #[test]
    fn unknown_kind_returns_none() {
        let s: Box<dyn Strategy> = Box::new(SectorLeaderStrategy);
        let out = s.score(&make_input("UnknownKind", 0.0));
        assert!(out.is_none());
    }

    #[test]
    fn register_all_8_strategies() {
        register_all();
        let r = crate::registry::StrategyRegistry::global();
        let all = r.list_all();
        let count = all
            .iter()
            .filter(|m| {
                m.name != "Overwrite" && m.name != "TestActive" && m.name != "TestReactivate"
            })
            .count();
        assert!(count >= 8, "8 strategy 应注册, 实际 {}", count);
    }

    #[test]
    fn strategy_id_is_stable_across_calls() {
        // Fix review #1: Strategy::id() 每次返回同 id
        let s1 = NewsCatalystStrategy;
        let id1 = s1.id();
        let id2 = s1.id();
        assert_eq!(id1, id2, "Strategy::id() 应稳定, 多次调返同 id");
    }

    #[test]
    fn register_overwrites_same_name_version() {
        // Fix review #2: 同 (name, version) 注册覆盖, 复用首次 id
        use crate::registry::StrategyRegistry;
        let r = StrategyRegistry::global();
        let id1 = r.register("TestOverwriteUnique", "v9", "first", "Label1");
        let id2 = r.register("TestOverwriteUnique", "v9", "second", "Label2");
        assert_eq!(id1, id2, "同 (name, version) 覆盖应复用首次 id");
        let meta = r.lookup(&id1).expect("应找到");
        assert_eq!(meta.description, "second", "应覆盖 description");
        let count = r
            .list_all()
            .iter()
            .filter(|m| m.name == "TestOverwriteUnique" && m.version == "v9")
            .count();
        assert_eq!(count, 1, "同 (name, version) 应只 1 entry, 不累积");
    }
}
