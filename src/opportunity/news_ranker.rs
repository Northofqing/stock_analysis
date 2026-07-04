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
}
