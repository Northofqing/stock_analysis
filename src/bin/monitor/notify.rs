//! Registered business rules: BR-047, BR-048, BR-077, BR-137.
//! 通知推送 + MagicLaw 守护进程 + Token 管理
//!
//! 从 main.rs 提取，减少单文件体积。

use serde::Deserialize;
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
#[allow(
    dead_code,
    reason = "PushKind is a versioned wire/governance catalog and retains compatibility variants across staged migrations"
)]
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
    // v17.5 审计删除 (2026-07-16): OptimalClose / VolumeWatchlist / VolumeRealTrade
    //   逐变体调用链审计确认 0 生产 caller, 归档 docs/v15.x/dead-pushkinds.md
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
    /// 数据模式变更 (T-02, ⚡ 状态变更即推、无粗粒度冷却) [MVP-1]
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
    /// v13.1 §5.6 / BR-033 创业板协议大宗盘中实时确认
    BlockTradeIntradayConfirm,
    /// v13.1 §5.7 / BR-034 北交所大宗价格区间
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
    // ============= v17.4 §5.1 能力1: 全天新闻聚合 (BR-033) =============
    /// v17.4: 高分新闻即时推 (strength≥80 且 certainty≥60, ⚡ 5min/事件)
    NewsFlashCritical,
    /// v17.4: 4 时段 (9:30/11:30/13:00/15:00) 聚合 Top3 (ℹ️ 1次/窗口/日)
    NewsFlashAggregated,
}

#[allow(
    dead_code,
    reason = "catalog metadata is independently audited and not every query is used by the production binary"
)]
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
    /// 4 个 variants: AuctionRepush, CandidateTriggered, CandidateInvalidated, VirtualWatch.
    /// (2026-07-16 审计: OptimalClose/VolumeWatchlist/VolumeRealTrade 已删;
    ///  CandidateTriggered/VirtualWatch 实为活链路, 仅保留 legacy 标记待后续迁移评估)
    pub fn is_legacy_v17_5(self) -> bool {
        matches!(
            self,
            Self::AuctionRepush
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

    /// v17.7 + v17.8: 12 个 spec 标的变体保持 active
    /// (有 production caller + metadata getter, 跟 v17.6 同样 gap).
    ///
    /// spec 字面写"6 项 0-caller" / "8 项交易类清理" — 实证不符:
    ///   - v17.7: Announcement, PolicyHit, EarningsBeat, EarningsMiss,
    ///     AnalystUpgrade, MarketActionAlert (6 个, 全部 active)
    ///   - v17.8: PostFixedPriceOrder, PostFixedPriceFill, StPriceLimitChanged,
    ///     EtfClosingCallAuction, BlockTradeIntradayConfirm, BlockTradePriceRange.
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

    /// v17.6 §5.1: 3 个 low-priority variants (FactorIC / SectorTier / CapitalVerify)
    /// 现在是 DailyReport 的"子段" — 推送时通过 `DailyReportSubKind` 标识子类型.
    ///
    /// 设计取舍: 不删 enum 变体 (向后兼容 + 现有 9 callsite 不破), 仅在 metadata
    /// 层 (本方法) 标"它们是 DailyReport 的子段". 后续 `daily_report_router` 模块
    /// 用本方法分流, 推送时仍走 PushKind::DailyReport 主路径 (cooldown 24h),
    /// 但 sub_kind 在 title prefix 区分 (e.g. "[FactorIC] ...") 避免合并后丢失语义.
    pub fn daily_report_sub_kind(self) -> Option<DailyReportSubKind> {
        match self {
            Self::FactorIC => Some(DailyReportSubKind::FactorIC),
            Self::SectorTier => Some(DailyReportSubKind::SectorTier),
            Self::CapitalVerify => Some(DailyReportSubKind::CapitalVerify),
            _ => None,
        }
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
            // v17.4 能力1: 高分新闻即时推重要级 (聚合 NewsFlashAggregated 走默认 Info)
            | PushKind::NewsFlashCritical => PushLevel::Important,
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
            PushKind::AccountMode | PushKind::DataMode | PushKind::HoldingEvent => None,
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
            PushKind::PolicyHit => Some(86_400),      // 1次/日
            PushKind::EarningsBeat => Some(43_200),   // 12h
            PushKind::EarningsMiss => Some(43_200),   // 12h
            PushKind::AnalystUpgrade => Some(86_400), // 1次/日
            PushKind::MarketActionAlert => Some(60),  // 1 min/票 (实盘异常需立即)
            // v17.4 能力1 (BR-082)
            PushKind::NewsFlashCritical => Some(300), // 5 min/事件 (code=event_id 前缀)
            PushKind::NewsFlashAggregated => Some(3600), // 1h/窗口 (code=窗口标签)
            _ => Some(1800),                          // 默认 30min
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
            | BlockTradePriceRange => CooldownScope::PerTicket,
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
            PushKind::NewsFlashCritical => "新闻快讯",
            PushKind::NewsFlashAggregated => "新闻时段聚合",
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

/// v17.6 §5.1: DailyReport 子段枚举.
///
/// 收纳原 PushKind 中 3 个 low-priority variants (FactorIC / SectorTier / CapitalVerify).
/// 它们都归属于"日报类"推送 (DailyReport), 但语义不同需在 title 区分, 因此引入子枚举.
///
/// 用法: `daily_report_router::push_factor_ic(text)` 内部走 `PushKind::DailyReport` 主路径
/// + title prefix "[FactorIC] ..." 标识子类型. cooldown 复用 `PushKind::DailyReport` 的 24h.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum DailyReportSubKind {
    /// 因子 IC (grill Q6 改)
    FactorIC,
    /// v4 赛道分档
    SectorTier,
    /// v4 资金验证
    CapitalVerify,
}

#[allow(
    dead_code,
    reason = "stable template metadata is part of the versioned audit contract and is also exercised by tests"
)]
impl DailyReportSubKind {
    /// 简短标签 (log + title prefix 用)
    pub fn label(self) -> &'static str {
        match self {
            Self::FactorIC => "FactorIC",
            Self::SectorTier => "SectorTier",
            Self::CapitalVerify => "CapitalVerify",
        }
    }

    /// 对应原 PushKind variant (用于 audit / 回退路径)
    pub fn legacy_kind(self) -> PushKind {
        match self {
            Self::FactorIC => PushKind::FactorIC,
            Self::SectorTier => PushKind::SectorTier,
            Self::CapitalVerify => PushKind::CapitalVerify,
        }
    }

    /// 子段独立冷却 (秒). None = 跟随 DailyReport 主 24h.
    /// 设计: 默认 None, 让 sub_kind 共享 DailyReport 1次/日窗口 (避免重复噪声).
    /// 个别场景可 override: SectorTier/CapitalVerify 偏实时 → 30min 独立窗口.
    pub fn cooldown_secs(self) -> Option<u32> {
        match self {
            Self::FactorIC => None,
            Self::SectorTier => Some(1800), // 30min
            Self::CapitalVerify => Some(1800),
        }
    }

    /// 稳定 template_id (snake_case + _v1, 跟 PushKind 一致规则)
    pub fn stable_template_id(self) -> String {
        // DailyReport 主路径是 daily_report_v1, 子段加 _sub suffix
        format!("daily_report_{}_v1", self.label().to_ascii_lowercase())
    }
}

impl PushKind {
    /// v17.x DispatchTable: 查表拿元数据 (audit 用, 后续 spec 治理阶段统一迁).
    /// 不在表内 → None (现有 5 个 match 块仍兜底).
    pub fn dispatch_row(self) -> Option<DispatchRow> {
        DISPATCH_TABLE
            .iter()
            .find(|(k, _)| *k == self)
            .map(|(_, row)| *row)
    }
}

/// v17.x DispatchTable: 15 audit-marked PushKind 的元数据集中表.
///
/// 整合:
/// - v17.6 §2.2: 3 个 low-priority variants (FactorIC / SectorTier / CapitalVerify)
/// - v17.7 + v17.8: 10 个 active spec targets
///   (Announcement, PolicyHit, EarningsBeat, EarningsMiss, AnalystUpgrade,
///   MarketActionAlert, PostFixedPriceOrder, PostFixedPriceFill,
///   StPriceLimitChanged, EtfClosingCallAuction,
///   BlockTradeIntradayConfirm, BlockTradePriceRange)
///
/// 设计 (Path D 一致): 不替换现有 `PushKind::level/cooldown_secs/cooldown_scope/
/// label/stable_template_id` 5 个 match 块 — 仅作 audit 跟踪 + 后续 spec 治理
/// 的 single source of truth. 调用方仍走原方法, 改动最小.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DispatchRow {
    /// 等级 (Emergency / Important / Info)
    pub level: PushLevel,
    /// 冷却秒数 (None = 无冷却)
    pub cooldown_secs: Option<u32>,
    /// L4 dedup 键语义
    pub cooldown_scope: CooldownScope,
    /// log + UI 简短标签
    pub label: &'static str,
    /// 稳定 template_id (snake_case + _v1)
    pub stable_template_id: &'static str,
}

/// v17.x 集中 Dispatch 表 — 15 audit-marked variants 的元数据.
///
/// 顺序: 先 v17.6 (3 个 low-priority), 再 v17.7 (6 active), 最后 v17.8 (6 active).
/// 总数 = 3 + 6 + 6 = 15 (跟 spec 字面一致).
///
/// 字段值跟现有 match 块当前实现保持一致 — 本表是"快照", 后续如要修改某 variant
/// 的 level/cooldown, 必须**同步**改 match 块 (留待 spec 治理阶段统一迁).
pub const DISPATCH_TABLE: &[(PushKind, DispatchRow)] = &[
    // ============== v17.6 §2.2: 3 low-priority (现 DailyReportSubKind 收纳) ==============
    (
        PushKind::FactorIC,
        DispatchRow {
            level: PushLevel::Info,
            cooldown_secs: Some(3600),
            cooldown_scope: CooldownScope::Global,
            label: "因子IC",
            stable_template_id: "factoric_v1",
        },
    ),
    (
        PushKind::SectorTier,
        DispatchRow {
            level: PushLevel::Info,
            cooldown_secs: Some(1800),
            cooldown_scope: CooldownScope::Global,
            label: "赛道分档",
            stable_template_id: "sectortier_v1",
        },
    ),
    (
        PushKind::CapitalVerify,
        DispatchRow {
            level: PushLevel::Info,
            cooldown_secs: Some(1800),
            cooldown_scope: CooldownScope::Global,
            label: "资金验证",
            stable_template_id: "capitalverify_v1",
        },
    ),
    // ============== v17.7: 6 active (Announcement + 政策 + 业绩 + 评级 + 实盘异常) ==============
    (
        PushKind::Announcement,
        DispatchRow {
            level: PushLevel::Important,
            cooldown_secs: Some(1800), // 默认 30min, 由 sm 状态机治理实际节流
            cooldown_scope: CooldownScope::External,
            label: "公告",
            stable_template_id: "announcement_v1",
        },
    ),
    (
        PushKind::PolicyHit,
        DispatchRow {
            level: PushLevel::Important,
            cooldown_secs: Some(86_400),
            cooldown_scope: CooldownScope::Global,
            label: "政策催化",
            stable_template_id: "policyhit_v1",
        },
    ),
    (
        PushKind::EarningsBeat,
        DispatchRow {
            level: PushLevel::Important,
            cooldown_secs: Some(43_200),
            cooldown_scope: CooldownScope::Global,
            label: "业绩超预期",
            stable_template_id: "earningsbeat_v1",
        },
    ),
    (
        PushKind::EarningsMiss,
        DispatchRow {
            level: PushLevel::Important,
            cooldown_secs: Some(43_200),
            cooldown_scope: CooldownScope::Global,
            label: "业绩低于预期",
            stable_template_id: "earningsmiss_v1",
        },
    ),
    (
        PushKind::AnalystUpgrade,
        DispatchRow {
            level: PushLevel::Important,
            cooldown_secs: Some(86_400),
            cooldown_scope: CooldownScope::Global,
            label: "卖方评级上调",
            stable_template_id: "analystupgrade_v1",
        },
    ),
    (
        PushKind::MarketActionAlert,
        DispatchRow {
            level: PushLevel::Emergency,
            cooldown_secs: Some(60),
            cooldown_scope: CooldownScope::Global,
            label: "实盘异常",
            stable_template_id: "marketactionalert_v1",
        },
    ),
    // ============== v17.8: 6 active (盘后固定价 + ST + ETF + 大宗) ==============
    (
        PushKind::PostFixedPriceOrder,
        DispatchRow {
            level: PushLevel::Important,
            cooldown_secs: Some(60),
            cooldown_scope: CooldownScope::PerTicket,
            label: "盘后固定价格申报",
            stable_template_id: "postfixedpriceorder_v1",
        },
    ),
    (
        PushKind::PostFixedPriceFill,
        DispatchRow {
            level: PushLevel::Important,
            cooldown_secs: Some(300),
            cooldown_scope: CooldownScope::PerTicket,
            label: "盘后固定价格成交",
            stable_template_id: "postfixedpricefill_v1",
        },
    ),
    (
        PushKind::StPriceLimitChanged,
        DispatchRow {
            level: PushLevel::Important,
            cooldown_secs: Some(86_400),
            cooldown_scope: CooldownScope::PerTicket,
            label: "ST 涨跌幅变更",
            stable_template_id: "stpricelimitchanged_v1",
        },
    ),
    (
        PushKind::EtfClosingCallAuction,
        DispatchRow {
            level: PushLevel::Important,
            cooldown_secs: Some(86_400),
            cooldown_scope: CooldownScope::Global,
            label: "ETF 集合竞价尾盘",
            stable_template_id: "etfclosingcallauction_v1",
        },
    ),
    (
        PushKind::BlockTradeIntradayConfirm,
        DispatchRow {
            level: PushLevel::Important,
            cooldown_secs: Some(300),
            cooldown_scope: CooldownScope::PerTicket,
            label: "大宗盘中确认",
            stable_template_id: "blocktradeintradayconfirm_v1",
        },
    ),
    (
        PushKind::BlockTradePriceRange,
        DispatchRow {
            level: PushLevel::Important,
            cooldown_secs: Some(3600),
            cooldown_scope: CooldownScope::PerTicket,
            label: "北交所大宗价格区间",
            stable_template_id: "blocktradepricerange_v1",
        },
    ),
];

/// 启动时 audit — 仅打印 summary (行数 + 字段分布), 不逐行打印 15 行 (修 FINDING #8: 启动噪声).
/// 详细表内容按需在运行时通过 push_governor_v3 命中具体 kind 时按需 log (push_governor_inner 内
/// kind.dispatch_row() 已接) — 避免每次重启刷屏.
pub fn dispatch_table_init_audit() {
    let emergency_count = DISPATCH_TABLE
        .iter()
        .filter(|(_, r)| matches!(r.level, PushLevel::Emergency))
        .count();
    let important_count = DISPATCH_TABLE
        .iter()
        .filter(|(_, r)| matches!(r.level, PushLevel::Important))
        .count();
    let info_count = DISPATCH_TABLE
        .iter()
        .filter(|(_, r)| matches!(r.level, PushLevel::Info))
        .count();
    log::info!(
        "[v17.x] DISPATCH_TABLE init: {} rows (Emergency={} Important={} Info={}); 逐行 metadata 见运行时 push_governor_inner",
        DISPATCH_TABLE.len(),
        emergency_count,
        important_count,
        info_count
    );
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

#[allow(
    dead_code,
    reason = "human-readable level labels are retained for audit and diagnostic consumers"
)]
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
///   - 路径: data/push_log/YYYY-MM-DD/HHMMSS_<唯一审计后缀>.md
///   - 沙箱 V10_DRY_RUN_PUSH=1 也保存 (用户能查测试推送)
///   - 写失败显式返回，禁止在审计证据缺失时继续确认投递
fn push_log_suffix_at(now: std::time::SystemTime) -> Result<String, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let nanos = now
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("push_log system clock is before UNIX epoch: {error}"))?
        .as_nanos();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!(
        "{nanos:032x}_{:08x}_{sequence:016x}",
        std::process::id()
    ))
}

fn create_push_log_file(path: &std::path::Path) -> Result<std::fs::File, String> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("push_log 不可覆盖创建失败 {}: {error}", path.display()))
}

fn save_push_log(text: &str) -> Result<std::path::PathBuf, String> {
    use std::io::Write;
    log::info!(
        "[v69] save_push_log entered, text len={}",
        text.chars().count()
    );
    let now = chrono::Local::now();
    let date_dir = now.format("%Y-%m-%d").to_string();
    let time_prefix = now.format("%H%M%S").to_string();
    let unique_suffix = push_log_suffix_at(std::time::SystemTime::now())?;
    let root = std::env::var("PUSH_LOG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            if cfg!(test) || std::env::var("STOCK_ENV_MODE").ok().as_deref() == Some("test") {
                std::path::PathBuf::from("data/test/push_log")
            } else {
                std::path::PathBuf::from("data/push_log")
            }
        });
    let dir = root.join(&date_dir);
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("push_log 目录创建失败 {}: {error}", dir.display()))?;
    let path = dir.join(format!("{time_prefix}_{unique_suffix}.md"));
    let mut file = create_push_log_file(&path)?;
    file.write_all(text.as_bytes())
        .map_err(|error| format!("push_log 写入失败 {}: {error}", path.display()))?;
    file.sync_data()
        .map_err(|error| format!("push_log fsync 失败 {}: {error}", path.display()))?;
    log::info!("[v69] push_log 写入: {}", path.display());
    Ok(path)
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
    Pushed,            // v14.2 + v13 都成功
    Deduped,           // v14.2 dedup hit, v13 未推送 (60s 内同 kind)
    Denied(String),    // v14.2 governance 拦截
    SinkError(String), // v14.2 sink 失败
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
    push_governor_inner_with_source_fact(text, kind, code, None).await
}

async fn push_governor_inner_with_source_fact(
    text: &str,
    kind: PushKind,
    code: Option<&str>,
    source_fact: Option<&crate::v14_adapter::SourceFactEvidence>,
) -> PushOutcome {
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
    // v17.x: 命中 DISPATCH_TABLE 15 audit-marked 行时, 打印表内 metadata (single source of truth).
    //   修 FINDING #2 (dispatch_row 死代码) — 让表在生产路径真起作用.
    if let Some(row) = kind.dispatch_row() {
        log::info!(
            "[v17.x dispatch_row] PushKind::{:?} → level={:?} cd={:?}s scope={:?} label={:?} tid={:?}",
            kind,
            row.level,
            row.cooldown_secs,
            row.cooldown_scope,
            row.label,
            row.stable_template_id
        );
    }

    // b013 review P0-4: v14 路径也走 LaunchGate (b011 漏: 17 处 main::push_wechat
    // 走 launch_gate, v14 直连 push_wechat 不走 — Stage=gray 下非 critical 仍能推).
    if !launch_gate_check(kind) {
        return PushOutcome::Denied("launch_gate_stage".to_string());
    }
    let gate = match source_fact {
        Some(evidence) => v14_adapter::v14_gate_source_fact(evidence),
        None => v14_adapter::v14_gate(kind, code),
    };
    let event = match gate {
        V14Gate::Deduped => return PushOutcome::Deduped,
        V14Gate::Denied(reason) => return PushOutcome::Denied(reason),
        V14Gate::Approved(event) => *event,
    };
    let start = std::time::Instant::now();
    deliver_and_record(event, kind, text, start, None, None).await
}

/// v17.6 §5.1: push_governor_inner 的 sub_kind-aware 版本.
/// cooldown 取 sub_kind.cooldown_secs() override (None 时跟随 kind 默认).
async fn push_governor_inner_with_sub_kind(
    text: &str,
    kind: PushKind,
    code: Option<&str>,
    sub_kind: Option<DailyReportSubKind>,
) -> PushOutcome {
    use crate::v14_adapter::{self, V14Gate};
    // 复用 push_governor_inner 的 audit log / launch_gate (kind-only)
    // 然后在 L4 dedup 步改用 v14_gate_with_sub_kind
    if !launch_gate_check(kind) {
        return PushOutcome::Denied("launch_gate_stage".to_string());
    }
    let sub_kind_str = sub_kind.map(|s| s.label());
    // cooldown override: sub_kind.cooldown_secs() 优先, None 时回退 kind 默认
    let override_cooldown = sub_kind.and_then(|s| s.cooldown_secs());
    let event =
        match v14_adapter::v14_gate_with_sub_kind(kind, code, sub_kind_str, override_cooldown) {
            V14Gate::Deduped => return PushOutcome::Deduped,
            V14Gate::Denied(reason) => return PushOutcome::Denied(reason),
            V14Gate::Approved(event) => {
                if let Some(cd) = override_cooldown {
                    log::info!(
                        "[v17.6 §5.1] sub_kind {:?} 使用 {}s 独立冷却窗口",
                        sub_kind_str,
                        cd
                    );
                }
                *event
            }
        };
    let start = std::time::Instant::now();
    deliver_and_record(event, kind, text, start, sub_kind_str, override_cooldown).await
}

/// 公共尾段: L5/L6 投递 + L7/哈希链留痕 + commit/rollback.
/// push_governor_inner + push_governor_inner_with_sub_kind 共用 (DRY).
async fn deliver_and_record(
    event: stock_analysis::push_l1::SignalEvent,
    kind: PushKind,
    text: &str,
    start: std::time::Instant,
    sub_kind: Option<&str>,
    cooldown_override_secs: Option<u32>,
) -> PushOutcome {
    use crate::v14_adapter;
    // v15.1 A3: 把 reserve/commit 拆分, 失败时 rollback 不占 cooldown 窗口
    // v17.1-r2 §3.6: env opt-in 走 L6 SinkRouter (env=STOCK_ANALYSIS_PUSH_V6_ENABLE=1).
    let delivered = if std::env::var("STOCK_ANALYSIS_PUSH_V6_ENABLE")
        .ok()
        .as_deref()
        == Some("1")
    {
        let msg = crate::l6_sink::build_push_message(&event, text, kind);
        matches!(
            crate::l6_sink::sink_router().route(&msg).await,
            stock_analysis::push_l6::SinkResult::Ok
        )
    } else {
        push_wechat(text).await
    };
    // b013 review P2-15: 入口取一次 channel (避免 push_wechat await 后 env 抖动)
    let channel = current_send_channel();
    let l7_result = v14_adapter::v14_record_delivery(&event, kind, text, delivered, channel);

    // v17.1-r2 §3.6: 发布投递审计事件 (观察路径, 不干预推送)
    let outcome_str = match delivered {
        true => "Pushed",
        false => "SinkError",
    };
    // channel 已在上方取过: let channel = current_send_channel();
    // v17.3 Task 1 F1: 用实际投递耗时 (从 deliver_and_record 入口的 Instant 计算)
    let latency_ms = start.elapsed().as_millis() as u64;
    let audit_result = stock_analysis::event::publish_delivery(
        &kind.stable_template_id(),
        event.code.as_deref(),
        outcome_str,
        channel,
        text.len(),
        latency_ms,
    );
    let dedup_result = settle_dedup_after_delivery(
        &event,
        kind,
        sub_kind,
        cooldown_override_secs,
        delivered,
        l7_result.is_ok() && audit_result.is_ok(),
    );

    let mut audit_errors = Vec::new();
    if let Err(error) = dedup_result {
        audit_errors.push(format!("dedup state: {error}"));
    }
    if let Err(error) = l7_result {
        audit_errors.push(format!("L7 analytics: {error}"));
    }
    if let Err(error) = audit_result {
        audit_errors.push(format!("delivery hash-chain: {error}"));
    }
    if !audit_errors.is_empty() {
        let error = audit_errors.join("; ");
        log::error!("[push.delivery.audit][BR-091/BR-113] {error}");
        return PushOutcome::SinkError(format!(
            "delivery audit failed after sink outcome={outcome_str}: {error}"
        ));
    }

    if delivered {
        PushOutcome::Pushed
    } else {
        PushOutcome::SinkError("push_wechat returned false".to_string())
    }
}

/// BR-137: a source-fact identity is committed only after the sink and both
/// authoritative post-delivery records succeed. Any other outcome releases
/// the reservation so a later real provider poll can retry.
fn settle_dedup_after_delivery(
    event: &stock_analysis::push_l1::SignalEvent,
    kind: PushKind,
    sub_kind: Option<&str>,
    cooldown_override_secs: Option<u32>,
    delivered: bool,
    post_delivery_audits_ok: bool,
) -> Result<(), String> {
    if delivered && post_delivery_audits_ok {
        crate::v14_adapter::commit_dedup_for_event(event, kind, sub_kind, cooldown_override_secs)
    } else {
        crate::v14_adapter::rollback_dedup_for_event(event, kind, sub_kind, cooldown_override_secs)
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
    if dry_run_push_active() {
        "dry_run"
    } else {
        resolve_send_type().as_str()
    }
}

/// 无票号的全局模板入口。票级模板必须使用 `push_governor_v3` 并传真实代码；
/// 若误用本入口会显式拒绝，避免不同股票共享一个伪代码冷却桶。
pub async fn push_governor(text: &str, kind: PushKind) -> bool {
    if requires_ticket_code(kind) {
        log::error!(
            "[push_governor] {:?} 需要真实 code，拒绝无票号兼容调用",
            kind
        );
        return false;
    }
    push_governor_inner(text, kind, None).await.is_pushed()
}

/// v14.2 单入口 (b011 P1-2 收敛后 + b013 review P0-1): 返回 enum 区分 4 种结果.
/// `code`: 票级冷却键 (§14.3 "/票" 类 kind 必传 real 票号, 否则 L4 不做票级冷却).
pub async fn push_governor_v3(text: &str, kind: PushKind, code: Option<&str>) -> PushOutcome {
    push_governor_inner(text, kind, code).await
}

/// BR-137 sole delivery entry for a validated source-self-contained fact.
/// Kind and dedup identity are derived from the evidence so callers cannot
/// pair a relaxed source profile with an unrelated PushKind.
pub async fn push_source_fact_v3(
    text: &str,
    evidence: &crate::v14_adapter::SourceFactEvidence,
) -> PushOutcome {
    push_governor_inner_with_source_fact(
        text,
        evidence.kind(),
        evidence.security_code(),
        Some(evidence),
    )
    .await
}

/// v17.6 §5.1: push_governor_v3 的 sub_kind-aware 版本.
/// daily_report_router 三个公开函数 (route_factor_ic / route_sector_tier /
/// route_capital_verify) 调用 — 让 3 个 sub_kind 在 L4 dedup key 第三元组独立.
pub async fn push_governor_v3_with_sub_kind(
    text: &str,
    kind: PushKind,
    code: Option<&str>,
    sub_kind: Option<DailyReportSubKind>,
) -> PushOutcome {
    push_governor_inner_with_sub_kind(text, kind, code, sub_kind).await
}

/// b013 P0-1 兜底: PerTicket 类 kind 在缺 code 时塞占位, 让 L4 走全局 key,
/// 至少防止"无限重发同一票"。b014 应把所有 caller 改成 push_governor_v3 显式传 code。
fn requires_ticket_code(kind: PushKind) -> bool {
    use PushKind::*;
    matches!(
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
    )
}

pub async fn push_wechat(text: &str) -> bool {
    // v10 P6 5 要素接入: V10_DRY_RUN_PUSH=1 时跳过实际推送, 仅 log
    // 用于开发/验证推送内容变化, 不骚扰飞书
    if dry_run_push_active() {
        log::info!("[V10_DRY_RUN_PUSH] 跳过飞书推送, 内容预览:\n{}", text);
        // v69: 沙箱 dry-run 也保存 push_log
        if let Err(error) = save_push_log(text) {
            log::error!("[BR-086] dry-run push audit failed: {error}");
            return false;
        }
        return true;
    }

    // v69: 不管走哪条推送路径 (magiclaw cli / feishu http / 后续), 都先保存 push_log
    if let Err(error) = save_push_log(text) {
        log::error!("[BR-086] push audit failed; delivery blocked: {error}");
        return false;
    }

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

fn dry_run_push_active() -> bool {
    cfg!(test) || std::env::var("V10_DRY_RUN_PUSH").ok().as_deref() == Some("1")
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

    push_feishu_http_with_client(&client, &url, text).await
}

async fn push_feishu_http_with_client(client: &reqwest::Client, url: &str, text: &str) -> bool {
    let payload = serde_json::json!({
        "msg_type": "text",
        "content": {
            "text": text,
        }
    });

    let resp = match client.post(url).json(&payload).send().await {
        Ok(v) => v,
        Err(e) => {
            log::error!("[飞书] 推送失败: 调用 webhook 失败: {}", e);
            return false;
        }
    };

    let status = resp.status();
    let body_text = match resp.text().await {
        Ok(body) => body,
        Err(error) => {
            log::error!("[飞书] 推送失败: 读取 webhook 响应失败: {}", error);
            return false;
        }
    };
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
        match parse_magiclaw_cli_delivery_receipt(send_type, &stdout) {
            Ok(receipt) => {
                log::info!(
                    "[{}] 推送成功 | via=cli receipt=validated message_id_len={} platform_msg_id_len={}",
                    send_type.label(),
                    receipt.message_id.len(),
                    receipt.platform_msg_id.len()
                );
                return true;
            }
            Err(error) => {
                log::error!(
                    "[{}][BR-111] magiclaw exit=0 but delivery receipt is invalid: {}",
                    send_type.label(),
                    error
                );
                return false;
            }
        }
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

#[derive(Debug, PartialEq, Eq)]
struct CliDeliveryReceipt {
    message_id: String,
    platform_msg_id: String,
}

fn parse_magiclaw_cli_delivery_receipt(
    send_type: MessageSendType,
    stdout: &str,
) -> Result<CliDeliveryReceipt, String> {
    let prefix = match send_type {
        MessageSendType::Feishu => "send ok (feishu):",
        MessageSendType::Wechat => "send ok:",
    };
    let line = stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(prefix))
        .ok_or_else(|| "missing channel-specific success receipt".to_string())?;

    let mut message_id = None;
    let mut platform_msg_id = None;
    for field in line
        .strip_prefix(prefix)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
    {
        let Some((key, value)) = field.split_once('=') else {
            continue;
        };
        match key.trim() {
            "message_id" => message_id = Some(value.trim()),
            "platform_msg_id" => platform_msg_id = Some(value.trim()),
            _ => {}
        }
    }

    let validate = |name: &str, value: Option<&str>| -> Result<String, String> {
        let value = value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing {name}"))?;
        if value.starts_with('<') && value.ends_with('>') {
            return Err(format!("placeholder {name}"));
        }
        Ok(value.to_string())
    };

    Ok(CliDeliveryReceipt {
        message_id: validate("message_id", message_id)?,
        platform_msg_id: validate("platform_msg_id", platform_msg_id)?,
    })
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
    let text_body = resp
        .text()
        .await
        .map_err(|error| format!("读取 /api/send 响应失败: {error}"))?;
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
    let body = resp
        .text()
        .await
        .map_err(|error| format!("读取 /api/window_status 响应失败: {error}"))?;

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

    #[test]
    fn push_log_suffix_rejects_pre_epoch_clock_and_is_unique() {
        let before_epoch = std::time::UNIX_EPOCH
            .checked_sub(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(push_log_suffix_at(before_epoch).is_err());

        let instant = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1);
        let first = push_log_suffix_at(instant).unwrap();
        let second = push_log_suffix_at(instant).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn push_log_artifact_creation_never_overwrites() {
        let suffix = push_log_suffix_at(std::time::SystemTime::now()).unwrap();
        let path = std::env::temp_dir().join(format!("TEST_CODE_push_log_{suffix}.md"));
        let first = create_push_log_file(&path).expect("create first audit artifact");
        drop(first);

        assert!(create_push_log_file(&path).is_err());
        std::fs::remove_file(path).expect("remove isolated audit fixture");
    }

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

    // ============== v17.5 §2.2: is_legacy_v17_5 标 4 variants (2026-07-16 审计后) ==============

    #[test]
    fn is_legacy_v17_5_marks_four_remaining_variants() {
        let legacy_variants = [
            PushKind::AuctionRepush,
            PushKind::CandidateTriggered,
            PushKind::CandidateInvalidated,
            PushKind::VirtualWatch,
        ];
        assert_eq!(
            legacy_variants.len(),
            4,
            "v17.5 §2.2 审计后应剩 4 个 variants (3 项已删)"
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
            assert!(!k.is_legacy_v17_5(), "{:?} 活动 variant 不应被标 legacy", k);
        }
    }

    /// v15.x 4 铁律承偌: 默认出声 (env 未设/silent 以外)。
    /// 本测试只验证 is_legacy_v17_5 谓词本身不漏;
    /// env 控制可见性逻辑 (OnceLock 缓存 env var) 在 push_governor_inner
    /// 同步单步跳邡, 完整 audit 路径靠 monitor --test smoke (Commit 4).
    #[test]
    fn is_legacy_v17_5_count_matches_v17_5_spec_section_2_2() {
        // v17.5 §2.2 (2026-07-16 勘误后): OptimalClose/VolumeWatchlist/VolumeRealTrade
        // 已经过调用链审计确认删除; 剩余 4 项 legacy 标记
        // (AuctionRepush + CandidateTriggered + CandidateInvalidated + VirtualWatch)
        let all_legacy_hits: Vec<PushKind> = [
            PushKind::AuctionRepush,
            PushKind::CandidateTriggered,
            PushKind::CandidateInvalidated,
            PushKind::VirtualWatch,
        ]
        .into_iter()
        .filter(|k| k.is_legacy_v17_5())
        .collect();
        assert_eq!(all_legacy_hits.len(), 4);
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
        // v17.8: 6 个 (交易类: 盘后固定价 + ST 涨幅 + ETF 收盘竞价 + 大宗交易)
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
            "v17.7+v17.8 spec targets 应包含 12 个"
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
        // v17.5 4 个 legacy (审计后)
        for k in [
            PushKind::AuctionRepush,
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
        for k in [
            PushKind::FactorIC,
            PushKind::SectorTier,
            PushKind::CapitalVerify,
        ] {
            assert!(
                !k.is_active_spec_target_v17_7_v17_8(),
                "{:?} low-priority 不应再标 active spec target",
                k
            );
        }
    }

    // ============== v17.6 §5.1: daily_report_sub_kind 标 3 variants ==============

    #[test]
    fn daily_report_sub_kind_marks_three_low_priority_variants() {
        let mappings = [
            (PushKind::FactorIC, DailyReportSubKind::FactorIC),
            (PushKind::SectorTier, DailyReportSubKind::SectorTier),
            (PushKind::CapitalVerify, DailyReportSubKind::CapitalVerify),
        ];
        assert_eq!(mappings.len(), 3);
        for (kind, expected_sub) in mappings {
            assert_eq!(
                kind.daily_report_sub_kind(),
                Some(expected_sub),
                "{:?} 应映射到 sub_kind {:?}",
                kind,
                expected_sub
            );
        }
    }

    /// v17.6 §5.1: 非 low-priority variants 不应被标 sub_kind (向后兼容)
    #[test]
    fn daily_report_sub_kind_none_for_other_variants() {
        for k in [
            PushKind::DailyReport,
            PushKind::HoldingEvent,
            PushKind::Announcement,
            PushKind::AuctionVolume,
            PushKind::LimitBoards,
            PushKind::SectorTop,
            PushKind::FundInflow,
            PushKind::HoldingPlan,
            PushKind::AccountMode,
        ] {
            assert!(
                k.daily_report_sub_kind().is_none(),
                "{:?} 不应是 DailyReport sub_kind",
                k
            );
        }
    }

    // ============== v17.x: DISPATCH_TABLE 15 rows 完整性 ==============

    #[test]
    fn dispatch_table_size_is_fifteen() {
        assert_eq!(
            DISPATCH_TABLE.len(),
            15,
            "v17.x DISPATCH_TABLE 应 15 rows (3 v17.6 + 6 v17.7 + 6 v17.8)"
        );
    }

    #[test]
    fn dispatch_table_all_unique_kinds() {
        let mut kinds: Vec<PushKind> = DISPATCH_TABLE.iter().map(|(k, _)| *k).collect();
        let total = kinds.len();
        kinds.sort_by_key(|k| format!("{:?}", k));
        kinds.dedup();
        assert_eq!(kinds.len(), total, "DISPATCH_TABLE kinds 必须唯一");
    }

    #[test]
    fn dispatch_table_covers_all_audit_marked() {
        // v17.6 low-priority 3 + v17.7 6 + v17.8 6 = 15
        let expected: Vec<PushKind> = vec![
            PushKind::FactorIC,
            PushKind::SectorTier,
            PushKind::CapitalVerify,
            PushKind::Announcement,
            PushKind::PolicyHit,
            PushKind::EarningsBeat,
            PushKind::EarningsMiss,
            PushKind::AnalystUpgrade,
            PushKind::MarketActionAlert,
            PushKind::PostFixedPriceOrder,
            PushKind::PostFixedPriceFill,
            PushKind::StPriceLimitChanged,
            PushKind::EtfClosingCallAuction,
            PushKind::BlockTradeIntradayConfirm,
            PushKind::BlockTradePriceRange,
        ];
        assert_eq!(expected.len(), 15);
        for k in expected {
            assert!(k.dispatch_row().is_some(), "{:?} 应在 DISPATCH_TABLE 内", k);
        }
    }

    #[test]
    fn dispatch_table_row_matches_existing_match_methods() {
        // spot-check: 表内值跟现有 match 块一致 (audit 验证)
        let factoric = PushKind::FactorIC.dispatch_row().unwrap();
        assert_eq!(factoric.level, PushKind::FactorIC.level());
        assert_eq!(factoric.cooldown_secs, PushKind::FactorIC.cooldown_secs());
        assert_eq!(factoric.cooldown_scope, PushKind::FactorIC.cooldown_scope());
        assert_eq!(factoric.label, PushKind::FactorIC.label());

        let announcement = PushKind::Announcement.dispatch_row().unwrap();
        assert_eq!(announcement.level, PushKind::Announcement.level());
        assert_eq!(
            announcement.cooldown_secs,
            PushKind::Announcement.cooldown_secs()
        );
        assert_eq!(
            announcement.cooldown_scope,
            PushKind::Announcement.cooldown_scope()
        );
        assert_eq!(announcement.label, PushKind::Announcement.label());

        let market_alert = PushKind::MarketActionAlert.dispatch_row().unwrap();
        assert_eq!(market_alert.level, PushLevel::Emergency);
        assert_eq!(market_alert.cooldown_secs, Some(60));
    }

    #[test]
    fn dispatch_table_label_no_collision() {
        let labels: Vec<&str> = DISPATCH_TABLE.iter().map(|(_, r)| r.label).collect();
        let mut sorted = labels.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), labels.len(), "DISPATCH_TABLE label 应唯一");
    }

    #[test]
    fn dispatch_table_stable_id_format_v1_suffix() {
        for (kind, row) in DISPATCH_TABLE.iter() {
            assert!(
                row.stable_template_id.ends_with("_v1"),
                "{:?} stable_template_id {:?} 应以 _v1 结尾",
                kind,
                row.stable_template_id
            );
        }
    }

    #[test]
    fn dispatch_table_non_audit_kind_returns_none() {
        // 现有 5 个 match 块覆盖的 kind (不在 v17.x audit 列表内) 应返回 None
        for k in [
            PushKind::DailyReport,
            PushKind::HoldingEvent,
            PushKind::AuctionVolume,
            PushKind::HoldingPlan,
            PushKind::AccountMode,
        ] {
            assert!(
                k.dispatch_row().is_none(),
                "{:?} 不在 DISPATCH_TABLE 内 (走原 match 块)",
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
        let r1 = push_governor_v3("first", PushKind::HoldingPlan, Some("TEST_CODE_000001")).await;
        if r1.is_pushed() {
            let r2 =
                push_governor_v3("second", PushKind::HoldingPlan, Some("TEST_CODE_000001")).await;
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
        let r1 = push_governor_v3("first", PushKind::HoldingPlan, Some("TEST_CODE_000001")).await;
        if r1.is_pushed() {
            let r2 =
                push_governor_v3("second", PushKind::HoldingPlan, Some("TEST_CODE_000002")).await;
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

    fn notify_temp_dir(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "stock_analysis_notify_{label}_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("create isolated notify directory");
        path
    }

    #[tokio::test]
    #[serial_test::serial(http_proxy_env)]
    async fn send_type_transport_and_target_resolution_are_explicit() {
        let _env = crate::TestEnvGuard::capture(&[
            "MAGICLAW_SEND_TYPE",
            "SEND_TYPE",
            "FEISHU_WEBHOOK_URL",
            "MAGICLAW_FEISHU_WEBHOOK_URL",
            "MAGICLAW_BIN",
            "MAGICLAW_HOME",
            "MAGICLAW_API_ADDR",
            "FEISHU_TO",
            "MAGICLAW_FEISHU_TO",
            "FEISHU_CHAT_ID",
            "FEISHU_OPEN_ID",
            "FEISHU_USER_ID",
            "FEISHU_EMAIL",
        ]);
        for key in [
            "MAGICLAW_SEND_TYPE",
            "SEND_TYPE",
            "FEISHU_WEBHOOK_URL",
            "MAGICLAW_FEISHU_WEBHOOK_URL",
            "FEISHU_TO",
            "MAGICLAW_FEISHU_TO",
            "FEISHU_CHAT_ID",
            "FEISHU_OPEN_ID",
            "FEISHU_USER_ID",
            "FEISHU_EMAIL",
        ] {
            std::env::remove_var(key);
        }

        assert!(matches!(resolve_send_type(), MessageSendType::Feishu));
        std::env::set_var("SEND_TYPE", " wx ");
        assert!(matches!(resolve_send_type(), MessageSendType::Wechat));
        std::env::set_var("MAGICLAW_SEND_TYPE", "unknown");
        assert!(matches!(resolve_send_type(), MessageSendType::Feishu));

        assert!(matches!(
            resolve_send_transport(MessageSendType::Wechat),
            MessageSendTransport::Http
        ));
        assert!(matches!(
            resolve_send_transport(MessageSendType::Feishu),
            MessageSendTransport::Cli
        ));
        std::env::set_var("FEISHU_WEBHOOK_URL", " https://example.invalid/hook ");
        assert_eq!(
            resolve_feishu_webhook_url().as_deref(),
            Some("https://example.invalid/hook")
        );
        assert!(matches!(
            resolve_send_transport(MessageSendType::Feishu),
            MessageSendTransport::Http
        ));

        std::env::set_var("MAGICLAW_BIN", "/TEST_CODE/bin/magiclaw");
        assert_eq!(resolve_magiclaw_bin(), "/TEST_CODE/bin/magiclaw");
        std::env::set_var("MAGICLAW_HOME", "/TEST_CODE/home");
        assert_eq!(
            resolve_magiclaw_home("/ignored/target/release/magiclaw").unwrap(),
            std::path::PathBuf::from("/TEST_CODE/home")
        );
        std::env::set_var("MAGICLAW_API_ADDR", " 127.0.0.1:9999 ");
        assert_eq!(resolve_api_addr(), "127.0.0.1:9999");

        let client = reqwest::Client::new();
        assert!(resolve_send_target(
            MessageSendType::Feishu,
            &client,
            "http://127.0.0.1:1",
            "TEST_CODE_token"
        )
        .await
        .is_err());
        std::env::set_var("FEISHU_TO", " TEST_CODE_chat ");
        assert_eq!(resolve_feishu_target().as_deref(), Some("TEST_CODE_chat"));
        assert_eq!(
            resolve_send_target(
                MessageSendType::Feishu,
                &client,
                "http://127.0.0.1:1",
                "TEST_CODE_token"
            )
            .await
            .unwrap()
            .as_deref(),
            Some("TEST_CODE_chat")
        );
    }

    #[tokio::test]
    #[serial_test::serial(notify_env)]
    async fn dynamic_token_parsing_and_caches_preserve_expiry_and_permissions() {
        let _env = crate::TestEnvGuard::capture(&[
            "DATABASE_PATH",
            "MAGICLAW_API_TOKEN",
            "MAGICLAW_TOKEN_REFRESH_AHEAD_SECS",
        ]);
        let dir = notify_temp_dir("token");
        let database = dir.join("TEST_CODE.db");
        std::env::set_var("DATABASE_PATH", &database);
        std::env::set_var("MAGICLAW_TOKEN_REFRESH_AHEAD_SECS", "0");
        clear_dynamic_token_cache().await;

        let future = now_epoch_secs() + 3_600;
        let parsed = parse_issue_token_output(&format!(
            "issued\ntoken=TEST_CODE_dynamic\nscopes=send expires_at={future}"
        ))
        .unwrap();
        assert_eq!(parsed.token, "TEST_CODE_dynamic");
        assert_eq!(parsed.expires_at, Some(future));
        assert!(!is_cached_token_expired(&parsed));
        assert!(parse_issue_token_output("expires_at=1").is_err());

        let expired = CachedApiToken {
            token: "TEST_CODE_expired".to_string(),
            expires_at: Some(now_epoch_secs() - 1),
        };
        assert!(is_cached_token_expired(&expired));
        cache_dynamic_token_in_mem(&expired).await;
        assert!(load_dynamic_token_from_mem_cache().await.is_none());

        cache_dynamic_token_in_file(&parsed).unwrap();
        assert_eq!(
            api_token_cache_file_path(),
            dir.join("magiclaw_api_token_cache.json")
        );
        assert_eq!(
            load_dynamic_token_from_file_cache().unwrap().token,
            "TEST_CODE_dynamic"
        );
        cache_dynamic_token_in_mem(&parsed).await;
        assert_eq!(
            load_dynamic_token_from_mem_cache().await.unwrap().token,
            "TEST_CODE_dynamic"
        );

        std::env::set_var("MAGICLAW_API_TOKEN", " TEST_CODE_env_token ");
        MAGICLAW_DISABLE_ENV_TOKEN.store(false, Ordering::Relaxed);
        let (token, source) = resolve_or_issue_api_token("/does/not/run").await.unwrap();
        assert_eq!(token, "TEST_CODE_env_token");
        assert!(matches!(source, ApiTokenSource::Env));
        assert!(is_unauthorized_error("HTTP 401"));
        assert!(is_unauthorized_error("Unauthorized"));
        assert!(!is_unauthorized_error("timeout"));

        clear_dynamic_token_cache().await;
        assert!(!api_token_cache_file_path().exists());
        std::fs::remove_dir_all(dir).expect("remove isolated token directory");
    }

    #[test]
    #[serial_test::serial(notify_env)]
    fn local_target_and_log_parsers_never_invent_recipient_identity() {
        let _env = crate::TestEnvGuard::capture(&[
            "MAGICLAW_DB_PATH",
            "DATABASE_PATH",
            "WECHAT_CHANNEL_DIR",
        ]);
        let dir = notify_temp_dir("logs");
        let database = dir.join("TEST_CODE.db");
        std::env::set_var("MAGICLAW_DB_PATH", &database);
        std::env::set_var("WECHAT_CHANNEL_DIR", dir.join("wechat"));

        assert_eq!(to_api_base_url("127.0.0.1:8080"), "http://127.0.0.1:8080");
        assert_eq!(
            to_api_base_url("https://example.invalid/"),
            "https://example.invalid"
        );
        assert_eq!(
            parse_first_peer_id_from_window_status(
                r#"{"peers":[{"peer_id":" "},{"peer_id":"TEST_CODE_peer"}]}"#
            )
            .as_deref(),
            Some("TEST_CODE_peer")
        );
        assert!(parse_first_peer_id_from_window_status("not-json").is_none());
        assert!(parse_first_peer_id_from_window_status(r#"{"peers":[]}"#).is_none());

        let log_dir = resolve_magiclaw_log_dir();
        std::fs::create_dir_all(&log_dir).unwrap();
        std::fs::write(log_dir.join("ignored.txt"), "peer_id=WRONG").unwrap();
        std::fs::write(
            log_dir.join("magiclaw-20260718.log"),
            "older peer_id=TEST_CODE_old\nnew peer_id=TEST_CODE_latest state=ready\n",
        )
        .unwrap();
        assert_eq!(
            resolve_wechat_target_from_magiclaw_logs().as_deref(),
            Some("TEST_CODE_latest")
        );
        assert_eq!(resolve_wechat_data_dir(), dir.join("wechat"));
        assert_eq!(tail_lines("one\ntwo\nthree", 2), "two | three");
        std::fs::remove_dir_all(dir).expect("remove isolated log directory");
    }

    async fn one_request_http_fixture(
        status: u16,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback HTTP fixture");
        let addr = listener.local_addr().expect("fixture local addr");
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept fixture request");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let n = stream.read(&mut chunk).await.expect("read fixture request");
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..n]);
                let Some(header_end) = request.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let header_end = header_end + 4;
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_len = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + content_len {
                    break;
                }
            }
            let reason = match status {
                200 => "OK",
                401 => "Unauthorized",
                412 => "Precondition Failed",
                500 => "Internal Server Error",
                _ => "Fixture",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write fixture response");
            String::from_utf8_lossy(&request).into_owned()
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn feishu_webhook_executes_success_http_error_and_protocol_error() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let (url, request) = one_request_http_fixture(200, r#"{"code":0}"#).await;
        assert!(push_feishu_http_with_client(&client, &url, "TEST_CODE webhook success").await);
        let request = request.await.unwrap();
        assert!(request.starts_with("POST / HTTP/1.1"));
        assert!(request.contains("TEST_CODE webhook success"));

        let (url, request) = one_request_http_fixture(500, r#"{"error":"down"}"#).await;
        assert!(!push_feishu_http_with_client(&client, &url, "TEST_CODE webhook status").await);
        request.await.unwrap();

        let (url, request) = one_request_http_fixture(200, r#"{"code":1}"#).await;
        assert!(!push_feishu_http_with_client(&client, &url, "TEST_CODE webhook protocol").await);
        request.await.unwrap();
    }

    #[test]
    fn magiclaw_cli_success_requires_a_real_channel_receipt() {
        let receipt = parse_magiclaw_cli_delivery_receipt(
            MessageSendType::Feishu,
            "send ok (feishu): message_id=receipt-1, platform_msg_id=om_platform_1\n",
        )
        .expect("explicit Feishu receipt");
        assert_eq!(receipt.message_id, "receipt-1");
        assert_eq!(receipt.platform_msg_id, "om_platform_1");

        for stdout in [
            "",
            "send completed",
            "send ok (feishu): message_id=receipt-1, platform_msg_id=<none>",
            "send ok (feishu): message_id=<daemon>, platform_msg_id=om_platform_1",
            "send ok (via daemon): message_id=<daemon>, to=TEST_CODE_target",
        ] {
            assert!(
                parse_magiclaw_cli_delivery_receipt(MessageSendType::Feishu, stdout).is_err(),
                "exit-zero stdout without a real Feishu receipt must fail: {stdout}"
            );
        }
    }

    #[tokio::test]
    #[serial_test::serial(notify_env)]
    async fn daemon_protocol_executes_health_target_send_and_auth_outcomes() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let (base, request) = one_request_http_fixture(200, r#"{"ok":true}"#).await;
        assert!(daemon_health_ok(&client, &base).await);
        assert!(request.await.unwrap().starts_with("GET /api/health"));

        let (base, request) = one_request_http_fixture(200, r#"{"ok":true}"#).await;
        assert!(matches!(
            ensure_magiclaw_daemon(&client, "/TEST_CODE/not-used", "127.0.0.1:1", &base)
                .await
                .unwrap(),
            DaemonReadySource::Reused
        ));
        request.await.unwrap();

        let (base, request) =
            one_request_http_fixture(200, r#"{"peers":[{"peer_id":"TEST_CODE_window_peer"}]}"#)
                .await;
        std::env::remove_var("WECHAT_TO");
        assert_eq!(
            resolve_wechat_target(&client, &base, "TEST_CODE_token")
                .await
                .unwrap(),
            "TEST_CODE_window_peer"
        );
        assert!(request
            .await
            .unwrap()
            .contains("authorization: Bearer TEST_CODE_token"));

        for (status, body, expected) in [
            (200, r#"{"ok":true}"#, None),
            (200, r#"{"ok":false}"#, Some("非成功体")),
            (401, r#"{"error":"bad token"}"#, Some("鉴权失败")),
            (
                412,
                r#"{"error":"no valid context_token for peer"}"#,
                Some("context_token 无效"),
            ),
            (500, r#"{"error":"down"}"#, Some("HTTP 500")),
        ] {
            let (base, request) = one_request_http_fixture(status, body).await;
            let result = send_via_magiclaw_daemon(
                &client,
                &base,
                "TEST_CODE_token",
                MessageSendType::Wechat,
                Some(" TEST_CODE_peer "),
                "TEST_CODE message",
            )
            .await;
            match expected {
                None => assert!(result.is_ok(), "{result:?}"),
                Some(fragment) => assert!(result.unwrap_err().contains(fragment)),
            }
            let request = request.await.unwrap();
            assert!(request.starts_with("POST /api/send"));
            assert!(request.contains("TEST_CODE_peer"));
        }

        let (base, request) = one_request_http_fixture(200, r#"{"ok":true}"#).await;
        assert!(
            verify_daemon_auth(&client, &base, "TEST_CODE_token", &ApiTokenSource::Env)
                .await
                .is_ok()
        );
        request.await.unwrap();

        let (base, request) = one_request_http_fixture(401, r#"{"error":"unauthorized"}"#).await;
        let error = verify_daemon_auth(
            &client,
            &base,
            "TEST_CODE_token",
            &ApiTokenSource::DynamicIssued,
        )
        .await
        .unwrap_err();
        assert!(error.contains("动态 token"));
        request.await.unwrap();

        let (base, request) = one_request_http_fixture(500, r#"{"error":"down"}"#).await;
        let error = verify_daemon_auth(&client, &base, "TEST_CODE_token", &ApiTokenSource::Env)
            .await
            .unwrap_err();
        assert!(error.contains("HTTP 500"));
        request.await.unwrap();
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br137_sink_success_with_post_delivery_audit_failure_releases_identity_for_retry() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let now = chrono::Local::now();
        let evidence = crate::v14_adapter::SourceFactEvidence::new(
            PushKind::Announcement,
            "TEST_CODE_POST_AUDIT_RETRY_ID".to_string(),
            Some("TEST_CODE_POST_AUDIT_RETRY".to_string()),
            "后置审计失败后允许重试".to_string(),
            "TEST_CODE_PROVIDER".to_string(),
            now,
            Some(now.date_naive()),
            80,
            90,
            false,
        )
        .expect("complete source fact");
        let first = match crate::v14_adapter::v14_gate_source_fact(&evidence) {
            crate::v14_adapter::V14Gate::Approved(event) => *event,
            other => panic!("first attempt must reserve: {other:?}"),
        };

        settle_dedup_after_delivery(&first, PushKind::Announcement, None, None, true, false)
            .expect("failed post-delivery audit must roll back L4 identity");

        let retry = match crate::v14_adapter::v14_gate_source_fact(&evidence) {
            crate::v14_adapter::V14Gate::Approved(event) => *event,
            other => panic!("audit failure must leave the source fact retryable: {other:?}"),
        };
        crate::v14_adapter::rollback_dedup_for_event(&retry, PushKind::Announcement, None, None)
            .expect("test cleanup rollback");
    }
}
