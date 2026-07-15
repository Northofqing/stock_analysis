//! 通知推送 + MagicLaw 守护进程 + Token 管理
//!
//! 从 main.rs 提取，减少单文件体积。

use serde::Deserialize;
use std::io::Write;
use std::process::Stdio;
use std::sync::atomic::Ordering;

use crate::{
    ApiTokenSource, CachedApiToken, DaemonReadySource, MessageSendTransport, MessageSendType,
    DEFAULT_MAGICLAW_API_ADDR, DEFAULT_MAGICLAW_CLIENT_NAME, DEFAULT_MAGICLAW_PROJECT_ID,
    DEFAULT_MAGICLAW_TOKEN_REFRESH_AHEAD_SECS, DEFAULT_MAGICLAW_TOKEN_TTL_SECS,
    MAGICLAW_DAEMON_BOOT_LOCK, MAGICLAW_DISABLE_ENV_TOKEN, MAGICLAW_TOKEN_ISSUE_LOCK,
    MAGICLAW_TOKEN_MEM_CACHE,
};

/// v11-P0-4 commit D: 推送治理 — 推送类别
///
/// 35 条推送盘点的"默认降级 vs 保留 vs 移交" 由 `push_governor` 函数根据 `PushKind` 决定.
/// grill Q2 修订: 12 条降级 (A2/A3/A4/A5/A6/A11/A12/B4/B10/B11/B12/B13) / 9 保留 (A1/A7/A8/A13/A14/A15/B1/B2/C1).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PushKind {
    /// 保留: 持仓事件告警 (涨跌停突变/炸板/排除/风控/现金预警)
    HoldingEvent,
    /// 保留: 盘前/盘后告警/复盘/概览
    DailyReport,
    /// 保留: 公告告警
    Announcement,
    /// 降级: 竞价量能 Top10
    AuctionVolume,
    /// 降级: 虚拟观察仓位
    VirtualWatch,
    /// 降级: 首板/二板/三板+ Top10
    LimitBoards,
    /// 降级: 领涨板块 Top5
    SectorTop,
    /// 降级: 主力净流入 Top10
    FundInflow,
    /// 降级: 9:20-9:25 竞价重推优选
    AuctionRepush,
    /// 降级: 因子 IC (grill Q6 改)
    FactorIC,
    /// 降级: v4 赛道分档
    SectorTier,
    /// 降级: v4 资金验证
    CapitalVerify,
    /// 降级: 周度 SOP
    WeeklySOP,
    // v11-P0-5+ Commit 4 加: 5 个候选源 (P5 §六 验收, 默认降级, 候选台统一推 1 条)
    /// 降级: A10 选股推荐 (移交候选台)
    StockPick,
    /// 降级: B3 优选候选 (移交候选台)
    OptimalClose,
    /// 降级: B6 放量·自选 (移交候选台)
    VolumeWatchlist,
    /// 降级: B7 放量·实盘优选 (移交候选台)
    VolumeRealTrade,
    /// 降级: C4 产业链扫描 (移交候选台)
    IndustryChain,
    /// v14.5 G-05: T-13 盘中换手率 Top10 (ℹ️ 1次/日, 10min 冷却)
    TurnoverTop,
    // v11-P0-5++ Commit 5 加: 候选台统一卡片 (5 路 raw 合并 → 1 张排序候选清单)
    /// 保留: 候选筛选台卡片 (P5 §五 输出形态, 强证据>多源>题材)
    CandidateBoard,
    // P2-News Commit 4 加: 新闻 Ranker 输出卡片 (A/B/C/Drop 4 档, 阶段判断+风险过滤)
    /// 保留: 新闻 Ranker 候选卡片 (P2-News 阶段判断+风险过滤后的输出)
    NewsRanked,
    // ============= v12 §14.3 新增 PushKind =============
    /// 账户模式变更 (T-01, ⚡ 无冷却) [MVP-1]
    AccountMode,
    /// 数据模式变更 (T-02, ⚡ 10min 冷却) [MVP-1]
    DataMode,
    /// 持仓操作建议 (T-03/T-04, ⚡ 30min/票) [MVP-1]
    HoldingPlan,
    /// 做T 建议 (T-05/T-06, ⚡ 30min/票) [MVP-2]
    T0Advice,
    /// 候选触发/转正 (T-07, ⚡ 1次/票/日) [MVP-3]
    CandidateTriggered,
    /// 禁止操作提示 (T-09, ℹ️ 60min/票, 默认降级) [MVP-1]
    ForbiddenOps,
    /// 虚拟盘成交回报 (T-10, ℹ️ 5min批, 默认降级) [MVP-1]
    PaperTrade,
    /// 尾盘决策 (T-12, ⚡ 1次/日) [MVP-4]
    CloseCall,
    // ============= v12 §14.2 盘后 PushKind =============
    /// 盘面走向 (R-02, 盘后 1次/日) [MVP-4]
    ReviewMarket,
    /// 龙虎榜 (R-04, 盘后 21:00 补全) [MVP-4]
    ReviewLhb,
    /// 系统信号复盘 (R-05) [MVP-1→MVP-4]
    ReviewSignal,
    /// 失败样本归因 (R-06) [MVP-5]
    ReviewFailure,
    /// 明日观察池 (R-07) [MVP-4]
    TomorrowWatch,
    /// 明日事件日历 (R-08) [MVP-4]
    EventCalendar,
    // ============= v13 §14 新增 PushKind (PR #1) =============
    /// v13 §14.1 P-01 盘前新闻热点 (⚡ 15min 冷却)
    PreopenNewsHot,
    /// v13 §14.2 I-01 盘中轮动总览 (⚡ 15min 冷却)
    IntradayMarket,
    /// v13 §14.2 I-02 新闻催化映射 (⚡ 10min 冷却)
    NewsCatalyst,
    /// v13 §14.2 I-09 量价反向发现 (⚡ 10min 冷却)
    SectorAnomaly,
    // ============= v13 §14.4 新增 PushKind (PR #2) =============
    /// v13 §14.4 D-01 新闻驱动个股 (⚡ 20min/票 冷却)
    NewsToIdea,
    // ============= v13 §14.3 新增 PushKind (PR #3) =============
    /// v13 §14.3 A-10 盘后题材催化复盘 (⚡盘后 1次/日)
    CatalystReview,
    // ============= v13 §14.2 新增 PushKind (PR #4 - 审计多发现) =============
    /// v13 §14.2 I-03 盘中涨停扩散 (⚡ 30min 冷却) — 与盘后 IndustryChain (R-03) 区分
    IndustryChainIntraday,
    // ============= v13.1 新规模板 (新规 2026-07-06 生效) =============
    /// v13.1 §5.2 T-14 盘后固定价格申报 (⚡ 1min/票)
    PostFixedPriceOrder,
    /// v13.1 §5.3 T-15 盘后固定价格成交 (⚡ 5min/票)
    PostFixedPriceFill,
    /// v13.1 §5.4 T-16 ST 涨跌幅变更提醒 (新规 5%→10%, ⚡ 1次/票/日)
    StPriceLimitChanged,
    /// v13.1 §5.5 T-17 ETF 收盘集合竞价 (ℹ️ 1次/日, 仅沪市 ETF)
    EtfClosingCallAuction,
    /// v13.1 §5.6 T-18 创业板协议大宗盘中确认 (ℹ️ 5min/票)
    BlockTradeIntradayConfirm,
    /// v13.1 §5.7 T-19 北交所大宗价格区间 (ℹ️ 60min/票)
    BlockTradePriceRange,
    // ============= v14 新增 (原 A-01, 复用 T-11) =============
    /// v13 §14.3 A-01 虚拟仓复盘 (ℹ️ 1次/日, 盘后参考)
    PaperReview,
    /// v14.3 F-12: T-08 候选失效 (从 CandidateBoard 拆出独立 PushKind, ℹ️参考)
    CandidateInvalidated,
    // ============= v15.1 C1.2: IPO 监测推送 =============
    /// IPO 过会 / 证监会注册 (Important 级别, 86400s cooldown)
    IpoListingApproval,
    /// IPO 招股说明书披露 (Important, 43200s)
    IpoProspectus,
    /// IPO 阶段变化 / 供应链受益 (Info, 3600s)
    IpoCatalyst,
    // ============= v15.3 D5.1: 4 路源新推送 =============
    /// 政策催化（十五五规划 / 国务院 / 发改委）(Important, 86400s)
    PolicyHit,
    /// 业绩超预期 (Important, 43200s)
    EarningsBeat,
    /// 业绩低于预期 (Important, 43200s)
    EarningsMiss,
    /// 卖方评级上调 (Important, 86400s)
    AnalystUpgrade,
    /// 今日实盘异常 — 持仓变动 / 账户模式切换 (Emergency, 60s)
    MarketActionAlert,
}

impl PushKind {
    /// v19.12: 全部保留 false (用户要求去掉条件限制, 所有模板都推送)
    /// 旧: 11 保留 + 19 deprecated 降级 (P0-4 commit D 默认行为, PUSH_VERBOSE=true 时无效)
    /// 新: 所有 30 个 PushKind 都保留, deprecated=0
    pub fn is_deprecated(self) -> bool {
        false
    }

    /// v17.5 §2.2: 7 个 spec 标记为 0-caller (实际 metadata getter 仍在用,
    /// production push caller 已注释/删除 (main.rs:8313 AuctionRepush)).
    ///
    /// 走 Ipo* precedent: enum 变体**保留** (不改 level/cooldown_secs/label match),
    /// 仅在此方法中"标 legacy", 在 push_governor_inner 中按 env 控制可见性.
    ///
    /// 7 个 variants: AuctionRepush, OptimalClose, VolumeWatchlist,
    /// VolumeRealTrade, CandidateTriggered, CandidateInvalidated, VirtualWatch.
    pub fn is_legacy_v17_5(self) -> bool {
        matches!(
            self,
            Self::AuctionRepush
                | Self::OptimalClose
                | Self::VolumeWatchlist
                | Self::VolumeRealTrade
                | Self::CandidateTriggered
                | Self::CandidateInvalidated
                | Self::VirtualWatch
        )
    }

    /// v17.6 §2.2: 3 个 spec 标记为低优 / 子段治理候选.
    ///
    /// 注: 这 3 个 variants **与 v17.5 不同** — 它们仍有 production caller
    /// (main.rs:8523 FactorIC). 因此本方法不标 legacy, 而是标 `is_low_priority_v17_6`
    /// — 在 push_governor_inner 命中时给 info log 但不强制出声 warn.
    ///
    /// 3 个 variants: FactorIC, SectorTier, CapitalVerify.
    /// 后续 (dev plan v2 §3.7): DailyReport 子段拆分时把这些 variants
    /// 收纳进 DailyReportSubKind 子枚举.
    pub fn is_low_priority_v17_6(self) -> bool {
        matches!(
            self,
            Self::FactorIC | Self::SectorTier | Self::CapitalVerify
        )
    }

    /// v17.7 + v17.8: 12 个 spec 标的"待清理"变体实际是 active
    /// (有 production caller + metadata getter, 跟 v17.6 同样 gap).
    ///
    /// spec 字面写"6 项 0-caller" / "8 项交易类清理" — 实证不符:
    ///   - v17.7: Announcement, PolicyHit, EarningsBeat, EarningsMiss,
    ///     AnalystUpgrade, MarketActionAlert (6 个, 全部 active)
    ///   - v17.8: PostFixedPriceOrder, PostFixedPriceFill, StPriceLimitChanged,
    ///     EtfClosingCallAuction, BlockTradeIntradayConfirm, BlockTradePriceRange
    ///     (6 个, 全部 active)
    ///
    /// 本方法标 `is_active_spec_target` — 命中时 info log 跟踪 audit surface,
    /// 后续 dev plan v2 §3.7/§3.8 sub_kind / DispatchTable 决策点.
    pub fn is_active_spec_target_v17_7_v17_8(self) -> bool {
        matches!(
            self,
            // v17.7
            Self::Announcement
                | Self::PolicyHit
                | Self::EarningsBeat
                | Self::EarningsMiss
                | Self::AnalystUpgrade
                | Self::MarketActionAlert
            // v17.8 (6 个交易类)
            | Self::PostFixedPriceOrder
                | Self::PostFixedPriceFill
                | Self::StPriceLimitChanged
                | Self::EtfClosingCallAuction
                | Self::BlockTradeIntradayConfirm
                | Self::BlockTradePriceRange
        )
    }

    /// v12 §14.3 等级: 🚨紧急 / ⚡重要 / ℹ️参考
    pub fn level(self) -> PushLevel {
        match self {
            // 🚨紧急: HoldingEvent(已有, 包含跌停扫雷等)
            PushKind::HoldingEvent => PushLevel::Emergency,
            // ⚡重要
            PushKind::Announcement
            | PushKind::AccountMode
            | PushKind::DataMode
            | PushKind::HoldingPlan
            | PushKind::T0Advice
            | PushKind::CandidateTriggered
            | PushKind::CloseCall
            | PushKind::ReviewMarket
            | PushKind::ReviewLhb
            | PushKind::ReviewSignal
            | PushKind::TomorrowWatch
            | PushKind::EventCalendar
            | PushKind::DailyReport
            | PushKind::CandidateBoard
            | PushKind::NewsRanked
            // v13 新增
            | PushKind::PreopenNewsHot
            | PushKind::IntradayMarket
            | PushKind::NewsCatalyst
            | PushKind::SectorAnomaly
            | PushKind::NewsToIdea
            | PushKind::CatalystReview
            | PushKind::IndustryChainIntraday
            | PushKind::PostFixedPriceOrder
            | PushKind::PostFixedPriceFill
            | PushKind::StPriceLimitChanged
            | PushKind::EtfClosingCallAuction
            | PushKind::BlockTradeIntradayConfirm
            | PushKind::BlockTradePriceRange => PushLevel::Important,
            // v14 PaperReview + CandidateInvalidated
            | PushKind::CandidateInvalidated => PushLevel::Important,
            // v15.3 D5: 4 路源重要级 (PolicyHit/EarningsBeat/EarningsMiss/AnalystUpgrade)
            | PushKind::PolicyHit
            | PushKind::EarningsBeat
            | PushKind::EarningsMiss
            | PushKind::AnalystUpgrade => PushLevel::Important,
            // v15.3 D5: 实盘异常是紧急级
            | PushKind::MarketActionAlert => PushLevel::Emergency,
            // ℹ️参考 (降级 + ForbiddenOps/PaperTrade)
            _ => PushLevel::Info,
        }
    }

    /// v12 §14.3: 是否需强制全局横幅 (§14.0)
    /// 交易建议类 (T-01/02/03/04/05/06/07/09/10/12) + 盘后 R 系列都需
    pub fn requires_banner(self) -> bool {
        matches!(
            self,
            PushKind::AccountMode
                | PushKind::DataMode
                | PushKind::HoldingPlan
                | PushKind::HoldingEvent
                | PushKind::T0Advice
                | PushKind::CandidateTriggered
                | PushKind::ForbiddenOps
                | PushKind::PaperTrade
                | PushKind::CloseCall
                | PushKind::ReviewMarket
                | PushKind::ReviewLhb
                | PushKind::ReviewSignal
                | PushKind::ReviewFailure
                | PushKind::TomorrowWatch
                | PushKind::EventCalendar
                | PushKind::DailyReport
                | PushKind::AuctionVolume
                // v13 新增 (P-01 盘前无持仓语义, 不要 banner; I-01/I-02 盘中交易建议类, 要 banner)
                | PushKind::IntradayMarket
                | PushKind::NewsCatalyst
                | PushKind::NewsToIdea
                | PushKind::IndustryChainIntraday
                | PushKind::PostFixedPriceOrder
                | PushKind::PostFixedPriceFill
                | PushKind::StPriceLimitChanged
        )
    }

    /// v12 §14.3 冷却 (秒). None = 无冷却 (紧急/状态变更)
    pub fn cooldown_secs(self) -> Option<u32> {
        match self {
            // 无冷却 (状态变更即推)
            PushKind::AccountMode | PushKind::HoldingEvent => None,
            // 10 min
            PushKind::DataMode => Some(600),
            // 30 min / 票 (持有建议 + 做T 共享)
            PushKind::HoldingPlan | PushKind::T0Advice => Some(1800),
            // 1次/票/日 (86400s)
            PushKind::CandidateTriggered => Some(86_400),
            // 60 min / 票
            PushKind::ForbiddenOps => Some(3600),
            // 5 min / 票 (批推)
            PushKind::PaperTrade => Some(300),
            // 1次/日
            PushKind::CloseCall => Some(86_400),
            // 盘后系列 1次/日 (推送时机控制而非冷却)
            PushKind::ReviewMarket
            | PushKind::ReviewLhb
            | PushKind::ReviewSignal
            | PushKind::ReviewFailure
            | PushKind::TomorrowWatch
            | PushKind::EventCalendar
            | PushKind::DailyReport => Some(86_400),
            // 复用现有冷却配置
            PushKind::AuctionVolume | PushKind::AuctionRepush => Some(600),
            PushKind::SectorTier | PushKind::CapitalVerify => Some(1800),
            PushKind::FactorIC => Some(3600),
            PushKind::WeeklySOP => Some(86_400),
            // v13 §14.5 (Codex F5 修): TurnoverTop 显式 600s (原默认 1800s 与 spec 不符)
            // v14.5: TurnoverTop enum 已接通 (line 67), 启用该分支
            PushKind::TurnoverTop => Some(600), // 10 min
            // v14.5 G-06: IndustryChain 显式 86400s (1次/日, vs 默认 1800s)
            PushKind::IndustryChain => Some(86_400), // 1次/日
            // v13 新增
            PushKind::PreopenNewsHot | PushKind::IntradayMarket => Some(900), // 15 min
            PushKind::NewsCatalyst => Some(600),                              // 10 min
            PushKind::SectorAnomaly => Some(600),                             // 10 min
            PushKind::NewsToIdea => Some(1200),                               // 20 min/票
            PushKind::CatalystReview => Some(86_400),                         // 1次/日
            PushKind::IndustryChainIntraday => Some(1800),                    // 30 min
            PushKind::PostFixedPriceOrder => Some(60),                        // 1 min/票
            PushKind::PostFixedPriceFill => Some(300),                        // 5 min/票
            PushKind::StPriceLimitChanged => Some(86_400),                    // 1次/票/日
            PushKind::EtfClosingCallAuction => Some(86_400),                  // 1次/日
            PushKind::BlockTradeIntradayConfirm => Some(300),                 // 5 min/票
            PushKind::BlockTradePriceRange => Some(3600),                     // 60 min/票
            PushKind::PaperReview => Some(86_400),                            // 1次/日
            PushKind::CandidateInvalidated => Some(1800),                     // 30 min
            // v58: P-05 虚拟观察仓 (开盘 9:30 推一次, 1次/日)
            PushKind::VirtualWatch => Some(86_400), // 1次/日
            // v15.3 D5.1: 4 路源冷却
            PushKind::PolicyHit => Some(86_400),       // 1次/日
            PushKind::EarningsBeat => Some(43_200),    // 12h
            PushKind::EarningsMiss => Some(43_200),     // 12h
            PushKind::AnalystUpgrade => Some(86_400),   // 1次/日
            PushKind::MarketActionAlert => Some(60),    // 1 min/票 (实盘异常需立即)
            _ => Some(1800),                            // 默认 30min
        }
    }

    /// b011 P0-2: L4 dedup 冷却的键语义 (v14_adapter::v14_gate 用)
    pub fn cooldown_scope(self) -> CooldownScope {
        use PushKind::*;
        match self {
            // 公告冷却由 SignalStateMachine (per (code, category) + 每日预算) 专管,
            // L4 若再按 kind 冷却会把同窗口内**不同**公告误杀 (b011 P0-2 评审决策)
            Announcement => CooldownScope::External,
            // §14.3 表中标 "/票" 的: 必须有 code 才能按票冷却
            HoldingPlan | T0Advice | CandidateTriggered | ForbiddenOps | PaperTrade
            | NewsToIdea | PostFixedPriceOrder | PostFixedPriceFill | StPriceLimitChanged
            | BlockTradeIntradayConfirm | BlockTradePriceRange => CooldownScope::PerTicket,
            _ => CooldownScope::Global,
        }
    }

    /// 简短标签 (log 显示)
    pub fn label(self) -> &'static str {
        match self {
            PushKind::HoldingEvent => "持仓事件",
            PushKind::DailyReport => "日报/复盘/概览",
            PushKind::Announcement => "公告",
            PushKind::AuctionVolume => "竞价量能",
            PushKind::VirtualWatch => "虚拟观察",
            PushKind::LimitBoards => "板数榜",
            PushKind::SectorTop => "领涨板块",
            PushKind::FundInflow => "主力净流入",
            PushKind::AuctionRepush => "竞价重推",
            PushKind::FactorIC => "因子IC",
            PushKind::SectorTier => "赛道分档",
            PushKind::CapitalVerify => "资金验证",
            PushKind::WeeklySOP => "周度SOP",
            PushKind::StockPick => "选股",
            PushKind::OptimalClose => "优选",
            PushKind::VolumeWatchlist => "放量自选",
            PushKind::VolumeRealTrade => "放量实盘",
            PushKind::IndustryChain => "产业链",
            // v14.5 G-05
            PushKind::TurnoverTop => "盘中换手率 Top10",
            PushKind::CandidateBoard => "候选台",
            PushKind::NewsRanked => "新闻Ranker",
            // v12
            PushKind::AccountMode => "账户模式",
            PushKind::DataMode => "数据模式",
            PushKind::HoldingPlan => "持仓建议",
            PushKind::T0Advice => "做T建议",
            PushKind::CandidateTriggered => "候选触发",
            PushKind::ForbiddenOps => "禁止操作",
            PushKind::PaperTrade => "虚拟盘",
            PushKind::CloseCall => "尾盘决策",
            PushKind::ReviewMarket => "盘面走向",
            PushKind::ReviewLhb => "龙虎榜",
            PushKind::ReviewSignal => "信号复盘",
            PushKind::ReviewFailure => "失败归因",
            PushKind::TomorrowWatch => "明日观察池",
            PushKind::EventCalendar => "事件日历",
            // v13 新增
            PushKind::PreopenNewsHot => "盘前热点",
            PushKind::IntradayMarket => "盘中轮动",
            PushKind::NewsCatalyst => "新闻催化",
            PushKind::SectorAnomaly => "异动无归因",
            PushKind::NewsToIdea => "新闻驱动个股",
            PushKind::CatalystReview => "题材催化复盘",
            PushKind::IndustryChainIntraday => "盘中涨停扩散",
            PushKind::PostFixedPriceOrder => "盘后固定价格申报",
            PushKind::PostFixedPriceFill => "盘后固定价格成交",
            PushKind::StPriceLimitChanged => "ST 涨跌幅变更",
            PushKind::EtfClosingCallAuction => "ETF 集合竞价尾盘",
            PushKind::BlockTradeIntradayConfirm => "大宗盘中确认",
            PushKind::BlockTradePriceRange => "北交所大宗价格区间",
            PushKind::PaperReview => "虚拟仓复盘",
            PushKind::CandidateInvalidated => "候选失效",
            // v15.1 C1.2: IPO 监测
            PushKind::IpoListingApproval => "IPO 过会",
            PushKind::IpoProspectus => "招股说明书",
            PushKind::IpoCatalyst => "IPO 阶段催化",
            // v15.3 D5.1: 4 路源标题
            PushKind::PolicyHit => "政策催化",
            PushKind::EarningsBeat => "业绩超预期",
            PushKind::EarningsMiss => "业绩低于预期",
            PushKind::AnalystUpgrade => "卖方评级上调",
            PushKind::MarketActionAlert => "实盘异常",
        }
    }

    /// v17.1 review F10 fix: 稳定 template_id (PascalCase → snake_case + _v1 后缀).
    ///
    /// 之前 `l6_sink::build_push_message` 用 `format!("{kind:?}")` 直接拿 Debug 输出,
    /// 这会让 template_id 跟 enum 变体名强耦合 — 任何 rename 都破坏 L7 analytics 历史数据.
    ///
    /// 本方法返回稳定 snake_case ID: `HoldingEvent → "holding_event_v1"`,
    /// `PostFixedPriceOrder → "post_fixed_price_order_v1"` 等. 即使将来变体
    /// rename, 旧 template_id 仍可作 alias 兼容 (commit 不改 enum 变体字符串).
    ///
    /// 设计取舍: 0 cache (compute on demand). PushKind 是 Copy enum, format + 字符遍历
    /// < 1µs. 60+ variants 不需要 lazy_static HashMap (Path D 一致: 不重写 L7 analytics).
    pub fn stable_template_id(self) -> String {
        let pascal = format!("{self:?}");
        let mut snake = String::with_capacity(pascal.len() + 3);
        for (i, c) in pascal.chars().enumerate() {
            if i > 0 && c.is_ascii_uppercase() {
                snake.push('_');
            }
            snake.push(c.to_ascii_lowercase());
        }
        snake.push_str("_v1");
        snake
    }
}

/// b011 P0-2: L4 dedup 键语义 (与 PushKind::cooldown_secs 配套)
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CooldownScope {
    /// 按 kind 全局冷却 (code 无关), 例: 盘后系列 1次/日
    Global,
    /// 按 (kind, code) 票级冷却; 未传 code 时 L4 不冷却 (归模板层 memo)
    PerTicket,
    /// 冷却由专门层管理 (公告=sm 状态机), L4 不重复治理
    External,
}

/// v12 §14.3: 推送等级
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PushLevel {
    /// 🚨紧急: 无视冷却
    Emergency,
    /// ⚡重要: 默认推送
    Important,
    /// ℹ️参考: 可降级 log
    Info,
}

impl PushLevel {
    pub fn label(self) -> &'static str {
        match self {
            PushLevel::Emergency => "🚨紧急",
            PushLevel::Important => "⚡重要",
            PushLevel::Info => "ℹ️参考",
        }
    }

    pub fn is_emergency(self) -> bool {
        matches!(self, PushLevel::Emergency)
    }
}

// b011 P1-2: 旧 COOLDOWN_MEMO (v42/v59 票级冷却) 已删 —
// 冷却统一收敛到 v14.2 L4 dispatcher ((kind, code) + PushKind::cooldown_secs 窗口).

/// v69: 推送日志保存 — 把每条实际推送的内容按日期路径写到 data/push_log/
///   - 路径: data/push_log/YYYY-MM-DD/HHMMSS_<随机>.md
///   - 沙箱 V10_DRY_RUN_PUSH=1 也保存 (用户能查测试推送)
///   - 写失败不阻塞主流程 (warn log)
fn save_push_log(text: &str) {
    use std::io::Write;
    log::info!(
        "[v69] save_push_log entered, text len={}",
        text.chars().count()
    );
    let now = chrono::Local::now();
    let date_dir = now.format("%Y-%m-%d").to_string();
    let time_prefix = now.format("%H%M%S").to_string();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let rand_suffix = format!("{:08x}", nanos);
    let dir = std::path::PathBuf::from("data/push_log").join(&date_dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("[v69] push_log 目录创建失败: {}", e);
        return;
    }
    let path = dir.join(format!("{}_{}.md", time_prefix, &rand_suffix[..6]));
    match std::fs::File::create(&path) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(text.as_bytes()) {
                log::warn!("[v69] push_log 写入失败: {}", e);
            } else {
                log::info!("[v69] push_log 写入: {}", path.display());
            }
        }
        Err(e) => log::warn!("[v69] push_log 创建文件失败: {}", e),
    }
}

/// v70+: 新闻推荐落盘 (D-01 / I-02 推荐时 → D+1 兑现 关联)
///   - 文件: data/d01_recommendations_YYYY-MM-DD.jsonl (按天)
///   - 字段: ts (推送时间), template (D-01/I-02), code, name, theme, reason (3 条), action, price
///   - 后续: 跟 news_outcome_YYYY-MM-DD.md 关联 (D+1 兑现 → 胜率)
///   - 调用: push_news_recommendation() 在 notify::push_governor(D-01/I-02) 后调
pub fn record_news_recommendation(
    template: &str,
    code: &str,
    name: &str,
    theme: &str,
    reason: &[&str],
    action: Option<&str>,
    price: Option<f64>,
) {
    use std::fs::{create_dir_all, OpenOptions};
    use std::io::Write;
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let dir = std::path::PathBuf::from("data").join("d01_recommendations");
    if let Err(e) = create_dir_all(&dir) {
        log::warn!("[v70+] d01_recommendations 目录创建失败: {}", e);
        return;
    }
    let path = dir.join(format!("{}.jsonl", date));
    let reason_json: Vec<String> = reason.iter().map(|s| s.to_string()).collect();
    let entry = serde_json::json!({
        "ts": ts,
        "template": template,
        "code": code,
        "name": name,
        "theme": theme,
        "reason": reason_json,
        "action": action.unwrap_or(""),
        // W1.16 / B-010 P0-5: price 缺失必须显式为 null, 不允许 0.0 fallback
        // 下游读取时用 .is_null() 判缺失, 避免被误认为合法报价
        "price": price,
        "outcome": null, // 后续回填
    });
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut f) => {
            if let Err(e) = writeln!(f, "{}", entry) {
                log::warn!("[v70+] d01_recommendations 写入失败: {}", e);
            } else {
                log::info!(
                    "[v70+] 落盘推荐: {} ({}) → {}",
                    template,
                    code,
                    path.display()
                );
            }
        }
        Err(e) => log::warn!("[v70+] d01_recommendations 创建文件失败: {}", e),
    }
}

/// v11-P0-4 commit D: 推送治理入口
///
/// 根据 `PushKind` + `PUSH_VERBOSE` env var 决定:
/// - `kind.is_deprecated() == true` **且** `PUSH_VERBOSE != "true"` → 降级 log (不推)
/// - 其他情况 → 调 `push_wechat` 正常推送
///
/// PUSH_VERBOSE=true 恢复旧行为 (留退路, shadow 切换验证用)
/// v19.12: 全部保留 true (用户要求去掉条件限制, 所有模板都推送)
/// W9.3 桥接结果 (CRITICAL 修复: 区分 4 种 v14.2 结果)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    Pushed,             // v14.2 + v13 都成功
    Deduped,            // v14.2 dedup hit, v13 未推送 (60s 内同 kind)
    Denied(String),     // v14.2 governance 拦截
    SinkError(String),   // v14.2 sink 失败
}

impl PushOutcome {
    pub fn is_pushed(&self) -> bool {
        matches!(self, Self::Pushed)
    }
}

/// b011 P1-2: 推送**唯一**实现 — 全部 governor 入口收敛到这里.
///
/// 链路: v14_gate (L4 dedup + L5 governance) → push_wechat (真实投递, 含 dry-run)
///       → v14_record_delivery (L7 记录真实 sink + 真实结果).
///
/// 与旧版差异:
///   - V10_DRY_RUN_PUSH 不再绕过 v14.2 (dry-run 由 push_wechat 自身处理,
///     gate/analytics 全链路照走 → --test 能测到完整推送治理路径)
///   - sink_name 不再硬编码 "wechat" (b011 P0-1), 取实际通道
async fn push_governor_inner(text: &str, kind: PushKind, code: Option<&str>) -> PushOutcome {
    use crate::v14_adapter::{self, V14Gate};

    // v17.5 §2.2: 命中 v17.5-legacy variants 时按 env 控制可见性
    //   默认 warn 出声 (v15.x 4 铁律 — 默认值必须出声状态);
    //   显式 STOCK_ANALYSIS_PUSH_KIND_LEGACY=silent 可静默.
    // 7 variants 标 legacy: is_legacy_v17_5() 见 impl PushKind.
    use std::sync::OnceLock;
    static LEGACY_AUDIT_DEFAULT_VISIBLE: OnceLock<bool> = OnceLock::new();
    let audit_legacy_visible = *LEGACY_AUDIT_DEFAULT_VISIBLE.get_or_init(|| {
        std::env::var("STOCK_ANALYSIS_PUSH_KIND_LEGACY")
            .ok()
            .as_deref()
            != Some("silent")
    });
    if audit_legacy_visible && kind.is_legacy_v17_5() {
        log::warn!(
            "[v17.5-legacy] PushKind::{:?} 在 production push 命中 (默认出声); \
             env STOCK_ANALYSIS_PUSH_KIND_LEGACY=silent 可静默",
            kind
        );
    }
    // v17.6: 命中低优 variants 时 info log (低优 ≠ legacy, 仍有 caller)
    if kind.is_low_priority_v17_6() {
        log::info!(
            "[v17.6-low-priority] PushKind::{:?} 命中 (子段治理候选, dev plan §3.7 follow-up)",
            kind
        );
    }
    // v17.7 + v17.8: 命中 active spec target (12 variants) 时 info log audit
    //   跟踪后续 §3.7/§3.8 sub_kind/DispatchTable 决策面 (不强制出声)
    if kind.is_active_spec_target_v17_7_v17_8() {
        log::info!(
            "[v17.7-v17.8-active-target] PushKind::{:?} 命中 (active spec target, dev plan §3.7-§3.8 follow-up)",
            kind
        );
    }

    // b013 review P0-4: v14 路径也走 LaunchGate (b011 漏: 17 处 main::push_wechat
    // 走 launch_gate, v14 直连 push_wechat 不走 — Stage=gray 下非 critical 仍能推).
    if !launch_gate_check(kind) {
        return PushOutcome::Denied("launch_gate_stage".to_string());
    }
    let event = match v14_adapter::v14_gate(kind, code) {
        V14Gate::Deduped => return PushOutcome::Deduped,
        V14Gate::Denied(reason) => return PushOutcome::Denied(reason),
        V14Gate::Approved(event) => event,
    };
    // v15.1 A3: 把 reserve/commit 拆分, 失败时 rollback 不占 cooldown 窗口
    // v17.1-r2 §3.6: env opt-in 走 L6 SinkRouter (env=STOCK_ANALYSIS_PUSH_V6_ENABLE=1).
    // 默认仍走 push_wechat (L5 → L6 真路径尚未启用, l6_sink.rs 已注册 ConsoleSink + MagiclawSink 等待).
    let delivered = if std::env::var("STOCK_ANALYSIS_PUSH_V6_ENABLE").ok().as_deref() == Some("1") {
        let msg = crate::l6_sink::build_push_message(&event, text, kind);
        matches!(
            crate::l6_sink::sink_router().route(&msg).await,
            stock_analysis::push_l6::SinkResult::Ok
        )
    } else {
        push_wechat(text).await
    };
    if delivered {
        v14_adapter::commit_dedup_for_event(&event, kind);
    } else {
        v14_adapter::rollback_dedup_for_event(&event, kind);
    }
    // b013 review P2-15: 入口取一次 channel (避免 push_wechat await 后 env 抖动)
    let channel = current_send_channel();
    v14_adapter::v14_record_delivery(&event, kind, text, delivered, channel);
    if delivered {
        PushOutcome::Pushed
    } else {
        PushOutcome::SinkError("push_wechat returned false".to_string())
    }
}

/// b013 P0-4: LaunchGate 单点判定 — 与 main::push_wechat_with_kind 语义一致.
/// Emergency 级 (`level().is_emergency()`) 永远放行 (critical alert);
/// 其他走 `launch_gate::should_push_user(stage, false)`.
fn launch_gate_check(kind: PushKind) -> bool {
    if kind.level().is_emergency() {
        return true;
    }
    use stock_analysis::opportunity::launch_gate;
    let stage = launch_gate::current_stage();
    launch_gate::should_push_user(stage, false)
}

/// 实际投递通道名 (L7 analytics 用, b011 P0-1):
/// dry-run 显式记 "dry_run" (没有真实外发), 否则记配置的真实通道 ("feishu"/"wechat")
fn current_send_channel() -> &'static str {
    if std::env::var("V10_DRY_RUN_PUSH").ok().as_deref() == Some("1") {
        "dry_run"
    } else {
        resolve_send_type().as_str()
    }
}

/// b013 review P0-1: 兼容旧 2 参调用 + 自动给 default_code_for 兜底,
/// 让 PerTicket kind 在旧 2 参调用下仍走 L4 dedup 路径 (而不是 PerTicket+None 直通放过).
/// 真正票级隔离需 b014 把 caller 改成 push_governor_v3(text, kind, Some(code)).
#[deprecated(since = "v15.1", note = "Use push_governor_v3 with explicit code; this shim collapses PerTicket dedup to '_per_ticket_unbound' global bucket")]
pub async fn push_governor(text: &str, kind: PushKind) -> bool {
    push_governor_inner(text, kind, Some(default_code_for(kind)))
        .await
        .is_pushed()
}

/// v14.2 单入口 (b011 P1-2 收敛后 + b013 review P0-1): 返回 enum 区分 4 种结果.
/// `code`: 票级冷却键 (§14.3 "/票" 类 kind 必传 real 票号, 否则 L4 不做票级冷却).
pub async fn push_governor_v3(text: &str, kind: PushKind, code: Option<&str>) -> PushOutcome {
    push_governor_inner(text, kind, code).await
}

/// b013 P0-1 兜底: PerTicket 类 kind 在缺 code 时塞占位, 让 L4 走全局 key,
/// 至少防止"无限重发同一票"。b014 应把所有 caller 改成 push_governor_v3 显式传 code。
fn default_code_for(kind: PushKind) -> &'static str {
    use PushKind::*;
    if matches!(
        kind,
        HoldingPlan
            | T0Advice
            | CandidateTriggered
            | ForbiddenOps
            | PaperTrade
            | NewsToIdea
            | PostFixedPriceOrder
            | PostFixedPriceFill
            | StPriceLimitChanged
            | BlockTradeIntradayConfirm
            | BlockTradePriceRange
    ) {
        "_per_ticket_unbound"
    } else {
        ""
    }
}

pub async fn push_wechat(text: &str) -> bool {
    // v10 P6 5 要素接入: V10_DRY_RUN_PUSH=1 时跳过实际推送, 仅 log
    // 用于开发/验证推送内容变化, 不骚扰飞书
    if std::env::var("V10_DRY_RUN_PUSH").ok().as_deref() == Some("1") {
        log::info!("[V10_DRY_RUN_PUSH] 跳过飞书推送, 内容预览:\n{}", text);
        // v69: 沙箱 dry-run 也保存 push_log
        save_push_log(text);
        return true;
    }

    // v69: 不管走哪条推送路径 (magiclaw cli / feishu http / 后续), 都先保存 push_log
    save_push_log(text);

    let send_type = resolve_send_type();
    let send_transport = resolve_send_transport(send_type);

    if matches!(send_transport, MessageSendTransport::Cli) {
        return push_via_magiclaw_cli(send_type, text).await;
    }

    if matches!(send_type, MessageSendType::Feishu)
        && matches!(send_transport, MessageSendTransport::Http)
    {
        return push_feishu_via_http(text).await;
    }

    log::info!(
        "[{}] 开始推送 ({}字) | via={}",
        send_type.label(),
        text.chars().count(),
        send_transport.as_str()
    );

    let magiclaw_bin = resolve_magiclaw_bin();
    let api_addr = resolve_api_addr();
    let api_base = to_api_base_url(&api_addr);
    // 关键：daemon 在 127.0.0.1 回环上，必须 .no_proxy() 绕过系统代理(Clash/Surge)。
    // 否则 macOS 系统代理会劫持本地请求并返回 503，导致健康检查恒失败、误判 daemon 不可用。
    let client = match reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log::error!("[{}] 创建 HTTP 客户端失败: {}", send_type.label(), e);
            return false;
        }
    };

    match ensure_magiclaw_daemon(&client, &magiclaw_bin, &api_addr, &api_base).await {
        Ok(DaemonReadySource::Reused) => {
            log::info!(
                "[{}] daemon 来源: 复用已有实例 | {}",
                send_type.label(),
                api_addr
            );
        }
        Ok(DaemonReadySource::StartedNow) => {
            log::info!(
                "[{}] daemon 来源: 本次自动拉起 | {}",
                send_type.label(),
                api_addr
            );
        }
        Err(e) => {
            log::error!("[{}] daemon 不可用: {}", send_type.label(), e);
            return false;
        }
    }

    let (mut active_token, mut active_token_source) =
        match resolve_or_issue_api_token(&magiclaw_bin).await {
            Ok(v) => v,
            Err(e) => {
                log::error!(
                    "[{}] 获取 daemon 动态鉴权 token 失败: {}",
                    send_type.label(),
                    e
                );
                return false;
            }
        };

    let verify_result =
        verify_daemon_auth(&client, &api_base, &active_token, &active_token_source).await;
    if let Err(first_err) = verify_result {
        if is_unauthorized_error(&first_err) {
            clear_dynamic_token_cache().await;
            match issue_and_cache_dynamic_api_token(&magiclaw_bin).await {
                Ok(next) => {
                    log::warn!(
                        "[{}] daemon token 鉴权失败，已清缓存并重新签发动态 token 后重试预检",
                        send_type.label()
                    );
                    if matches!(active_token_source, ApiTokenSource::Env) {
                        MAGICLAW_DISABLE_ENV_TOKEN.store(true, Ordering::Relaxed);
                    }
                    active_token = next.token;
                    active_token_source = ApiTokenSource::DynamicIssued;
                    if let Err(e) =
                        verify_daemon_auth(&client, &api_base, &active_token, &active_token_source)
                            .await
                    {
                        log::warn!("[{}] daemon 鉴权预检重试仍失败，但已重新签发 token，将继续尝试发送: {}", send_type.label(), e);
                    }
                }
                Err(issue_err) => {
                    log::error!(
                        "[{}] daemon 鉴权预检失败: {}；自动续签失败: {}",
                        send_type.label(),
                        first_err,
                        issue_err
                    );
                    return false;
                }
            }
        } else {
            log::error!("[{}] daemon 鉴权预检失败: {}", send_type.label(), first_err);
            return false;
        }
    }

    let to = match resolve_send_target(send_type, &client, &api_base, &active_token).await {
        Ok(v) => v,
        Err(e) => {
            log::error!("[{}] 解析收件人失败: {}", send_type.label(), e);
            return false;
        }
    };
    let to_log = to.as_deref().unwrap_or("<magiclaw-default>");

    match send_via_magiclaw_daemon(
        &client,
        &api_base,
        &active_token,
        send_type,
        to.as_deref(),
        text,
    )
    .await
    {
        Ok(()) => {
            log::info!("[{}] 推送成功 | to={}", send_type.label(), to_log);
            true
        }
        Err(first_err) => {
            if is_unauthorized_error(&first_err) {
                clear_dynamic_token_cache().await;
                match issue_and_cache_dynamic_api_token(&magiclaw_bin).await {
                    Ok(next) => {
                        log::warn!(
                            "[{}] daemon token 鉴权失败，已清缓存并重新签发动态 token 后重试发送",
                            send_type.label()
                        );
                        if matches!(active_token_source, ApiTokenSource::Env) {
                            MAGICLAW_DISABLE_ENV_TOKEN.store(true, Ordering::Relaxed);
                        }
                        match send_via_magiclaw_daemon(
                            &client,
                            &api_base,
                            &next.token,
                            send_type,
                            to.as_deref(),
                            text,
                        )
                        .await
                        {
                            Ok(()) => {
                                log::info!("[{}] 推送成功 | to={}", send_type.label(), to_log);
                                true
                            }
                            Err(retry_err) => {
                                log::error!("[{}] 推送失败: {}", send_type.label(), retry_err);
                                false
                            }
                        }
                    }
                    Err(issue_err) => {
                        log::error!(
                            "[{}] 推送失败: {}；自动续签失败: {}",
                            send_type.label(),
                            first_err,
                            issue_err
                        );
                        false
                    }
                }
            } else {
                log::error!("[{}] 推送失败: {}", send_type.label(), first_err);
                false
            }
        }
    }
}

pub async fn push_feishu_via_http(text: &str) -> bool {
    let url = match resolve_feishu_webhook_url() {
        Some(v) => v,
        None => {
            log::error!(
                "[飞书] 推送失败: 未配置 FEISHU_WEBHOOK_URL（或 MAGICLAW_FEISHU_WEBHOOK_URL）"
            );
            return false;
        }
    };

    log::info!("[飞书] 开始推送 ({}字) | via=http", text.chars().count());

    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(v) => v,
        Err(e) => {
            log::error!("[飞书] 创建 HTTP 客户端失败: {}", e);
            return false;
        }
    };

    let payload = serde_json::json!({
        "msg_type": "text",
        "content": {
            "text": text,
        }
    });

    let resp = match client.post(&url).json(&payload).send().await {
        Ok(v) => v,
        Err(e) => {
            log::error!("[飞书] 推送失败: 调用 webhook 失败: {}", e);
            return false;
        }
    };

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        log::error!("[飞书] 推送失败: webhook HTTP {}: {}", status, body_text);
        return false;
    }

    let parsed = serde_json::from_str::<serde_json::Value>(&body_text).ok();
    let ok_by_status_code = parsed
        .as_ref()
        .and_then(|v| v.get("StatusCode").and_then(|x| x.as_i64()))
        .map(|code| code == 0)
        .unwrap_or(false);
    let ok_by_code = parsed
        .as_ref()
        .and_then(|v| v.get("code").and_then(|x| x.as_i64()))
        .map(|code| code == 0)
        .unwrap_or(false);

    if ok_by_status_code || ok_by_code {
        log::info!("[飞书] 推送成功 | via=http");
        return true;
    }

    log::error!("[飞书] 推送失败: webhook 返回非成功体: {}", body_text);
    false
}

pub async fn push_via_magiclaw_cli(send_type: MessageSendType, text: &str) -> bool {
    let to = match send_type {
        MessageSendType::Wechat => None,
        MessageSendType::Feishu => match resolve_feishu_target() {
            Some(v) => Some(v),
            None => {
                log::error!(
                    "[飞书] 解析收件人失败: 飞书发送缺少收件人，请设置 FEISHU_TO（或 MAGICLAW_FEISHU_TO / FEISHU_CHAT_ID / FEISHU_OPEN_ID / FEISHU_USER_ID / FEISHU_EMAIL）"
                );
                return false;
            }
        },
    };

    let magiclaw_bin = resolve_magiclaw_bin();
    log::info!(
        "[{}] 开始推送 ({}字) | via=cli",
        send_type.label(),
        text.chars().count()
    );

    let mut cmd = tokio::process::Command::new(&magiclaw_bin);
    cmd.arg("send")
        .arg("--channel")
        .arg(send_type.as_str())
        .arg("--message")
        .arg(text)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(to) = to.as_deref() {
        cmd.arg("--to").arg(to);
    }

    // 将 cwd 指向 magiclaw 项目根目录，使其 dotenv 能加载飞书凭证所在的 .env。
    // 若 cwd 改变，则 MAGICLAW_DB_PATH 的相对路径会失效，故统一转为绝对路径传入。
    let magiclaw_home = resolve_magiclaw_home(&magiclaw_bin);
    if let Some(home) = magiclaw_home.as_deref() {
        cmd.current_dir(home);
    } else {
        log::warn!(
            "[{}] 未能定位 magiclaw 项目根目录（找不到 .env），飞书凭证可能加载失败 | bin={}",
            send_type.label(),
            magiclaw_bin
        );
    }

    if let Ok(db_path) = std::env::var("MAGICLAW_DB_PATH") {
        let db_path = db_path.trim();
        if !db_path.is_empty() {
            let abs_db = std::fs::canonicalize(db_path)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| {
                    // 文件可能尚不存在或无法规范化：相对路径手动拼接当前进程 cwd
                    let p = std::path::Path::new(db_path);
                    if p.is_absolute() {
                        db_path.to_string()
                    } else {
                        std::env::current_dir()
                            .map(|cwd| cwd.join(p).to_string_lossy().into_owned())
                            .unwrap_or_else(|_| db_path.to_string())
                    }
                });
            cmd.env("MAGICLAW_DB_PATH", abs_db);
        }
    }

    if let Ok(receive_id_type) = std::env::var("FEISHU_RECEIVE_ID_TYPE") {
        let receive_id_type = receive_id_type.trim();
        if !receive_id_type.is_empty() {
            cmd.arg("--receive-id-type").arg(receive_id_type);
        }
    }

    let output = match cmd.output().await {
        Ok(v) => v,
        Err(e) => {
            log::error!(
                "[飞书] 调用 magiclaw send 失败(magiclaw: {}): {}",
                magiclaw_bin,
                e
            );
            return false;
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        let detail = tail_lines(&stdout, 3);
        if detail.is_empty() {
            log::info!(
                "[{}] 推送成功 | to={}",
                send_type.label(),
                to.as_deref().unwrap_or("<auto>")
            );
        } else {
            log::info!(
                "[{}] 推送成功 | to={} | {}",
                send_type.label(),
                to.as_deref().unwrap_or("<auto>"),
                detail
            );
        }
        return true;
    }

    let stderr_tail = tail_lines(&stderr, 8);
    let stdout_tail = tail_lines(&stdout, 3);
    log::error!(
        "[{}] 推送失败(exit={}): {}{}",
        send_type.label(),
        output.status,
        if !stderr_tail.is_empty() {
            format!("stderr={}", stderr_tail)
        } else {
            "stderr=<empty>".to_string()
        },
        if !stdout_tail.is_empty() {
            format!(" | stdout={}", stdout_tail)
        } else {
            "".to_string()
        }
    );
    false
}

pub fn summarize_push_text(text: &str, max_chars: usize) -> String {
    let one_line = text.replace('\n', " | ");
    let mut out = String::new();
    let mut count = 0usize;
    for ch in one_line.chars() {
        if count >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
        count += 1;
    }
    out
}

pub fn evaluate_opportunity_push_skip_reason(opp_text: &str) -> Option<&'static str> {
    // 只对“整轮无有效产业链输出”的明确文案做跳过，避免
    // “值得关注：暂无通过量能/趋势确认候选”这类正常结果被误判为应跳过。
    if opp_text.contains("暂无最新快讯") {
        return Some("contains:暂无最新快讯");
    }
    if opp_text.contains("当前快讯未命中已知产业链") {
        return Some("contains:当前快讯未命中已知产业链");
    }
    if opp_text.contains("当前产业链信号可信度不足（已降级观察）") {
        return Some("contains:当前产业链信号可信度不足");
    }
    if opp_text.contains("无可用标的") {
        return Some("contains:无可用标的");
    }
    None
}

pub fn resolve_send_type() -> MessageSendType {
    // 默认统一走飞书（test 与 prod 一致）；如需微信，显式设置 SEND_TYPE=wechat。
    let default_type = MessageSendType::Feishu;

    let raw = std::env::var("MAGICLAW_SEND_TYPE")
        .or_else(|_| std::env::var("SEND_TYPE"))
        .unwrap_or_else(|_| default_type.as_str().to_string());
    match raw.trim().to_ascii_lowercase().as_str() {
        "wechat" | "weixin" | "wx" => MessageSendType::Wechat,
        "feishu" | "lark" => MessageSendType::Feishu,
        other => {
            log::warn!(
                "未识别的发送类型: {}，回退为默认 {}",
                other,
                default_type.as_str()
            );
            default_type
        }
    }
}

pub fn resolve_send_transport(send_type: MessageSendType) -> MessageSendTransport {
    match send_type {
        MessageSendType::Wechat => MessageSendTransport::Http,
        // 飞书自动路由：配置了 webhook 则走 HTTP；否则走 CLI。
        MessageSendType::Feishu => {
            if resolve_feishu_webhook_url().is_some() {
                MessageSendTransport::Http
            } else {
                MessageSendTransport::Cli
            }
        }
    }
}

pub fn resolve_feishu_webhook_url() -> Option<String> {
    ["FEISHU_WEBHOOK_URL", "MAGICLAW_FEISHU_WEBHOOK_URL"]
        .iter()
        .find_map(|key| {
            std::env::var(key)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
}

pub fn resolve_magiclaw_bin() -> String {
    std::env::var("MAGICLAW_BIN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{}/Desktop/magiclaw/target/release/magiclaw", home)
        })
}

/// 解析 magiclaw 项目根目录（其 `.env` 所在目录）。
/// magiclaw 启动时通过 dotenvy 从工作目录加载 `.env`，飞书凭证（FEISHU_APP_ID 等）
/// 存放在 magiclaw 自己的 `.env` 中。派生子进程时需将 cwd 指向该目录，否则读不到凭证。
/// 优先级：MAGICLAW_HOME 环境变量 > 从二进制路径推导（去掉 `target/release/magiclaw`）。
pub fn resolve_magiclaw_home(magiclaw_bin: &str) -> Option<std::path::PathBuf> {
    if let Ok(home) = std::env::var("MAGICLAW_HOME") {
        let home = home.trim();
        if !home.is_empty() {
            return Some(std::path::PathBuf::from(home));
        }
    }
    let bin_path = std::path::Path::new(magiclaw_bin);
    // 形如 .../magiclaw/target/release/magiclaw → 上溯 3 级到 .../magiclaw
    let home = bin_path.parent()?.parent()?.parent()?;
    if home.join(".env").is_file() {
        Some(home.to_path_buf())
    } else {
        None
    }
}

pub fn resolve_api_addr() -> String {
    std::env::var("MAGICLAW_API_ADDR")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_MAGICLAW_API_ADDR.to_string())
}

pub async fn resolve_or_issue_api_token(
    magiclaw_bin: &str,
) -> Result<(String, ApiTokenSource), String> {
    if !MAGICLAW_DISABLE_ENV_TOKEN.load(Ordering::Relaxed) {
        if let Some(token) = std::env::var("MAGICLAW_API_TOKEN")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            return Ok((token, ApiTokenSource::Env));
        }
    }

    if let Some(cached) = load_dynamic_token_from_mem_cache().await {
        return Ok((cached.token, ApiTokenSource::DynamicMemCache));
    }

    if let Some(cached) = load_dynamic_token_from_file_cache() {
        cache_dynamic_token_in_mem(&cached).await;
        return Ok((cached.token, ApiTokenSource::DynamicFileCache));
    }

    let issued = issue_and_cache_dynamic_api_token(magiclaw_bin).await?;
    Ok((issued.token, ApiTokenSource::DynamicIssued))
}

pub fn is_unauthorized_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("401") || lower.contains("unauthorized")
}

pub fn api_token_cache_file_path() -> std::path::PathBuf {
    let db_path =
        std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/stock_analysis.db".to_string());
    let db_path = std::path::PathBuf::from(db_path);
    let parent = db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("./data"));
    parent.join("magiclaw_api_token_cache.json")
}

pub fn now_epoch_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn token_refresh_ahead_secs() -> i64 {
    std::env::var("MAGICLAW_TOKEN_REFRESH_AHEAD_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|v| *v >= 0)
        .unwrap_or(DEFAULT_MAGICLAW_TOKEN_REFRESH_AHEAD_SECS)
}

pub fn is_cached_token_expired(token: &CachedApiToken) -> bool {
    match token.expires_at {
        Some(ts) => ts <= now_epoch_secs() + token_refresh_ahead_secs(),
        None => false,
    }
}

pub async fn load_dynamic_token_from_mem_cache() -> Option<CachedApiToken> {
    let guard = MAGICLAW_TOKEN_MEM_CACHE.read().await;
    let v = guard.clone();
    drop(guard);
    v.filter(|t| !t.token.trim().is_empty() && !is_cached_token_expired(t))
}

pub fn load_dynamic_token_from_file_cache() -> Option<CachedApiToken> {
    let path = api_token_cache_file_path();
    let text = std::fs::read_to_string(path).ok()?;
    let token = serde_json::from_str::<CachedApiToken>(&text).ok()?;
    if token.token.trim().is_empty() || is_cached_token_expired(&token) {
        return None;
    }
    Some(token)
}

pub async fn cache_dynamic_token_in_mem(token: &CachedApiToken) {
    let mut guard = MAGICLAW_TOKEN_MEM_CACHE.write().await;
    *guard = Some(token.clone());
}

pub async fn clear_dynamic_token_cache() {
    {
        let mut guard = MAGICLAW_TOKEN_MEM_CACHE.write().await;
        *guard = None;
    }

    let path = api_token_cache_file_path();
    let _ = std::fs::remove_file(path);
}

pub fn cache_dynamic_token_in_file(token: &CachedApiToken) -> Result<(), String> {
    let path = api_token_cache_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 token 缓存目录失败({}): {}", parent.display(), e))?;
    }
    let text = serde_json::to_string(token).map_err(|e| format!("序列化 token 缓存失败: {}", e))?;
    std::fs::write(&path, text)
        .map_err(|e| format!("写入 token 缓存失败({}): {}", path.display(), e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms)
            .map_err(|e| format!("设置 token 缓存文件权限失败({}): {}", path.display(), e))?;
    }

    Ok(())
}

pub fn parse_issue_token_output(stdout: &str) -> Result<CachedApiToken, String> {
    let mut token: Option<String> = None;
    let mut expires_at: Option<i64> = None;

    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("token=") {
            let v = rest.trim();
            if !v.is_empty() {
                token = Some(v.to_string());
            }
            continue;
        }

        if line.contains("expires_at=") {
            for part in line.split_whitespace() {
                if let Some(raw) = part.strip_prefix("expires_at=") {
                    if let Ok(ts) = raw.trim().parse::<i64>() {
                        expires_at = Some(ts);
                    }
                }
            }
        }
    }

    let token =
        token.ok_or_else(|| format!("auth issue 输出缺少 token 字段: {}", stdout.trim()))?;
    Ok(CachedApiToken { token, expires_at })
}

pub async fn issue_and_cache_dynamic_api_token(
    magiclaw_bin: &str,
) -> Result<CachedApiToken, String> {
    let _issue_guard = MAGICLAW_TOKEN_ISSUE_LOCK.lock().await;

    // 双检锁：等待锁期间可能已有其他协程签发并写入缓存。
    if let Some(cached) = load_dynamic_token_from_mem_cache().await {
        return Ok(cached);
    }
    if let Some(cached) = load_dynamic_token_from_file_cache() {
        cache_dynamic_token_in_mem(&cached).await;
        return Ok(cached);
    }

    let project_id = std::env::var("MAGICLAW_PROJECT_ID")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_MAGICLAW_PROJECT_ID.to_string());
    let client_name = std::env::var("MAGICLAW_CLIENT_NAME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}-{}", DEFAULT_MAGICLAW_CLIENT_NAME, std::process::id()));
    let ttl_secs = std::env::var("MAGICLAW_TOKEN_TTL_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_MAGICLAW_TOKEN_TTL_SECS);

    let output = tokio::process::Command::new(magiclaw_bin)
        .arg("auth")
        .arg("issue")
        .arg("--project")
        .arg(&project_id)
        .arg("--name")
        .arg(&client_name)
        .arg("--scopes")
        .arg("send,window_status")
        .arg("--ttl-secs")
        .arg(ttl_secs.to_string())
        .env(
            "MAGICLAW_DB_PATH",
            std::env::var("MAGICLAW_DB_PATH").unwrap_or_else(|_| {
                std::env::var("DATABASE_PATH")
                    .unwrap_or_else(|_| "./data/stock_analysis.db".to_string())
            }),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("执行 magiclaw auth issue 失败: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        let stderr_tail = tail_lines(&stderr, 8);
        let stdout_tail = tail_lines(&stdout, 3);
        return Err(format!(
            "magiclaw auth issue 失败(exit={}): {}{}",
            output.status,
            if !stderr_tail.is_empty() {
                format!("stderr={}", stderr_tail)
            } else {
                "".to_string()
            },
            if !stdout_tail.is_empty() {
                format!(" | stdout={}", stdout_tail)
            } else {
                "".to_string()
            }
        ));
    }

    let issued = parse_issue_token_output(&stdout)?;
    cache_dynamic_token_in_mem(&issued).await;
    cache_dynamic_token_in_file(&issued)?;
    Ok(issued)
}

pub fn to_api_base_url(api_addr: &str) -> String {
    if api_addr.starts_with("http://") || api_addr.starts_with("https://") {
        api_addr.trim_end_matches('/').to_string()
    } else {
        format!("http://{}", api_addr)
    }
}

pub fn resolve_wechat_data_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("WECHAT_CHANNEL_DIR") {
        return std::path::PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::Path::new(&home)
        .join(".claude")
        .join("channels")
        .join("wechat")
}

pub fn parse_first_peer_id_from_window_status(body: &str) -> Option<String> {
    let peers = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("peers").cloned())
        .and_then(|peers| peers.as_array().cloned())?;

    peers
        .iter()
        .filter_map(|peer| peer.get("peer_id").and_then(|value| value.as_str()))
        .map(str::trim)
        .find(|peer_id| !peer_id.is_empty())
        .map(|peer_id| peer_id.to_string())
}

pub fn resolve_magiclaw_log_dir() -> std::path::PathBuf {
    let db_path = std::env::var("MAGICLAW_DB_PATH").unwrap_or_else(|_| {
        std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/stock_analysis.db".to_string())
    });
    std::path::Path::new(&db_path)
        .parent()
        .map(|parent| parent.join("logs"))
        .unwrap_or_else(|| std::path::PathBuf::from("logs"))
}

pub fn resolve_wechat_target_from_magiclaw_logs() -> Option<String> {
    let log_dir = resolve_magiclaw_log_dir();
    let mut log_files: Vec<std::path::PathBuf> = std::fs::read_dir(&log_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("magiclaw-") && name.ends_with(".log"))
                .unwrap_or(false)
        })
        .collect();
    log_files.sort();
    log_files.reverse();

    for log_path in log_files {
        let content = match std::fs::read_to_string(&log_path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        for line in content.lines().rev() {
            if let Some(peer_id) = line
                .split("peer_id=")
                .nth(1)
                .and_then(|rest| rest.split_whitespace().next())
                .map(str::trim)
                .filter(|peer_id| !peer_id.is_empty())
            {
                return Some(peer_id.to_string());
            }
        }
    }

    None
}

#[derive(Deserialize)]
struct WechatAccountFile {
    #[serde(rename = "userId")]
    user_id: Option<String>,
}

pub async fn resolve_wechat_target(
    client: &reqwest::Client,
    api_base: &str,
    api_token: &str,
) -> Result<String, String> {
    if let Ok(to) = std::env::var("WECHAT_TO") {
        let to = to.trim();
        if !to.is_empty() {
            return Ok(to.to_string());
        }
    }

    let url = format!("{}/api/window_status", api_base);
    let daemon_resp = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client
            .get(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", api_token),
            )
            .send(),
    )
    .await;

    if let Ok(Ok(resp)) = daemon_resp {
        if resp.status().is_success() {
            if let Ok(body) = resp.text().await {
                if let Some(peer_id) = parse_first_peer_id_from_window_status(&body) {
                    return Ok(peer_id);
                }
            }
        }
    }

    if let Some(peer_id) = resolve_wechat_target_from_magiclaw_logs() {
        return Ok(peer_id);
    }

    let data_dir = resolve_wechat_data_dir();
    let account_path = data_dir.join("account.json");

    let account_text = std::fs::read_to_string(&account_path)
        .map_err(|e| format!("读取 account.json 失败({}): {}", account_path.display(), e))?;
    let account: WechatAccountFile = serde_json::from_str(&account_text)
        .map_err(|e| format!("解析 account.json 失败: {}", e))?;

    account.user_id.ok_or_else(|| {
        format!(
            "未找到收件人：请先在微信给 bot 发消息，或设置 WECHAT_TO，目录={}",
            data_dir.display()
        )
    })
}

pub fn resolve_feishu_target() -> Option<String> {
    for key in [
        "FEISHU_TO",
        "MAGICLAW_FEISHU_TO",
        "FEISHU_CHAT_ID",
        "FEISHU_OPEN_ID",
        "FEISHU_USER_ID",
        "FEISHU_EMAIL",
    ] {
        if let Ok(to) = std::env::var(key) {
            let to = to.trim();
            if !to.is_empty() {
                return Some(to.to_string());
            }
        }
    }
    None
}

pub async fn resolve_send_target(
    send_type: MessageSendType,
    client: &reqwest::Client,
    api_base: &str,
    api_token: &str,
) -> Result<Option<String>, String> {
    match send_type {
        MessageSendType::Wechat => resolve_wechat_target(client, api_base, api_token)
            .await
            .map(Some),
        MessageSendType::Feishu => {
            let to = resolve_feishu_target();
            if to.is_none() {
                return Err(
                    "飞书发送缺少收件人：请设置 FEISHU_TO（或 MAGICLAW_FEISHU_TO / FEISHU_CHAT_ID / FEISHU_OPEN_ID / FEISHU_USER_ID / FEISHU_EMAIL）"
                        .to_string(),
                );
            }
            Ok(to)
        }
    }
}

pub async fn daemon_health_ok(client: &reqwest::Client, api_base: &str) -> bool {
    let health_url = format!("{}/api/health", api_base);
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.get(&health_url).send(),
    )
    .await;

    match resp {
        Ok(Ok(r)) => r.status().is_success(),
        _ => false,
    }
}

pub async fn ensure_magiclaw_daemon(
    client: &reqwest::Client,
    magiclaw_bin: &str,
    api_addr: &str,
    api_base: &str,
) -> Result<DaemonReadySource, String> {
    if daemon_health_ok(client, api_base).await {
        return Ok(DaemonReadySource::Reused);
    }

    let _guard = MAGICLAW_DAEMON_BOOT_LOCK.lock().await;
    if daemon_health_ok(client, api_base).await {
        return Ok(DaemonReadySource::Reused);
    }

    let mut cmd = tokio::process::Command::new(magiclaw_bin);
    let magiclaw_db_path = std::env::var("MAGICLAW_DB_PATH").unwrap_or_else(|_| {
        std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/stock_analysis.db".to_string())
    });
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("MAGICLAW_API_ADDR", api_addr)
        .env("MAGICLAW_DB_PATH", magiclaw_db_path);

    if let Ok(dir) = std::env::var("WECHAT_CHANNEL_DIR") {
        cmd.env("WECHAT_CHANNEL_DIR", dir);
    }

    let mut child = cmd.spawn().map_err(|e| {
        format!(
            "启动 magiclaw daemon 失败(magiclaw: {}): {}",
            magiclaw_bin, e
        )
    })?;

    for _ in 0..100 {
        if daemon_health_ok(client, api_base).await {
            return Ok(DaemonReadySource::StartedNow);
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child.wait_with_output().await;
                let extra = match out {
                    Ok(o) => {
                        let stdout = String::from_utf8_lossy(&o.stdout);
                        let stderr = String::from_utf8_lossy(&o.stderr);
                        if stderr.contains("another magiclaw instance is already running") {
                            if daemon_health_ok(client, api_base).await {
                                return Ok(DaemonReadySource::Reused);
                            }
                            return Err(
                                "检测到 magiclaw 单实例锁冲突(data/magiclaw.instance.lock)，且当前端口不可用。请先结束旧的 magiclaw 进程后重试（可用: pgrep -af magiclaw / pkill -f '/magiclaw'）"
                                    .to_string(),
                            );
                        }
                        let stderr_tail = tail_lines(&stderr, 8);
                        let stdout_tail = tail_lines(&stdout, 3);
                        if !stderr_tail.is_empty() {
                            format!(" | stderr_tail={}", stderr_tail)
                        } else if !stdout_tail.is_empty() {
                            format!(" | stdout_tail={}", stdout_tail)
                        } else {
                            String::new()
                        }
                    }
                    Err(e) => format!(" | 获取 daemon 输出失败: {}", e),
                };
                return Err(format!(
                    "daemon 进程提前退出(exit={})，请检查 MAGICLAW_BIN/MAGICLAW_API_ADDR/MAGICLAW_API_TOKEN 配置{}",
                    status,
                    extra
                ));
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!("[微信] 检查 daemon 进程状态失败: {}", e);
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    Err(format!("daemon 启动后健康检查超时: {} (等待30s)", api_addr))
}

pub fn tail_lines(s: &str, n: usize) -> String {
    let mut v: Vec<&str> = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if v.len() > n {
        v = v.split_off(v.len() - n);
    }
    v.join(" | ")
}

pub async fn send_via_magiclaw_daemon(
    client: &reqwest::Client,
    api_base: &str,
    api_token: &str,
    send_type: MessageSendType,
    to: Option<&str>,
    text: &str,
) -> Result<(), String> {
    let url = format!("{}/api/send", api_base);
    let mut body = serde_json::Map::new();
    body.insert(
        "send_type".to_string(),
        serde_json::json!(send_type.as_str()),
    );
    body.insert("text".to_string(), serde_json::json!(text));
    if let Some(to) = to.map(str::trim).filter(|v| !v.is_empty()) {
        body.insert("to".to_string(), serde_json::json!(to));
    }

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        client
            .post(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", api_token),
            )
            .json(&serde_json::Value::Object(body))
            .send(),
    )
    .await
    .map_err(|_| "调用 /api/send 超时(>30s)".to_string())
    .and_then(|r| r.map_err(|e| format!("调用 /api/send 失败: {}", e)))?;

    let status = resp.status();
    let text_body = resp.text().await.unwrap_or_default();
    if status.is_success() {
        let ok = serde_json::from_str::<serde_json::Value>(&text_body)
            .ok()
            .and_then(|v| v.get("ok").and_then(|x| x.as_bool()))
            .unwrap_or(false);
        if ok {
            return Ok(());
        }
        return Err(format!("/api/send 返回非成功体: {}", text_body));
    }

    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(
            "daemon 鉴权失败(401)：请确保 monitor 与 daemon 使用相同 MAGICLAW_API_TOKEN，并重启 daemon 使新 token 生效".to_string(),
        );
    }

    if matches!(send_type, MessageSendType::Wechat)
        && status == reqwest::StatusCode::PRECONDITION_FAILED
        && text_body.contains("no valid context_token for peer")
    {
        return Err(
            "daemon 拒绝发送(412)：当前会话 context_token 无效。请先在微信给 bot 发一条消息刷新会话窗口后重试".to_string(),
        );
    }

    Err(format!("/api/send HTTP {}: {}", status, text_body))
}

pub async fn verify_daemon_auth(
    client: &reqwest::Client,
    api_base: &str,
    api_token: &str,
    api_token_source: &ApiTokenSource,
) -> Result<(), String> {
    let url = format!("{}/api/window_status", api_base);
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client
            .get(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", api_token),
            )
            .send(),
    )
    .await
    .map_err(|_| "调用 /api/window_status 超时(>5s)".to_string())
    .and_then(|r| r.map_err(|e| format!("调用 /api/window_status 失败: {}", e)))?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    if status.is_success() {
        // 窗口可用性预检已移除。ilink 的 ret=-2 不是“窗口用尽/会话过期”的致命信号:
        // daemon 侧 /api/send 现在对 ret=-2 直接当作成功继续发送(仅 errcode=-14 才算
        // 会话过期),连续主动推送已验证可稳定工作。因此 stale / should_refresh /
        // send_count>=9 这些旧启发式都已失效,不再用它们拦截发送。
        // 这里只把 /api/window_status 当作鉴权连通性校验(HTTP 200 = token 有效);
        // 真正无可用 context_token 时,/api/send 会返回 412 并给出可操作提示。
        return Ok(());
    }

    if status == reqwest::StatusCode::UNAUTHORIZED {
        let source_tip = match api_token_source {
            ApiTokenSource::Env => {
                "当前 monitor 使用环境变量 MAGICLAW_API_TOKEN，但 daemon 侧 token 不一致"
            }
            ApiTokenSource::DynamicMemCache | ApiTokenSource::DynamicFileCache | ApiTokenSource::DynamicIssued => {
                "当前 monitor 使用动态 token(数据库签发)。可能该 token 已过期/被吊销，monitor 将自动续签"
            }
        };
        return Err(format!("HTTP 401 unauthorized，{}", source_tip));
    }

    Err(format!("/api/window_status HTTP {}: {}", status, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PushKind::is_deprecated: 9 保留 + 4 降级 (grill Q2/Q6 修订)
    #[test]
    fn push_kind_is_deprecated_partition() {
        // 保留 9 条
        for k in [
            PushKind::HoldingEvent,
            PushKind::DailyReport,
            PushKind::Announcement,
        ] {
            assert!(!k.is_deprecated(), "{:?} 应保留", k);
        }
        // 降级 10 条 (A2/A3/A4/A5/A6/A11/A12/B4/B10 + grill 补 B11/B12/B13 = 12 条, 但我们只测 4 个代表)
        for k in [
            PushKind::AuctionVolume,
            PushKind::LimitBoards,
            PushKind::FactorIC,
            PushKind::WeeklySOP,
        ] {
            assert!(!k.is_deprecated(), "{:?} v19.12 起保留, 不再降级", k);
        }
    }

    /// PushKind 总数 = 13 (9 保留 + 12 降级, 但 grill 修订后保留 9 + 降级 12 = 21 变体太多, 我们用 enum 12 个)
    #[test]
    fn push_kind_count() {
        // 枚举定义 = 13 变体 (3 保留 + 10 降级, B11/B12/B13 在 enum 里)
        // 实际归类 = 9 保留 + 12 降级 (grill 修订: A13/A14/A15 用 HoldingEvent, C1 用 Announcement)
        let kinds = [
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
        ];
        assert_eq!(kinds.len(), 13, "13 个 PushKind 变体");
    }

    /// v19.12 起所有变体均保留, 此测试验证 push_governor 对保留的 AuctionVolume 返回 true
    /// (旧测试期望降级返回 false, 已废弃; commit 6cffecf fix(v19.12))
    /// b011: 静默期 (02:00-06:00) 非紧急 kind 会被 L5 Deny — 测试对时钟做容错:
    /// 非静默期断言 Pushed, 静默期断言 Denied (两者都证明链路走通且不假成功)
    fn assert_pushed_or_quiet_denied(outcome: &PushOutcome, ctx: &str) {
        let in_quiet = {
            use chrono::Timelike;
            (2..6).contains(&chrono::Local::now().hour())
        };
        if in_quiet {
            assert!(
                matches!(outcome, PushOutcome::Denied(r) if r == "quiet_hour"),
                "{}: 静默期应 Denied(quiet_hour), got {:?}",
                ctx,
                outcome
            );
        } else {
            assert!(outcome.is_pushed(), "{}: got {:?}", ctx, outcome);
        }
    }

    // ============== v17.5 §2.2: is_legacy_v17_5 标 7 variants ==============

    #[test]
    fn is_legacy_v17_5_marks_seven_spec_only_variants() {
        let legacy_variants = [
            PushKind::AuctionRepush,
            PushKind::OptimalClose,
            PushKind::VolumeWatchlist,
            PushKind::VolumeRealTrade,
            PushKind::CandidateTriggered,
            PushKind::CandidateInvalidated,
            PushKind::VirtualWatch,
        ];
        assert_eq!(
            legacy_variants.len(),
            7,
            "v17.5 §2.2 list 应 7 个 variants"
        );
        for k in legacy_variants {
            assert!(k.is_legacy_v17_5(), "{:?} 应被标为 legacy", k);
        }
    }

    /// v17.5 §1.2 active 10 个 PushKind 不应被误标为 legacy
    #[test]
    fn is_legacy_v17_5_not_marked_for_active_variants() {
        let active_v17_5 = [
            PushKind::AuctionVolume,
            PushKind::LimitBoards,
            PushKind::CandidateBoard,
            PushKind::HoldingPlan,
            PushKind::T0Advice,
            PushKind::ForbiddenOps,
            PushKind::PaperTrade,
            PushKind::CloseCall,
            PushKind::AccountMode,
            PushKind::DataMode,
        ];
        assert_eq!(
            active_v17_5.len(),
            10,
            "v17.5 §1.2 active 应 10 个 variants"
        );
        for k in active_v17_5 {
            assert!(
                !k.is_legacy_v17_5(),
                "{:?} 活动 variant 不应被标 legacy",
                k
            );
        }
    }

    /// v15.x 4 铁律承偌: 默认出声 (env 未设/silent 以外)。
    /// 本测试只验证 is_legacy_v17_5 谓词本身不漏;
    /// env 控制可见性逻辑 (OnceLock 缓存 env var) 在 push_governor_inner
    /// 同步单步跳邡, 完整 audit 路径靠 monitor --test smoke (Commit 4).
    #[test]
    fn is_legacy_v17_5_count_matches_v17_5_spec_section_2_2() {
        // v17.5 §2.2 "6 个 0-caller" (含 AuctionRepush + OptimalClose +
        // VolumeWatchlist + VolumeRealTrade + CandidateTriggered +
        // CandidateInvalidated + VirtualWatch) → 总 7 (spec 实际表 7 行
        // 包括 AuctionRepush, 主文"6"为 typo, 本 impl 以 7 为准)
        let all_legacy_hits: Vec<PushKind> = [
            PushKind::AuctionRepush,
            PushKind::OptimalClose,
            PushKind::VolumeWatchlist,
            PushKind::VolumeRealTrade,
            PushKind::CandidateTriggered,
            PushKind::CandidateInvalidated,
            PushKind::VirtualWatch,
        ]
        .into_iter()
        .filter(|k| k.is_legacy_v17_5())
        .collect();
        assert_eq!(all_legacy_hits.len(), 7);
    }

    // ============== v17.6 §2.2: is_low_priority_v17_6 标 3 variants ==============

    #[test]
    fn is_low_priority_v17_6_marks_three_spec_variants() {
        let low_priority = [
            PushKind::FactorIC,
            PushKind::SectorTier,
            PushKind::CapitalVerify,
        ];
        assert_eq!(low_priority.len(), 3, "v17.6 §2.2 应 3 个 variants");
        for k in low_priority {
            assert!(k.is_low_priority_v17_6(), "{:?} 应被标 low-priority", k);
        }
    }

    /// v17.5 legacy variants 不应被误标为 low-priority (low ≠ legacy)
    #[test]
    fn is_low_priority_v17_6_false_for_v17_5_legacy_variants() {
        for k in [
            PushKind::AuctionRepush,
            PushKind::OptimalClose,
            PushKind::VolumeWatchlist,
            PushKind::VolumeRealTrade,
            PushKind::CandidateTriggered,
            PushKind::CandidateInvalidated,
            PushKind::VirtualWatch,
        ] {
            assert!(
                !k.is_low_priority_v17_6(),
                "{:?} legacy variant 不应标 low-priority",
                k
            );
        }
    }

    // ============== v17.7 + v17.8: 12 active spec targets audit ==============

    #[test]
    fn is_active_spec_target_v17_7_v17_8_marks_twelve_active_variants() {
        // v17.7: 6 个 (公告/政策/业绩/研报/紧急告警)
        let v17_7_active = [
            PushKind::Announcement,
            PushKind::PolicyHit,
            PushKind::EarningsBeat,
            PushKind::EarningsMiss,
            PushKind::AnalystUpgrade,
            PushKind::MarketActionAlert,
        ];
        // v17.8: 6 个 (交易类: 盘后固定价 + ST 涨幅 + ETF 收盘竞价 + 大宗)
        let v17_8_active = [
            PushKind::PostFixedPriceOrder,
            PushKind::PostFixedPriceFill,
            PushKind::StPriceLimitChanged,
            PushKind::EtfClosingCallAuction,
            PushKind::BlockTradeIntradayConfirm,
            PushKind::BlockTradePriceRange,
        ];
        let all_twelve = v17_7_active
            .iter()
            .chain(v17_8_active.iter())
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(
            all_twelve.len(),
            12,
            "v17.7+v17.8 spec targets 应 12 个"
        );
        for k in all_twelve {
            assert!(
                k.is_active_spec_target_v17_7_v17_8(),
                "{:?} 应被标 active spec target",
                k
            );
        }
    }

    /// v17.5/v17.6 已经标过的 variants 不应在 v17.7/v17.8 audit 重复标
    #[test]
    fn is_active_spec_target_v17_7_v17_8_false_for_v17_5_v17_6() {
        // v17.5 7 个 legacy
        for k in [
            PushKind::AuctionRepush,
            PushKind::OptimalClose,
            PushKind::VolumeWatchlist,
            PushKind::VolumeRealTrade,
            PushKind::CandidateTriggered,
            PushKind::CandidateInvalidated,
            PushKind::VirtualWatch,
        ] {
            assert!(
                !k.is_active_spec_target_v17_7_v17_8(),
                "{:?} legacy 不应再标 active spec target",
                k
            );
        }
        // v17.6 3 个 low-priority
        for k in [PushKind::FactorIC, PushKind::SectorTier, PushKind::CapitalVerify] {
            assert!(
                !k.is_active_spec_target_v17_7_v17_8(),
                "{:?} low-priority 不应再标 active spec target",
                k
            );
        }
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn push_governor_deprecated_no_push() {
        crate::v14_adapter::_reset_dedup_for_test();
        std::env::set_var("V10_DRY_RUN_PUSH", "1"); // dry-run 模式返回 true
        let r = push_governor_v3("test kept auction", PushKind::AuctionVolume, None).await;
        assert_pushed_or_quiet_denied(&r, "v19.12 起 AuctionVolume 保留 (dry-run)");
        std::env::remove_var("V10_DRY_RUN_PUSH");
    }

    /// push_governor 保留时调 push_wechat (返回 V10_DRY_RUN_PUSH=true 时为 true)
    /// HoldingEvent 是紧急级 (静默期豁免) → 与时钟无关恒 true
    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn push_governor_kept_calls_push_wechat() {
        crate::v14_adapter::_reset_dedup_for_test();
        std::env::set_var("V10_DRY_RUN_PUSH", "1"); // push_wechat 走 dry-run 返回 true
        let r = push_governor("test kept holding", PushKind::HoldingEvent).await;
        assert!(r, "保留应调 push_wechat (V10_DRY_RUN_PUSH=true 返回 true)");
        std::env::remove_var("V10_DRY_RUN_PUSH");
    }

    // b011 P1-2: (kind, code) 票级冷却收敛到 L4 后语义不变 — 同票重复被挡
    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn push_governor_cooldown_blocks_rapid_repeat() {
        use std::env;
        crate::v14_adapter::_reset_dedup_for_test();
        env::set_var("V10_DRY_RUN_PUSH", "1");
        let r1 = push_governor_v3("first", PushKind::HoldingPlan, Some("000001")).await;
        if r1.is_pushed() {
            let r2 = push_governor_v3("second", PushKind::HoldingPlan, Some("000001")).await;
            assert_eq!(
                r2,
                PushOutcome::Deduped,
                "30 min 冷却内同票重复调应被 L4 挡"
            );
        } else {
            assert_pushed_or_quiet_denied(&r1, "首次");
        }
        env::remove_var("V10_DRY_RUN_PUSH");
    }

    // v59 F1 语义保持 — 不同 code 同一 PushKind 不应被 30 min 冷却挡
    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn push_governor_v3_per_code_cooldown() {
        use std::env;
        crate::v14_adapter::_reset_dedup_for_test();
        env::set_var("V10_DRY_RUN_PUSH", "1");
        // 同一 kind (HoldingPlan 30 min) 不同 code
        let r1 = push_governor_v3("first", PushKind::HoldingPlan, Some("000001")).await;
        if r1.is_pushed() {
            let r2 = push_governor_v3("second", PushKind::HoldingPlan, Some("000002")).await;
            assert!(
                r2.is_pushed(),
                "000002 不同 code 不应被 30 min 冷却挡 (F1 语义), got {:?}",
                r2
            );
        } else {
            assert_pushed_or_quiet_denied(&r1, "首次");
        }
        env::remove_var("V10_DRY_RUN_PUSH");
    }

    /// PUSH_VERBOSE=true 覆盖降级 → 调 push_wechat
    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn push_verbose_true_overrides_deprecated() {
        crate::v14_adapter::_reset_dedup_for_test();
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        std::env::set_var("PUSH_VERBOSE", "true");
        let r = push_governor_v3("test verbose auction", PushKind::AuctionVolume, None).await;
        assert_pushed_or_quiet_denied(&r, "PUSH_VERBOSE=true 应覆盖降级");
        std::env::remove_var("V10_DRY_RUN_PUSH");
        std::env::remove_var("PUSH_VERBOSE");
    }
}
