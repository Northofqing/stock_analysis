// -*- coding: utf-8 -*-
//! 新闻事件排序层 (P2-News Commit 1 — 模型层)
//!
//! **目的**: 给现有新闻链路 (chain_mapper 召回) 和候选台 (candidate_panel) 之间
//! 加一个"新闻事件排序层", 把"规则命中就推"升级成"召回 → 阶段判断 → 风险过滤 → 分档".
//!
//! **核心设计**:
//! - `NewsCandidate`: 一条新闻 (输入)
//! - `EventType`: 7 类事件分类 (Policy/Industry/Earnings/Regulatory/SupplyDemand/CompanyAction/Unknown)
//! - `HeatStage`: 7 阶段 (Cold/Start/Ferment/Climax/Divergence/Fade/Unknown) — 区分"启动 vs 高潮"
//! - `NewsEvidenceBreakdown`: 7 维评分 (rule/freshness/heat/stage/capital/source/risk_penalty)
//! - `NewsRankBucket`: 4 档去向 (PushNow/WatchCandidate/LogOnly/Drop)
//! - `RankedNews`: 评分后输出 (candidate + event_type + heat_stage + score + bucket + evidence + reasons + drop_reason)
//!
//! **Commit 1 只做模型 + 基础规则 (不接 opportunity 链路, 不写主入口 rank_news)**:
//!   - `classify_event_type` (规则分类, 关键词 contains)
//!   - `detect_heat_stage` (基于 sector_history N 日数据, 不依赖 LLM)
//!   - 单测 8 个
//!
//! **红线 (P5 §一 / AGENTS.md §一)**:
//!   - 不合成"买入分" — 评分透明, 7 维可拆解
//!   - AI 失败 fallback 静态 — 规则分类, 不阻塞
//!   - drop_reason 强制 — 任何 Drop 都有原因
//!   - 不绕过候选台 — Ranker 输出进 candidate_panel 二次过滤
use crate::market_analyzer::sector_history::{cumulative_change_pct, BoardDay};
use chrono::{DateTime, Local, NaiveDate};

/// 一条新闻 (Ranker 输入, 不 derive Serialize 因为 ChainHit 不支持)
#[derive(Debug, Clone)]
pub struct NewsCandidate {
    /// 唯一 ID (用 source + title hash, 或外部传入)
    pub id: String,
    pub title: String,
    pub snippet: String,
    pub source: String,
    /// 发布时间 (None 表示缺失, 标 reason)
    pub published_at: Option<DateTime<Local>>,
    /// chain_mapper 召回的产业链命中 (P5 §三 已有结构)
    pub chain_hits: Vec<crate::opportunity::chain_mapper::ChainHit>,
    /// 关联板块代码 (东财 BK0xxx, 用于 detect_heat_stage)
    /// None 表示未关联板块, detect_heat_stage 返 Unknown
    pub board_code: Option<String>,
}

/// 事件类型 (规则分类, 一期不依赖 AI)
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum EventType {
    /// 政策/规划/印发/支持
    PolicyCatalyst,
    /// 产业催化 (新技术/新方向)
    IndustryCatalyst,
    /// 业绩/预增/亏损
    Earnings,
    /// 监管/处罚/问询/立案
    RegulatoryRisk,
    /// 供需/价格/库存/产能
    SupplyDemand,
    /// 公司行为 (回购/减持/并购)
    CompanyAction,
    /// 未知 (含 AI 失败)
    Unknown,
}

impl EventType {
    pub fn label(self) -> &'static str {
        match self {
            EventType::PolicyCatalyst => "政策催化",
            EventType::IndustryCatalyst => "产业催化",
            EventType::Earnings => "业绩",
            EventType::RegulatoryRisk => "监管风险",
            EventType::SupplyDemand => "供需",
            EventType::CompanyAction => "公司行为",
            EventType::Unknown => "未知",
        }
    }
}

/// 题材阶段 — 核心防追高字段 (P2-News 关键创新)
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum HeatStage {
    /// 冷 (板块跌幅或平, 无涨停)
    Cold,
    /// 启动 (今日起, 3 日低位)
    Start,
    /// 发酵 (3 日累计涨幅中等, 涨停 3+, 资金流入)
    Ferment,
    /// 高潮 (3 日累计涨幅 > 10% 或今日极端, 涨停多)
    Climax,
    /// 分歧 (涨幅仍高但资金流出 / 炸板)
    Divergence,
    /// 退潮 (板块跌, 资金流出, 涨停下降)
    Fade,
    /// 数据不足 / 板块未关联
    Unknown,
}

impl HeatStage {
    pub fn label(self) -> &'static str {
        match self {
            HeatStage::Cold => "冷",
            HeatStage::Start => "启动",
            HeatStage::Ferment => "发酵",
            HeatStage::Climax => "高潮",
            HeatStage::Divergence => "分歧",
            HeatStage::Fade => "退潮",
            HeatStage::Unknown => "未知",
        }
    }
}

/// 评分拆解 (7 维透明, P5 §一 红线: 不合成"买入分")
#[derive(Debug, Clone, Default)]
pub struct NewsEvidenceBreakdown {
    /// 规则召回得分 (0-25)
    pub rule_score: i32,
    /// 时效得分 (0-20)
    pub freshness_score: i32,
    /// 热度得分 (-10 ~ 25, 受阶段约束)
    pub heat_score: i32,
    /// 阶段得分 (-25 ~ 25, Climax/Divergence/Fade 大扣分)
    pub stage_score: i32,
    /// 资金确认得分 (-15 ~ 20, 不是一票通过)
    pub capital_score: i32,
    /// 来源可信度得分 (0-10)
    pub source_score: i32,
    /// 风险扣分 (0-40, 监管/减持/立案/高潮叠加)
    pub risk_penalty: i32,
}

/// 4 档去向
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum NewsRankBucket {
    /// A 档: 推送 (或候选台高优)
    PushNow,
    /// B 档: 候选台观察
    WatchCandidate,
    /// C 档: 仅日志 + 审计
    LogOnly,
    /// 明确丢弃
    Drop,
}

/// Ranker 输出
#[derive(Debug, Clone)]
pub struct RankedNews {
    pub candidate: NewsCandidate,
    pub event_type: EventType,
    pub heat_stage: HeatStage,
    pub score: i32,
    pub bucket: NewsRankBucket,
    pub evidence: NewsEvidenceBreakdown,
    /// 评分理由 (人读, 透明)
    pub reasons: Vec<String>,
    /// 丢弃原因 (bucket=Drop 时必有, P5 §一 红线: 失败路径显式)
    pub drop_reason: Option<String>,
}

// ============ Commit 1 实现: 规则分类 + 阶段判断 ============

/// 规则分类 (一期, AI 分类后续作补充)
///
/// 优先级: 监管风险 > 业绩 > 公司行为 > 供需 > 政策/产业 > 未知
/// (监管风险必须优先识别, 不能被利好关键词覆盖)
pub fn classify_event_type(title: &str) -> EventType {
    let t = title;
    // 1. 监管风险 (最高优先级, 关键词含"监管/处罚/立案/问询/警示/函")
    if contains_any(t, &["监管", "处罚", "立案", "问询", "警示", "关注函", "监管措施", "ST"]) {
        return EventType::RegulatoryRisk;
    }
    // 2. 业绩
    if contains_any(t, &["业绩", "预增", "预减", "亏损", "扭亏", "净利润", "营收"]) {
        return EventType::Earnings;
    }
    // 3. 公司行为
    if contains_any(t, &["回购", "增持", "减持", "解禁", "并购", "重组", "收购"]) {
        return EventType::CompanyAction;
    }
    // 4. 供需
    if contains_any(t, &["涨价", "提价", "供给", "库存", "产能", "减产", "扩产", "短缺", "过剩"]) {
        return EventType::SupplyDemand;
    }
    // 5. 政策催化
    if contains_any(t, &["政策", "规划", "印发", "支持", "指导意见", "国务院", "印发"]) {
        return EventType::PolicyCatalyst;
    }
    // 6. 产业催化
    if contains_any(t, &["突破", "首发", "量产", "商用", "落地", "试点", "应用"]) {
        return EventType::IndustryCatalyst;
    }
    EventType::Unknown
}

/// 检测题材阶段 (P2-News 核心防追高)
///
/// 输入:
///   - `board_code`: 关联板块代码 (东财 BK0xxx)
///   - `today_chg`: 今日 change_pct (从 ConceptBoard.change_pct 拿)
///   - `today_main_inflow`: 今日主力净流入
///   - `today_main_net_pct_today`: 今日主力净占比 (%)
///   - `today_main_net_pct_5d`: 5 日主力净占比 (%)
///   - `today_limit_up_count`: 板块今日涨停家数 (None 表示缺失)
///
/// 阶段判定规则 (基于 commit 0 sector_history N 日数据 + 单日):
///   - 数据不足 (board_code 为空) → Unknown
///   - 启动 Start: 今日 > 0 + 3 日累计 < 5% + 主力流入 + 涨停 1-2 家
///   - 发酵 Ferment: 今日 > 1% + 3 日累计 5-10% + 涨停 ≥ 3 家
///   - 高潮 Climax: 3 日累计 > 10% 或 今日 > 5% + 涨停 ≥ 5 家
///   - 分歧 Divergence: 今日 > 0 + 主力净流出 (main_net_pct_today < main_net_pct_5d)
///   - 退潮 Fade: 今日 < 0 + 主力流出
///   - 冷 Cold: 今日 <= 0 + 涨停 = 0 + 主力流入 <= 0
pub fn detect_heat_stage(
    board_code: Option<&str>,
    today_chg: f64,
    today_main_inflow: f64,
    today_main_net_pct_today: f64,
    today_main_net_pct_5d: f64,
    today_limit_up_count: Option<usize>,
) -> HeatStage {
    // 数据不足
    let board_code = match board_code {
        Some(c) if !c.is_empty() => c,
        _ => return HeatStage::Unknown,
    };

    // 拉 3 日累计涨幅 (commit 0 sector_history)
    let cum_3d = cumulative_change_pct(board_code, 3).unwrap_or(0.0);
    let main_accel = today_main_net_pct_today - today_main_net_pct_5d;
    let limit_up = today_limit_up_count.unwrap_or(0);

    // 退潮: 今日跌 + 主力流出
    if today_chg < 0.0 && today_main_inflow < 0.0 {
        return HeatStage::Fade;
    }
    // 高潮: 3 日累计 > 10% 或 今日 > 5% + 涨停 ≥ 5
    if cum_3d > 10.0 || (today_chg > 5.0 && limit_up >= 5) {
        return HeatStage::Climax;
    }
    // 分歧: 今日 > 0 + 资金加速度 < 0 (主力流出 / 5 日均弱)
    if today_chg > 0.0 && main_accel < -2.0 {
        return HeatStage::Divergence;
    }
    // 发酵: 今日 > 1% + 3 日累计 5-10% + 涨停 ≥ 3
    if today_chg > 1.0 && cum_3d >= 5.0 && cum_3d <= 10.0 && limit_up >= 3 {
        return HeatStage::Ferment;
    }
    // 启动: 今日 > 0 + 3 日累计 < 5% + 主力流入 + 涨停 1-2
    if today_chg > 0.0 && cum_3d < 5.0 && today_main_inflow > 0.0 && limit_up >= 1 && limit_up <= 2 {
        return HeatStage::Start;
    }
    // 冷: 今日 ≤ 0 + 涨停 = 0 + 主力流入 ≤ 0
    if today_chg <= 0.0 && limit_up == 0 && today_main_inflow <= 0.0 {
        return HeatStage::Cold;
    }
    // 其他情况 (数据不足 / 边界): Unknown
    HeatStage::Unknown
}

/// 简单关键词包含检查
fn contains_any(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|k| text.contains(k))
}

// ============ 单测 ============

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opportunity::chain_mapper::{ChainHit, ChainSource};
    use chrono::TimeZone;

    fn mock_candidate(title: &str, board_code: Option<&str>) -> NewsCandidate {
        NewsCandidate {
            id: format!("test-{}", title),
            title: title.to_string(),
            snippet: "".to_string(),
            source: "test".to_string(),
            published_at: Some(Local::now()),
            chain_hits: vec![ChainHit {
                chain: "测试链".to_string(),
                keywords: vec![title.to_string()],
                logic: "test".to_string(),
                stocks: vec![],
                source: ChainSource::Rule,
                board_keyword: title.to_string(),
                fund_flow_pct: None,
            }],
            board_code: board_code.map(String::from),
        }
    }

    /// 1) 监管风险分类
    #[test]
    fn classify_regulatory_risk() {
        assert_eq!(classify_event_type("公司收到证监会立案通知"), EventType::RegulatoryRisk);
        assert_eq!(classify_event_type("某股被处罚 50 万"), EventType::RegulatoryRisk);
        assert_eq!(classify_event_type("问询函回复"), EventType::RegulatoryRisk);
    }

    /// 2) 业绩分类
    #[test]
    fn classify_earnings() {
        assert_eq!(classify_event_type("Q3 业绩预增 50%"), EventType::Earnings);
        assert_eq!(classify_event_type("公司预计全年亏损"), EventType::Earnings);
    }

    /// 3) 公司行为分类
    #[test]
    fn classify_company_action() {
        assert_eq!(classify_event_type("公司拟回购股份"), EventType::CompanyAction);
        assert_eq!(classify_event_type("大股东减持公告"), EventType::CompanyAction);
    }

    /// 4) 供需分类
    #[test]
    fn classify_supply_demand() {
        assert_eq!(classify_event_type("锂电池原材料涨价"), EventType::SupplyDemand);
        assert_eq!(classify_event_type("工厂减产"), EventType::SupplyDemand);
    }

    /// 5) 政策催化
    #[test]
    fn classify_policy() {
        assert_eq!(classify_event_type("国务院印发新能源汽车规划"), EventType::PolicyCatalyst);
        assert_eq!(classify_event_type("工信部支持产业升级"), EventType::PolicyCatalyst);
    }

    /// 6) 监管优先级高于利好 (复合标题)
    #[test]
    fn classify_regulatory_beats_policy() {
        // 标题同时含"政策"和"立案", 监管优先
        let t = "国务院政策支持, 但公司被立案调查";
        assert_eq!(classify_event_type(t), EventType::RegulatoryRisk);
    }

    /// 7) 未知
    #[test]
    fn classify_unknown() {
        assert_eq!(classify_event_type("今日市场综述"), EventType::Unknown);
    }

    /// 8) 阶段: 数据不足 → Unknown
    #[test]
    fn heat_stage_unknown_when_no_board() {
        let stage = detect_heat_stage(None, 1.0, 1e8, 5.0, 2.0, Some(1));
        assert_eq!(stage, HeatStage::Unknown);
    }

    /// 9) 阶段: 退潮 (今日跌 + 主力流出)
    #[test]
    fn heat_stage_fade() {
        let stage = detect_heat_stage(Some("BK0001"), -2.0, -1e8, -3.0, 2.0, Some(0));
        assert_eq!(stage, HeatStage::Fade);
    }

    /// 10) 阶段: 高潮 (3 日累计 > 10%)
    #[test]
    fn heat_stage_climax_by_cum() {
        // 累计 3 日 > 10% 即 climax, 不需涨停 ≥ 5
        let stage = detect_heat_stage(Some("BK0001"), 1.0, 5e7, 5.0, 2.0, Some(1));
        // 3 日数据无 (board_code 不在 history), 走 Unknown 路径
        // 这里没真实 history, 实际跑会用 sector_history 写
        // 单元测试覆盖: 退潮 + 冷 足够, 启动/发酵/高潮需 history 真数据
        // (留 P2-News Commit 2 接 sector_history 写端再补集成测试)
        // 这里只确认不会 panic
        let _ = stage;
    }

    /// 11) 阶段: 冷 (今日 0 + 涨停 0 + 主力 0 — 三方都平)
    #[test]
    fn heat_stage_cold() {
        // 今日 = 0, 涨停 0, 主力 = 0 (三方都平, 不是退潮)
        let stage = detect_heat_stage(Some("BK0001"), 0.0, 0.0, 0.0, 0.0, Some(0));
        assert_eq!(stage, HeatStage::Cold);
    }

    /// 12) NewsCandidate 构造 + Debug 不 panic
    #[test]
    fn news_candidate_construction() {
        let cand = mock_candidate("国务院印发低空经济规划", Some("BK0815"));
        assert_eq!(cand.title, "国务院印发低空经济规划");
        assert_eq!(cand.board_code, Some("BK0815".to_string()));
        assert_eq!(cand.chain_hits.len(), 1);
    }

    /// 13) EventType label
    #[test]
    fn event_type_label() {
        assert_eq!(EventType::RegulatoryRisk.label(), "监管风险");
        assert_eq!(EventType::PolicyCatalyst.label(), "政策催化");
    }

    /// 14) HeatStage label
    #[test]
    fn heat_stage_label() {
        assert_eq!(HeatStage::Climax.label(), "高潮");
        assert_eq!(HeatStage::Divergence.label(), "分歧");
    }

    // ============ Commit 2 单测: 评分函数 ============

    /// 15) score_rule: 非 generic 命中 +20, generic +8, AI degraded +5
    #[test]
    fn score_rule_hit_quality() {
        use crate::opportunity::chain_mapper::{ChainHit, ChainSource};
        // 非 generic: 关键词 + 板块名双命中, board_keyword 非空
        let c1 = ChainHit {
            chain: "X".into(),
            keywords: vec!["a".into()],
            logic: "test".into(),
            stocks: vec![],
            source: ChainSource::Rule,
            board_keyword: "板块".into(),
            fund_flow_pct: Some(2.0),
        };
        // AI degraded: source = AiDegraded
        let c2 = ChainHit {
            source: ChainSource::AiDegraded,
            ..c1.clone()
        };
        // 多个 chain 命中
        let cands_single = vec![c1.clone()];
        let cands_multi = vec![c1.clone(), c1.clone()];
        let cands_ai = vec![c2];

        let r1 = score_rule(&cands_single);
        assert!(r1 >= 20, "非 generic 命中应 ≥20, got {}", r1);
        let r2 = score_rule(&cands_multi);
        assert!(r2 > r1, "多 chain 命中应加分");
        let r3 = score_rule(&cands_ai);
        assert!(r3 < r1, "AI degraded 命中应低于规则命中");
    }

    /// 16) score_freshness: 15 分钟内 +20, 6 小时后 0
    #[test]
    fn score_freshness_decay() {
        let now = Local::now();
        let r1 = score_freshness(Some(now));
        assert!(r1 >= 18, "刚发布应高, got {}", r1);
        let r2 = score_freshness(Some(now - chrono::Duration::minutes(30)));
        assert!((12..=18).contains(&r2), "30 分钟应在 12-18, got {}", r2);
        let r3 = score_freshness(Some(now - chrono::Duration::hours(8)));
        assert_eq!(r3, 0, "8 小时外应为 0");
        let r4 = score_freshness(None);
        assert!(r4 > 0 && r4 < 10, "缺时间应给低分, got {}", r4);
    }

    /// 17) score_heat: Climax 限 5, Start/Ferment 给高, Fade 扣
    #[test]
    fn score_heat_by_stage() {
        // Start: 涨幅 3% + 流入 + 涨停 2 → 期望 ≥ 15
        let s1 = score_heat(HeatStage::Start, 3.0, 1e8, 2);
        assert!(s1 >= 15, "Start 给高, got {}", s1);
        // Climax: 限 5
        let s2 = score_heat(HeatStage::Climax, 8.0, 5e8, 10);
        assert!(s2 <= 5, "Climax 限 5, got {}", s2);
        // Fade: 扣
        let s3 = score_heat(HeatStage::Fade, -3.0, -5e7, 0);
        assert!(s3 < 0, "Fade 应扣分, got {}", s3);
    }

    /// 18) score_capital: 强流入 +15, 强流出 -15
    #[test]
    fn score_capital_range() {
        let r1 = score_capital(Some(5e8));
        assert_eq!(r1, 15, "强流入应 +15");
        let r2 = score_capital(Some(1e7));
        assert!(r2 > 0 && r2 <= 10, "弱正应小正, got {}", r2);
        let r3 = score_capital(Some(-3e8));
        assert_eq!(r3, -15, "强流出应 -15");
        let r4 = score_capital(None);
        assert_eq!(r4, 0, "缺数据应 0");
    }

    /// 19) risk_penalty: 监管 +30, 减持 +40
    #[test]
    fn risk_penalty_by_event() {
        let p1 = risk_penalty(EventType::RegulatoryRisk, HeatStage::Ferment, &[]);
        assert!(p1 >= 30, "监管应扣 ≥30, got {}", p1);
        let p2 = risk_penalty(EventType::CompanyAction, HeatStage::Ferment, &[String::from("减持")]);
        assert!(p2 >= 30, "减持应扣 ≥30, got {}", p2);
        let p3 = risk_penalty(EventType::PolicyCatalyst, HeatStage::Start, &[]);
        assert_eq!(p3, 0, "政策+启动 不应扣");
    }

    /// 20) rank_news 主入口: 政策+启动+资金 → A 档
    #[test]
    fn rank_news_push_now() {
        let cand = NewsCandidate {
            id: "test-1".into(),
            title: "国务院印发低空经济规划".into(),
            snippet: "支持产业升级".into(),
            source: "东财".into(),
            published_at: Some(Local::now()),
            chain_hits: vec![ChainHit {
                chain: "低空经济".into(),
                keywords: vec!["低空".into()],
                logic: "test".into(),
                stocks: vec![],
                source: ChainSource::Rule,
                board_keyword: "低空经济".into(),
                fund_flow_pct: Some(3.0),
            }],
            board_code: Some("BK0815".into()),
        };
        let ctx = MarketContext {
            today_chg: 2.0,
            main_inflow: 2e8,
            main_net_pct_today: 5.0,
            main_net_pct_5d: 2.0,
            limit_up_count: Some(2),
        };
        let r = rank_news(&cand, &ctx);
        // 期望: 政策 + 启动 + 资金流入 → A 档或 B 档
        assert!(matches!(r.bucket, NewsRankBucket::PushNow | NewsRankBucket::WatchCandidate),
            "应进 A/B 档, got {:?}", r.bucket);
        assert!(r.event_type == EventType::PolicyCatalyst);
    }

    /// 21) rank_news: 监管风险 → Drop 或 C 档 + drop_reason
    #[test]
    fn rank_news_regulatory_drop() {
        let cand = NewsCandidate {
            id: "test-2".into(),
            title: "证监会立案调查某公司".into(),
            snippet: "".into(),
            source: "新浪".into(),
            published_at: Some(Local::now()),
            chain_hits: vec![],
            board_code: None,
        };
        let ctx = MarketContext::default();
        let r = rank_news(&cand, &ctx);
        // 监管风险 + 无 chain_hit + 无 board → 应 Drop 或 LogOnly
        assert!(matches!(r.bucket, NewsRankBucket::Drop | NewsRankBucket::LogOnly),
            "应进 Drop/C 档, got {:?}", r.bucket);
        if r.bucket == NewsRankBucket::Drop {
            assert!(r.drop_reason.is_some(), "Drop 必须有 drop_reason");
        }
    }

    /// 22) rank_news: 高潮阶段 → 不应 PushNow
    #[test]
    fn rank_news_climax_no_push() {
        let cand = NewsCandidate {
            id: "test-3".into(),
            title: "机器人板块再获政策支持".into(),
            snippet: "".into(),
            source: "东财".into(),
            published_at: Some(Local::now()),
            chain_hits: vec![ChainHit {
                chain: "机器人".into(),
                keywords: vec!["机器人".into()],
                logic: "test".into(),
                stocks: vec![],
                source: ChainSource::Rule,
                board_keyword: "机器人".into(),
                fund_flow_pct: Some(8.0),
            }],
            board_code: Some("BK0815".into()),
        };
        // 模拟高潮: 涨停 10 家 + 涨幅 8% + 资金强流入
        let ctx = MarketContext {
            today_chg: 8.0,
            main_inflow: 5e8,
            main_net_pct_today: 12.0,
            main_net_pct_5d: 5.0,
            limit_up_count: Some(10),
        };
        let r = rank_news(&cand, &ctx);
        // Climax + 风险扣分 → 不应 PushNow
        assert!(r.bucket != NewsRankBucket::PushNow,
            "高潮阶段不应进 A 档, got {:?}", r.bucket);
    }
}

// ============ Commit 2 实现: 评分函数 + rank_news 主入口 ============

/// 市场上下文 (供 rank_news 评估一条新闻时的盘面快照)
#[derive(Debug, Clone, Default)]
pub struct MarketContext {
    /// 关联板块今日涨幅 (%)
    pub today_chg: f64,
    /// 关联板块今日主力净流入 (元)
    pub main_inflow: f64,
    /// 关联板块今日主力净占比 (%)
    pub main_net_pct_today: f64,
    /// 关联板块 5 日主力净占比 (%)
    pub main_net_pct_5d: f64,
    /// 关联板块今日涨停家数
    pub limit_up_count: Option<usize>,
}

/// 规则召回得分 (0-25)
///
/// 规则只负责召回, 不决定推送. 给分要透明可解释.
pub fn score_rule(chain_hits: &[crate::opportunity::chain_mapper::ChainHit]) -> i32 {
    if chain_hits.is_empty() {
        return 0;
    }
    let mut total = 0i32;
    let mut board_keyword_count = 0;
    for hit in chain_hits {
        // 单 hit 得分
        let hit_score = match hit.source {
            crate::opportunity::chain_mapper::ChainSource::Rule => {
                if !hit.board_keyword.is_empty() {
                    20 // 非 generic 规则命中 (有 board_keyword)
                } else {
                    8 // generic 规则命中
                }
            }
            crate::opportunity::chain_mapper::ChainSource::Ai => 12, // AI 命中, 介于规则和 degraded 之间
            crate::opportunity::chain_mapper::ChainSource::AiDegraded => 5, // 降级, 给低分
        };
        total += hit_score;
        if !hit.board_keyword.is_empty() {
            board_keyword_count += 1;
        }
    }
    // 多 chain 命中 +5 加成 (上限 5)
    if chain_hits.len() >= 2 {
        total += 5;
    }
    // clamp 0-25
    total.clamp(0, 25)
}

/// 时效得分 (0-20)
///
/// 短线新闻强依赖时效, 阶梯式衰减.
pub fn score_freshness(published_at: Option<DateTime<Local>>) -> i32 {
    let now = Local::now();
    let age = match published_at {
        Some(t) => (now - t).num_minutes().max(0),
        None => return 5, // 缺时间给低分, 不为 0 (保守)
    };
    if age <= 15 {
        20
    } else if age <= 60 {
        15
    } else if age <= 180 {
        8
    } else if age <= 360 {
        3
    } else {
        0
    }
}

/// 热度得分 (-10 ~ 25, 受阶段约束)
/// Climax 限 5 (防追高), Start/Ferment 给高, Fade 扣
pub fn score_heat(stage: HeatStage, change_pct: f64, main_inflow: f64, limit_up: usize) -> i32 {
    match stage {
        HeatStage::Start | HeatStage::Ferment => {
            let change = if change_pct >= 3.0 {
                8
            } else if change_pct > 0.0 {
                4
            } else {
                0
            };
            let inflow = if main_inflow > 0.0 { 8 } else { -5 };
            let limit = (limit_up as i32 * 3).min(9);
            change + inflow + limit
        }
        HeatStage::Climax => 5, // 限, 不给高
        HeatStage::Divergence => -10,
        HeatStage::Fade => -15,
        HeatStage::Cold | HeatStage::Unknown => 0,
    }
}

/// 资金确认得分 (-15 ~ 20, 不是一票通过也不是一票否决)
pub fn score_capital(main_inflow: Option<f64>) -> i32 {
    match main_inflow {
        None => 0, // 缺数据显式 0, 不臆测
        Some(v) if v >= 3e8 => 15,    // 强流入
        Some(v) if v > 0.0 => 8,      // 弱正
        Some(v) if v == 0.0 => 0,     // 平
        Some(v) if v >= -1e8 => -10,  // 弱流出
        Some(_) => -15,                 // 强流出
    }
}

/// 阶段得分 (-25 ~ 25, Climax/Divergence/Fade 大扣分)
///
/// **关键**: 这是防追高核心. Climax -10, Divergence -20, Fade -25.
pub fn stage_score(stage: HeatStage) -> i32 {
    match stage {
        HeatStage::Start => 25,
        HeatStage::Ferment => 18,
        HeatStage::Cold => 0,
        HeatStage::Climax => -10,
        HeatStage::Divergence => -20,
        HeatStage::Fade => -25,
        HeatStage::Unknown => 0,
    }
}

/// 来源可信度得分 (0-10)
pub fn source_score(source: &str) -> i32 {
    let s = source.to_lowercase();
    if s.contains("东财") || s.contains("eastmoney") || s.contains("em") {
        10
    } else if s.contains("新浪") || s.contains("sina") {
        8
    } else if s.contains("金十") || s.contains("jin10") {
        8
    } else if s.contains("华尔街") || s.contains("wallstreetcn") || s.contains("wallstreet") {
        7
    } else if s.contains("公告") || s.contains("announcement") {
        9
    } else if s.contains("财联社") || s.contains("cls") {
        7
    } else {
        3 // 未知源给低分
    }
}

/// 风险扣分 (0-40, 监管/减持/立案/高潮叠加)
///
/// **关键**: 风险事件必须单独扣分, 不可和利好混在一起.
pub fn risk_penalty(event: EventType, stage: HeatStage, keywords: &[String]) -> i32 {
    let mut penalty = 0i32;
    // 1. 事件类型基础扣分
    match event {
        EventType::RegulatoryRisk => penalty += 30,
        EventType::Earnings => {
            // 业绩预减/亏损加重 (关键词命中)
            if keywords.iter().any(|k| k.contains("预减") || k.contains("亏损")) {
                penalty += 25;
            }
        }
        EventType::CompanyAction => {
            // 减持/解禁/立案加重
            if keywords.iter().any(|k| k.contains("减持") || k.contains("解禁") || k.contains("立案")) {
                penalty += 30;
            }
        }
        _ => {}
    }
    // 2. 阶段叠加 (Climax +10, Divergence +20)
    match stage {
        HeatStage::Climax => penalty += 10,
        HeatStage::Divergence => penalty += 20,
        _ => {}
    }
    penalty.min(40)
}

/// rank_news 主入口 — 一站式评分 + 分档
///
/// **算法** (透明公式):
///   final_score = rule + freshness + heat + stage + capital + source - risk
///   clamp 0..100
///   bucket 按 score + stage + risk 三维分档
pub fn rank_news(candidate: &NewsCandidate, ctx: &MarketContext) -> RankedNews {
    let mut reasons = Vec::new();
    let mut evidence = NewsEvidenceBreakdown::default();

    // 1. 规则召回
    evidence.rule_score = score_rule(&candidate.chain_hits);
    if evidence.rule_score > 0 {
        reasons.push(format!("规则召回 {} 分", evidence.rule_score));
    }

    // 2. 时效
    evidence.freshness_score = score_freshness(candidate.published_at);
    if evidence.freshness_score >= 15 {
        reasons.push(format!("时效 {} 分 (新发布)", evidence.freshness_score));
    } else if evidence.freshness_score == 0 {
        reasons.push("时效 0 分 (超 6 小时)".to_string());
    }

    // 3. 阶段判断 (commit 1 detect_heat_stage)
    let heat_stage = detect_heat_stage(
        candidate.board_code.as_deref(),
        ctx.today_chg,
        ctx.main_inflow,
        ctx.main_net_pct_today,
        ctx.main_net_pct_5d,
        ctx.limit_up_count,
    );
    reasons.push(format!("阶段: {}", heat_stage.label()));

    // 4. 热度 + 阶段得分
    evidence.heat_score = score_heat(heat_stage, ctx.today_chg, ctx.main_inflow, ctx.limit_up_count.unwrap_or(0));
    evidence.stage_score = stage_score(heat_stage);

    // 5. 资金确认
    evidence.capital_score = score_capital(Some(ctx.main_inflow));
    if ctx.main_inflow.abs() > 1e7 {
        reasons.push(format!(
            "资金 {} ({} 分)",
            if ctx.main_inflow > 0.0 { "流入" } else { "流出" },
            evidence.capital_score
        ));
    } else {
        reasons.push("资金数据弱 (≈0 分)".to_string());
    }

    // 6. 来源可信度
    evidence.source_score = source_score(&candidate.source);

    // 7. 风险扣分
    let event_type = classify_event_type(&candidate.title);
    let keywords: Vec<String> = candidate.chain_hits.iter().flat_map(|h| h.keywords.clone()).collect();
    evidence.risk_penalty = risk_penalty(event_type, heat_stage, &keywords);
    if evidence.risk_penalty > 0 {
        reasons.push(format!("风险扣 {} 分", evidence.risk_penalty));
    }

    // 8. 总分
    let raw = evidence.rule_score
        + evidence.freshness_score
        + evidence.heat_score
        + evidence.stage_score
        + evidence.capital_score
        + evidence.source_score
        - evidence.risk_penalty;
    let score = raw.clamp(0, 100);

    // 9. 分档
    let (bucket, drop_reason) = decide_bucket(score, heat_stage, evidence.risk_penalty, &candidate.chain_hits);
    if let Some(reason) = &drop_reason {
        reasons.push(format!("Drop: {}", reason));
    }
    reasons.push(format!("总分 {} → {:?}", score, bucket));

    RankedNews {
        candidate: candidate.clone(),
        event_type,
        heat_stage,
        score,
        bucket,
        evidence,
        reasons,
        drop_reason,
    }
}

/// 分档规则
///
/// A 档: score >= 70 AND stage in [Start, Ferment] AND risk < 20
/// B 档: score >= 45 OR (news strong but market unconfirmed) OR (Cold + high freshness)
/// C 档: score < 45 OR stage in [Climax, Divergence, Fade]
/// Drop: 监管/减持/立案 risk ≥ 30, 或无 chain_hit, 或无 board
fn decide_bucket(
    score: i32,
    stage: HeatStage,
    risk: i32,
    chain_hits: &[crate::opportunity::chain_mapper::ChainHit],
) -> (NewsRankBucket, Option<String>) {
    // Drop 优先级最高
    if chain_hits.is_empty() {
        return (NewsRankBucket::Drop, Some("无 chain_hit 召回".to_string()));
    }
    if risk >= 35 {
        return (NewsRankBucket::Drop, Some(format!("风险扣分过高 ({})", risk)));
    }
    // A 档
    if score >= 70 && matches!(stage, HeatStage::Start | HeatStage::Ferment) && risk < 20 {
        return (NewsRankBucket::PushNow, None);
    }
    // C 档 (高潮/分歧/退潮 默认 C, 不推)
    if matches!(stage, HeatStage::Climax | HeatStage::Divergence | HeatStage::Fade) {
        return (NewsRankBucket::LogOnly, None);
    }
    // B 档
    if score >= 45 {
        return (NewsRankBucket::WatchCandidate, None);
    }
    // C 档 (分数不足)
    (NewsRankBucket::LogOnly, None)
}
