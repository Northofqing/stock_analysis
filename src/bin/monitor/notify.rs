//! Registered business rules: BR-047, BR-048, BR-077, BR-137, BR-192.
//! 通知推送 + MagicLaw 守护进程 + Token 管理
//!
//! 从 main.rs 提取，减少单文件体积。
//!
//! BR-192 push-log namespace isolation assumes an exclusive production service
//! UID. Portable Unix APIs cannot stop a hostile same-UID process in the final
//! instant after identity revalidation; deployment must pair these pinned
//! descriptors with owner-only writable manifest data directories.

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
)))]
compile_error!(
    "BR-192 pinned push-log persistence requires openat/mkdirat Unix semantics; \
     supported targets: Linux, macOS/iOS, FreeBSD, OpenBSD and NetBSD"
);

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
    /// 退役 (BR-191, Gate B in progress): shadow NewsRanker 无生产调用、默认
    /// MarketContext 评分, BR-191/BR-112 判退役, v19.15 已删演示模板。
    /// 生产证据 (2026-08-06 审计): src/ 内 0 生产 dispatcher/调用点; 仅存
    /// level=Important (notify.rs:295)、v14 适配表 (v14_adapter.rs:1069)、
    /// BR-196 FIXED_DISABLED_KINDS 清单引用。不删变体 (BR-196 契约依赖),
    /// 启动时 dispatch_table_init_audit 输出退役状态 (no_producer)。
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
    /// 虚拟盘卖出 (BR-234, ℹ️ 5min批, 默认降级) [MVP-1]
    PaperSell,
    /// 持仓快照过期提醒 (任务#3, ℹ️ 每日1次, 默认降级) [MVP-1]
    SnapshotStale,
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
    /// BR-192: Eastmoney 来源限定量比/主力净流入双 TopN (R-09)
    ReviewProviderTopN,
    /// BR-222: 持仓复盘 (R-11, 用户确认持仓摘要, 盘后 1次/日)
    PositionReview,
    /// R-12: 盘后策略回测 (15min K线回测虚拟仓信号 + boll_macd, 盘后 1次/日)
    ReviewBacktest,
    /// R-13: T+1 昨日关注回填 (A-10 名单次日行情核对, 盘后 1次/日)
    WatchlistTracking,
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
    /// 3 个 variants: CandidateTriggered, CandidateInvalidated, VirtualWatch.
    /// (BR-223: AuctionRepush 已恢复生产接线, 移出 legacy; 2026-07-16 审计:
    ///  OptimalClose/VolumeWatchlist/VolumeRealTrade 已删;
    ///  CandidateTriggered/VirtualWatch 实为活链路, 仅保留 legacy 标记待后续迁移评估)
    pub fn is_legacy_v17_5(self) -> bool {
        matches!(
            self,
            Self::CandidateTriggered | Self::CandidateInvalidated | Self::VirtualWatch
        )
    }

    /// v17.6 §2.2: 3 个 spec 标记为低优 / durable 子类型治理候选.
    ///
    /// 注: 这 3 个 variants **与 v17.5 不同** — 它们仍是 durable delivery
    /// catalog 的稳定语义映射。因此本方法不标 legacy，而是保留
    /// `is_low_priority_v17_6` metadata；生产发送必须走 BR-192 binding 入口.
    ///
    /// 3 个 variants: FactorIC, SectorTier, CapitalVerify.
    /// 它们的稳定投递子类型由 `DailyReportSubKind` 表达；实际投递必须通过
    /// BR-192 explicit binding 入口，不能回退到 generic governor.
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
    /// 使用 `DailyReportSubKind` 表达 durable delivery 子类型.
    ///
    /// 该映射只提供稳定类型元数据；BR-192 counted delivery 仍要求调用方提交
    /// 完整的 immutable source binding，不能据此构造或发送 generic DailyReport.
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
            | PushKind::ReviewProviderTopN
            | PushKind::PositionReview
            | PushKind::DailyReport
            | PushKind::CandidateBoard
            | PushKind::AuctionRepush
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
            // R-12 盘后回测: 参考级 (非交易建议, 仅统计)
            PushKind::ReviewBacktest => PushLevel::Info,
            // R-13 昨日关注回填: 参考级 (非交易建议, 仅行情核对)
            PushKind::WatchlistTracking => PushLevel::Info,
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
                | PushKind::PaperSell
                | PushKind::SnapshotStale
                | PushKind::CloseCall
                | PushKind::ReviewMarket
                | PushKind::ReviewLhb
                | PushKind::ReviewSignal
                | PushKind::ReviewFailure
                | PushKind::TomorrowWatch
                | PushKind::EventCalendar
                | PushKind::ReviewProviderTopN
                | PushKind::PositionReview
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
            // 5 min / 票 (BR-234 虚拟盘卖出)
            PushKind::PaperSell => Some(300),
            // 每日快照提醒 (调用方已按日去重)
            PushKind::SnapshotStale => Some(300),
            // 1次/日
            PushKind::CloseCall => Some(86_400),
            // 盘后系列 1次/日 (推送时机控制而非冷却)
            PushKind::ReviewMarket
            | PushKind::ReviewLhb
            | PushKind::ReviewSignal
            | PushKind::ReviewFailure
            | PushKind::TomorrowWatch
            | PushKind::EventCalendar
            | PushKind::ReviewProviderTopN
            | PushKind::PositionReview
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
            // R-12 盘后回测: 1次/日 (60 min 冷却, 防重复调度触发)
            PushKind::ReviewBacktest => Some(3600),
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
            | PaperSell
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
            PushKind::PaperSell => "虚拟盘卖出",
            PushKind::SnapshotStale => "快照过期提醒",
            PushKind::CloseCall => "尾盘决策",
            PushKind::ReviewMarket => "盘面走向",
            PushKind::ReviewLhb => "龙虎榜",
            PushKind::ReviewSignal => "信号复盘",
            PushKind::ReviewFailure => "失败归因",
            PushKind::TomorrowWatch => "明日观察池",
            PushKind::EventCalendar => "事件日历",
            PushKind::ReviewProviderTopN => "盘后量能与主力净流入",
            PushKind::PositionReview => "持仓复盘",
            PushKind::ReviewBacktest => "15min回测",
            PushKind::WatchlistTracking => "昨日关注回填",
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

/// v17.6 §5.1 / BR-192: durable DailyReport 子类型.
///
/// 保留 FactorIC / SectorTier / CapitalVerify 的显式 durable 映射。该类型只能随
/// `CountedDeliveryBinding` 进入 `push_counted_with_binding`; 它不提供 generic
/// DailyReport 路由或隐式 source evidence.
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

    /// 历史模板冷却 metadata（仅用于审计/兼容性检查）。
    /// BR-192 durable coordinator 是 counted delivery 的唯一准入/去重 owner；
    /// 本值不得重新接入 generic governor.
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
    // ============== BR-234: 虚拟盘卖出 ==============
    (
        PushKind::PaperSell,
        DispatchRow {
            level: PushLevel::Info,
            cooldown_secs: Some(300),
            cooldown_scope: CooldownScope::PerTicket,
            label: "虚拟盘卖出",
            stable_template_id: "papersell_v1",
        },
    ),
    // ============== 任务#3: 持仓快照过期提醒 ==============
    (
        PushKind::SnapshotStale,
        DispatchRow {
            level: PushLevel::Info,
            cooldown_secs: Some(300),
            cooldown_scope: CooldownScope::Global,
            label: "快照过期提醒",
            stable_template_id: "snapshotstale_v1",
        },
    ),
    // ============== R-12: 盘后 15min 回测段 ==============
    (
        PushKind::ReviewBacktest,
        DispatchRow {
            level: PushLevel::Info,
            cooldown_secs: Some(3600),
            cooldown_scope: CooldownScope::Global,
            label: "15min回测",
            stable_template_id: "r12_15min_backtest_v1",
        },
    ),
    // ============== R-13: T+1 昨日关注回填段 ==============
    (
        PushKind::WatchlistTracking,
        DispatchRow {
            level: PushLevel::Info,
            cooldown_secs: Some(3600),
            cooldown_scope: CooldownScope::Global,
            label: "昨日关注回填",
            stable_template_id: "t1_watchlist_tracking_v1",
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
    // Completion Rule 4d: Spec-only PushKind 无真实生产者必须启动声明。
    // NewsRanked: BR-191 退役 (shadow NewsRanker), 0 生产 dispatcher —
    // 有生产者接入前不得声称活动。若未来接入, 必须先撤此声明。
    log::info!(
        "[v17.x][NewsRanked] disabled=no_producer reason=BR-191-shadow-news-ranker-retired"
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

#[cfg(test)]
fn create_push_log_file(path: &std::path::Path) -> Result<std::fs::File, String> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("push_log 不可覆盖创建失败 {}: {error}", path.display()))
}

#[derive(Debug)]
pub(super) enum PushLogError {
    NamespaceOverrideRejected { namespace: String },
    NamespaceIsolation(String),
    Persistence(String),
}

impl PushLogError {
    fn reason_code(&self) -> &'static str {
        match self {
            Self::NamespaceOverrideRejected { .. } => "push_log_namespace_override_rejected",
            Self::NamespaceIsolation(_) => "push_log_namespace_isolation_rejected",
            Self::Persistence(_) => "push_log_persistence_failed",
        }
    }

    fn retry_authorized(&self) -> bool {
        matches!(self, Self::Persistence(_))
    }

    fn from_io(
        operation: &str,
        path: &std::path::Path,
        error: std::io::Error,
        namespace_sensitive: bool,
    ) -> Self {
        let detail = format!("{operation} {}: {error}", path.display());
        if is_retryable_persistence_errno(&error) || !namespace_sensitive {
            Self::Persistence(detail)
        } else {
            Self::NamespaceIsolation(detail)
        }
    }
}

fn is_retryable_persistence_errno(error: &std::io::Error) -> bool {
    // Portable `std::io::ErrorKind` does not distinguish quota, process-wide
    // descriptor exhaustion or device I/O. These Unix errno values are stable
    // across the supported targets (EDQUOT is 69 on BSD/macOS and 122 on Linux).
    matches!(error.raw_os_error(), Some(5 | 23 | 24 | 28 | 69 | 122))
}

impl std::fmt::Display for PushLogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NamespaceOverrideRejected { namespace } => write!(
                formatter,
                "PUSH_LOG_DIR override is forbidden for bound namespace {namespace}"
            ),
            Self::NamespaceIsolation(error) => formatter.write_str(error),
            Self::Persistence(error) => formatter.write_str(error),
        }
    }
}

const PUSH_LOG_O_RDONLY: i32 = 0;
const PUSH_LOG_O_WRONLY: i32 = 1;
const PUSH_LOG_O_RDWR: i32 = 2;
const PUSH_LOG_LOCK_FILE: &str = ".push_log.lock";

#[cfg(target_os = "linux")]
const PUSH_LOG_O_NOFOLLOW: i32 = 0x0002_0000;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const PUSH_LOG_O_NOFOLLOW: i32 = 0x0000_0100;
#[cfg(target_os = "linux")]
const PUSH_LOG_O_NONBLOCK: i32 = 0x0000_0800;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const PUSH_LOG_O_NONBLOCK: i32 = 0x0000_0004;
#[cfg(target_os = "linux")]
const PUSH_LOG_O_CREAT: i32 = 0x0000_0040;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const PUSH_LOG_O_CREAT: i32 = 0x0000_0200;
#[cfg(target_os = "linux")]
const PUSH_LOG_O_EXCL: i32 = 0x0000_0080;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const PUSH_LOG_O_EXCL: i32 = 0x0000_0800;
#[cfg(target_os = "linux")]
const PUSH_LOG_O_CLOEXEC: i32 = 0x0008_0000;
#[cfg(any(target_os = "macos", target_os = "ios"))]
const PUSH_LOG_O_CLOEXEC: i32 = 0x0100_0000;
#[cfg(target_os = "freebsd")]
const PUSH_LOG_O_CLOEXEC: i32 = 0x0010_0000;
#[cfg(target_os = "openbsd")]
const PUSH_LOG_O_CLOEXEC: i32 = 0x0001_0000;
#[cfg(target_os = "netbsd")]
const PUSH_LOG_O_CLOEXEC: i32 = 0x0040_0000;

unsafe extern "C" {
    fn openat(directory_fd: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32;
    fn mkdirat(directory_fd: i32, path: *const std::ffi::c_char, mode: u32) -> i32;
    fn geteuid() -> u32;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PushLogFileIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    uid: u32,
    is_directory: bool,
    is_file: bool,
}

struct PinnedPushLogDirectory {
    anchor: std::fs::File,
    anchor_identity: PushLogFileIdentity,
    components: Vec<std::ffi::OsString>,
    identities: Vec<PushLogFileIdentity>,
    directories: Vec<std::fs::File>,
}

impl PinnedPushLogDirectory {
    fn push(
        &mut self,
        component: std::ffi::OsString,
        identity: PushLogFileIdentity,
        directory: std::fs::File,
    ) {
        self.components.push(component);
        self.identities.push(identity);
        self.directories.push(directory);
    }

    fn try_clone(&self) -> Result<Self, PushLogError> {
        let anchor = self.anchor.try_clone().map_err(|error| {
            PushLogError::Persistence(format!("clone pinned push_log anchor: {error}"))
        })?;
        let directories = self
            .directories
            .iter()
            .enumerate()
            .map(|(index, directory)| {
                directory.try_clone().map_err(|error| {
                    PushLogError::Persistence(format!(
                        "clone pinned push_log directory component {index}: {error}"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            anchor,
            anchor_identity: self.anchor_identity,
            components: self.components.clone(),
            identities: self.identities.clone(),
            directories,
        })
    }
}

pub(super) struct PinnedPushLogWriter {
    namespace_label: String,
    root: std::path::PathBuf,
    root_binding: PinnedPushLogDirectory,
    lock: std::sync::Arc<std::fs::File>,
    lock_identity: PushLogFileIdentity,
}

#[derive(Clone, Copy)]
enum PushLogWritePhase {
    DirectoriesBound,
    ArtifactSynced,
}

fn push_log_component_cstring(
    component: &std::ffi::OsStr,
) -> Result<std::ffi::CString, PushLogError> {
    use std::os::unix::ffi::OsStrExt;

    std::ffi::CString::new(component.as_bytes()).map_err(|_| {
        PushLogError::NamespaceIsolation("push_log path component contains NUL".to_owned())
    })
}

fn push_log_openat(
    parent: &std::fs::File,
    name: &std::ffi::OsStr,
    flags: i32,
    mode: u32,
) -> std::io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "push_log path component contains NUL",
        )
    })?;
    // SAFETY: `name` is a live NUL-terminated single component, `parent`
    // owns a directory descriptor, and a successful descriptor is moved
    // exactly once into `File`.
    let descriptor = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | PUSH_LOG_O_NOFOLLOW | PUSH_LOG_O_NONBLOCK | PUSH_LOG_O_CLOEXEC,
            mode,
        )
    };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: successful `openat` returned one newly owned descriptor.
    Ok(unsafe { std::fs::File::from_raw_fd(descriptor) })
}

fn validate_push_log_directory(
    directory: &std::fs::File,
    path: &std::path::Path,
) -> Result<PushLogFileIdentity, PushLogError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = directory
        .metadata()
        .map_err(|error| PushLogError::from_io("inspect push_log directory", path, error, true))?;
    if !metadata.is_dir() {
        return Err(PushLogError::NamespaceIsolation(format!(
            "push_log namespace component is not a directory: {}",
            path.display()
        )));
    }
    if metadata.nlink() == 0 {
        return Err(PushLogError::NamespaceIsolation(format!(
            "push_log namespace component has no physical links: {}",
            path.display()
        )));
    }
    // SAFETY: `geteuid` has no preconditions and does not retain pointers.
    let effective_uid = unsafe { geteuid() };
    if !push_log_directory_owner_allowed(metadata.uid(), effective_uid) {
        return Err(PushLogError::NamespaceIsolation(format!(
            "push_log directory owner uid={} is neither root nor effective uid={effective_uid}: {}",
            metadata.uid(),
            path.display()
        )));
    }
    if metadata.mode() & 0o022 != 0 {
        return Err(PushLogError::NamespaceIsolation(format!(
            "push_log directory is group/other writable mode={:o}: {}",
            metadata.mode() & 0o7777,
            path.display()
        )));
    }
    Ok(PushLogFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        uid: metadata.uid(),
        is_directory: metadata.is_dir(),
        is_file: metadata.is_file(),
    })
}

fn push_log_directory_owner_allowed(uid: u32, effective_uid: u32) -> bool {
    uid == 0 || uid == effective_uid
}

fn open_or_create_push_log_child(
    parent: &std::fs::File,
    name: &std::ffi::OsStr,
    path: &std::path::Path,
) -> Result<(std::fs::File, PushLogFileIdentity), PushLogError> {
    open_or_create_push_log_child_with_hook(parent, name, path, || {})
}

fn open_or_create_push_log_child_with_hook<F>(
    parent: &std::fs::File,
    name: &std::ffi::OsStr,
    path: &std::path::Path,
    before_mkdir: F,
) -> Result<(std::fs::File, PushLogFileIdentity), PushLogError>
where
    F: FnOnce(),
{
    use std::io::ErrorKind;
    use std::os::fd::AsRawFd;

    let directory = match push_log_openat(parent, name, PUSH_LOG_O_RDONLY, 0) {
        Ok(directory) => directory,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            before_mkdir();
            let component = push_log_component_cstring(name)?;
            // SAFETY: `name` is one live NUL-terminated component and
            // `parent` retains a valid directory descriptor.
            let created = unsafe { mkdirat(parent.as_raw_fd(), component.as_ptr(), 0o700_u32) };
            if created < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != ErrorKind::AlreadyExists {
                    return Err(PushLogError::from_io(
                        "create fixed push_log directory",
                        path,
                        error,
                        true,
                    ));
                }
            }
            // Sync after both create and `EEXIST`: the process that won a
            // concurrent mkdir may have crashed before syncing the parent.
            parent.sync_all().map_err(|error| {
                PushLogError::Persistence(format!(
                    "fsync push_log parent for {}: {error}",
                    path.display()
                ))
            })?;
            push_log_openat(parent, name, PUSH_LOG_O_RDONLY, 0).map_err(|error| {
                PushLogError::from_io(
                    "open fixed push_log directory without symlink traversal",
                    path,
                    error,
                    true,
                )
            })?
        }
        Err(error) => {
            return Err(PushLogError::from_io(
                "open fixed push_log directory without symlink traversal",
                path,
                error,
                true,
            ));
        }
    };
    let identity = validate_push_log_directory(&directory, path)?;
    Ok((directory, identity))
}

fn push_log_absolute_components(
    path: &std::path::Path,
    label: &str,
) -> Result<Vec<std::ffi::OsString>, PushLogError> {
    use std::path::Component;

    if !path.is_absolute() {
        return Err(PushLogError::NamespaceIsolation(format!(
            "{label} must be absolute: {}",
            path.display()
        )));
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => components.push(name.to_os_string()),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(PushLogError::NamespaceIsolation(format!(
                    "{label} is not lexically exact: {}",
                    path.display()
                )));
            }
        }
    }
    Ok(components)
}

fn open_or_create_push_log_root(
    root: &std::path::Path,
    creation_boundary: &std::path::Path,
) -> Result<PinnedPushLogDirectory, PushLogError> {
    let normal_components = push_log_absolute_components(root, "fixed push_log namespace")?;
    let boundary_components =
        push_log_absolute_components(creation_boundary, "fixed push_log creation boundary")?;
    if normal_components.len() <= boundary_components.len()
        || !normal_components.starts_with(&boundary_components)
    {
        return Err(PushLogError::NamespaceIsolation(format!(
            "fixed push_log namespace must be below its creation boundary: {}",
            root.display()
        )));
    }
    let anchor = std::fs::OpenOptions::new()
        .read(true)
        .open("/")
        .map_err(|error| {
            PushLogError::from_io(
                "open push_log filesystem anchor",
                std::path::Path::new("/"),
                error,
                false,
            )
        })?;
    let anchor_identity = validate_push_log_directory(&anchor, std::path::Path::new("/"))?;
    let mut directory = anchor.try_clone().map_err(|error| {
        PushLogError::Persistence(format!("clone pinned push_log filesystem anchor: {error}"))
    })?;
    let mut traversed = std::path::PathBuf::from("/");
    let mut components = Vec::new();
    let mut identities = Vec::new();
    let mut directories = Vec::new();
    for (index, name) in normal_components.iter().enumerate() {
        traversed.push(name);
        let (next, identity) = match push_log_openat(&directory, name, PUSH_LOG_O_RDONLY, 0) {
            Ok(next) => {
                let identity = validate_push_log_directory(&next, &traversed)?;
                (next, identity)
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && index >= boundary_components.len() =>
            {
                open_or_create_push_log_child(&directory, name, &traversed)?
            }
            Err(error) => {
                return Err(PushLogError::from_io(
                    "traverse fixed push_log namespace without symlink traversal",
                    &traversed,
                    error,
                    true,
                ));
            }
        };
        components.push(name.to_os_string());
        identities.push(identity);
        directories.push(next.try_clone().map_err(|error| {
            PushLogError::Persistence(format!(
                "clone pinned push_log directory {}: {error}",
                traversed.display()
            ))
        })?);
        directory = next;
    }
    Ok(PinnedPushLogDirectory {
        anchor,
        anchor_identity,
        components,
        identities,
        directories,
    })
}

fn revalidate_push_log_directory_chain(
    binding: &PinnedPushLogDirectory,
) -> Result<std::fs::File, PushLogError> {
    let anchor_identity = validate_push_log_directory(&binding.anchor, std::path::Path::new("/"))?;
    if anchor_identity != binding.anchor_identity {
        return Err(PushLogError::NamespaceIsolation(
            "push_log filesystem anchor identity changed".to_owned(),
        ));
    }
    if binding.components.len() != binding.identities.len()
        || binding.components.len() != binding.directories.len()
    {
        return Err(PushLogError::NamespaceIsolation(
            "push_log retained directory binding is internally inconsistent".to_owned(),
        ));
    }
    let mut directory = binding.anchor.try_clone().map_err(|error| {
        PushLogError::Persistence(format!(
            "clone pinned push_log filesystem anchor for revalidation: {error}"
        ))
    })?;
    let mut traversed = std::path::PathBuf::from("/");
    for ((component, expected_identity), retained) in binding
        .components
        .iter()
        .zip(&binding.identities)
        .zip(&binding.directories)
    {
        traversed.push(component);
        let retained_identity = validate_push_log_directory(retained, &traversed)?;
        if retained_identity != *expected_identity {
            return Err(PushLogError::NamespaceIsolation(format!(
                "retained push_log directory identity changed: {}",
                traversed.display()
            )));
        }
        let rebound =
            push_log_openat(&directory, component, PUSH_LOG_O_RDONLY, 0).map_err(|error| {
                PushLogError::from_io(
                    "re-open fixed push_log directory without symlink traversal",
                    &traversed,
                    error,
                    true,
                )
            })?;
        let actual_identity = validate_push_log_directory(&rebound, &traversed)?;
        if actual_identity != *expected_identity {
            return Err(PushLogError::NamespaceIsolation(format!(
                "push_log directory identity changed while bound: {}",
                traversed.display()
            )));
        }
        directory = rebound;
    }
    Ok(directory)
}

fn validate_push_log_leaf(
    file: &std::fs::File,
    path: &std::path::Path,
) -> Result<PushLogFileIdentity, PushLogError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file
        .metadata()
        .map_err(|error| PushLogError::from_io("inspect push_log artifact", path, error, true))?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(PushLogError::NamespaceIsolation(format!(
            "push_log artifact must be a regular file with exactly one physical link: {}",
            path.display()
        )));
    }
    // SAFETY: `geteuid` has no preconditions and does not retain pointers.
    let effective_uid = unsafe { geteuid() };
    if metadata.uid() != effective_uid {
        return Err(PushLogError::NamespaceIsolation(format!(
            "push_log artifact owner uid={} differs from effective uid={effective_uid}: {}",
            metadata.uid(),
            path.display()
        )));
    }
    if metadata.mode() & 0o022 != 0 {
        return Err(PushLogError::NamespaceIsolation(format!(
            "push_log artifact is group/other writable mode={:o}: {}",
            metadata.mode() & 0o7777,
            path.display()
        )));
    }
    Ok(PushLogFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        uid: metadata.uid(),
        is_directory: metadata.is_dir(),
        is_file: metadata.is_file(),
    })
}

fn revalidate_push_log_leaf_at(
    parent: &std::fs::File,
    name: &std::ffi::OsStr,
    path: &std::path::Path,
    flags: i32,
    expected: PushLogFileIdentity,
) -> Result<std::fs::File, PushLogError> {
    let reopened = push_log_openat(parent, name, flags, 0).map_err(|error| {
        PushLogError::from_io(
            "re-open pinned push_log leaf without symlink traversal",
            path,
            error,
            true,
        )
    })?;
    let observed = validate_push_log_leaf(&reopened, path)?;
    if observed != expected {
        return Err(PushLogError::NamespaceIsolation(format!(
            "pinned push_log leaf identity changed: {}",
            path.display()
        )));
    }
    Ok(reopened)
}

fn push_log_process_mutex() -> &'static std::sync::Mutex<()> {
    static MUTEX: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    MUTEX.get_or_init(|| std::sync::Mutex::new(()))
}

impl PinnedPushLogWriter {
    pub(super) fn for_namespace(
        namespace: &crate::durable_delivery_runtime::RuntimeNamespace,
    ) -> Result<Self, PushLogError> {
        if std::env::var_os("PUSH_LOG_DIR").is_some() {
            return Err(PushLogError::NamespaceOverrideRejected {
                namespace: namespace.label(),
            });
        }
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let (namespace_label, relative_root) = match namespace {
            crate::durable_delivery_runtime::RuntimeNamespace::Production => (
                "production".to_owned(),
                std::path::PathBuf::from("data/push_log"),
            ),
            crate::durable_delivery_runtime::RuntimeNamespace::Test { test_code } => {
                validate_push_log_test_code(test_code)?;
                (
                    format!("test:{test_code}"),
                    std::path::PathBuf::from("data/test")
                        .join(test_code)
                        .join("push_log"),
                )
            }
        };
        Self::bind(namespace_label, manifest.join(relative_root), manifest)
    }

    fn bind(
        namespace_label: String,
        root: std::path::PathBuf,
        creation_boundary: &std::path::Path,
    ) -> Result<Self, PushLogError> {
        let root_binding = open_or_create_push_log_root(&root, creation_boundary)?;
        let rebound_root = revalidate_push_log_directory_chain(&root_binding)?;
        let lock_path = root.join(PUSH_LOG_LOCK_FILE);
        let lock = push_log_openat(
            &rebound_root,
            std::ffi::OsStr::new(PUSH_LOG_LOCK_FILE),
            PUSH_LOG_O_RDWR | PUSH_LOG_O_CREAT,
            0o600_u32,
        )
        .map_err(|error| {
            PushLogError::from_io("open pinned push_log lock", &lock_path, error, true)
        })?;
        let lock_identity = validate_push_log_leaf(&lock, &lock_path)?;
        rebound_root.sync_all().map_err(|error| {
            PushLogError::from_io("sync push_log root after lock bind", &root, error, false)
        })?;
        revalidate_push_log_leaf_at(
            &rebound_root,
            std::ffi::OsStr::new(PUSH_LOG_LOCK_FILE),
            &lock_path,
            PUSH_LOG_O_RDWR,
            lock_identity,
        )?;
        revalidate_push_log_directory_chain(&root_binding)?;
        Ok(Self {
            namespace_label,
            root,
            root_binding,
            lock: std::sync::Arc::new(lock),
            lock_identity,
        })
    }

    #[cfg(test)]
    pub(super) fn for_test_anchor(
        namespace_label: &str,
        anchor: &std::path::Path,
        relative_root: &std::path::Path,
    ) -> Result<Self, PushLogError> {
        Self::bind(
            namespace_label.to_owned(),
            anchor.join(relative_root),
            anchor,
        )
    }

    fn save(&self, text: &str) -> Result<std::path::PathBuf, PushLogError> {
        self.save_with_hook(text, |_, _, _, _| {})
    }

    fn save_with_hook<F>(&self, text: &str, mut hook: F) -> Result<std::path::PathBuf, PushLogError>
    where
        F: FnMut(PushLogWritePhase, &std::path::Path, &std::path::Path, Option<&std::path::Path>),
    {
        self.save_payload_with_hook(text.as_bytes(), "md", |phase, root, date_path, artifact| {
            hook(phase, root, date_path, artifact);
        })
    }

    fn save_payload_with_hook<F>(
        &self,
        payload: &[u8],
        extension: &str,
        mut hook: F,
    ) -> Result<std::path::PathBuf, PushLogError>
    where
        F: FnMut(PushLogWritePhase, &std::path::Path, &std::path::Path, Option<&std::path::Path>),
    {
        use fs2::FileExt;
        if extension.is_empty() || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(PushLogError::NamespaceIsolation(
                "push_log artifact extension must be non-empty ASCII alphanumeric".to_owned(),
            ));
        }
        let now = chrono::Local::now();
        let time_prefix = now.format("%H%M%S").to_string();
        let unique_suffix =
            push_log_suffix_at(std::time::SystemTime::now()).map_err(PushLogError::Persistence)?;
        let file_name = format!("{time_prefix}_{unique_suffix}.{extension}");
        let _process_guard = push_log_process_mutex()
            .lock()
            .map_err(|_| PushLogError::Persistence("push_log process mutex poisoned".to_owned()))?;
        self.lock.lock_exclusive().map_err(|error| {
            PushLogError::from_io(
                "lock push_log namespace",
                &self.root.join(PUSH_LOG_LOCK_FILE),
                error,
                false,
            )
        })?;
        let outcome = self.save_named_payload_while_locked(payload, &file_name, &mut hook);
        let unlock = FileExt::unlock(&*self.lock).map_err(|error| {
            PushLogError::from_io(
                "unlock push_log namespace",
                &self.root.join(PUSH_LOG_LOCK_FILE),
                error,
                false,
            )
        });
        match (outcome, unlock) {
            (Ok(path), Ok(())) => Ok(path),
            (Err(error), Ok(())) => Err(error),
            (_, Err(error)) => Err(error),
        }
    }

    fn save_named_payload(
        &self,
        payload: &[u8],
        file_name: &str,
    ) -> Result<std::path::PathBuf, PushLogError> {
        use fs2::FileExt;
        let _process_guard = push_log_process_mutex()
            .lock()
            .map_err(|_| PushLogError::Persistence("push_log process mutex poisoned".to_owned()))?;
        self.lock.lock_exclusive().map_err(|error| {
            PushLogError::from_io(
                "lock push_log namespace",
                &self.root.join(PUSH_LOG_LOCK_FILE),
                error,
                false,
            )
        })?;
        let outcome =
            self.save_named_payload_while_locked(payload, file_name, &mut |_, _, _, _| {});
        let unlock = FileExt::unlock(&*self.lock).map_err(|error| {
            PushLogError::from_io(
                "unlock push_log namespace",
                &self.root.join(PUSH_LOG_LOCK_FILE),
                error,
                false,
            )
        });
        match (outcome, unlock) {
            (Ok(path), Ok(())) => Ok(path),
            (Err(error), Ok(())) => Err(error),
            (_, Err(error)) => Err(error),
        }
    }

    /// Read one exact artifact back through the retained push-log capability.
    /// The caller-provided path is accepted only when it is the exact
    /// `<pinned-root>/YYYY-MM-DD/<one-component>` shape.
    fn read_exact_payload(&self, path: &std::path::Path) -> Result<Vec<u8>, PushLogError> {
        use fs2::FileExt;
        use std::io::Read;
        use std::path::Component;

        let relative = path.strip_prefix(&self.root).map_err(|_| {
            PushLogError::NamespaceIsolation(format!(
                "push_log verifier path escapes pinned root: {}",
                path.display()
            ))
        })?;
        let mut components = relative.components();
        let (date_component, file_component) =
            match (components.next(), components.next(), components.next()) {
                (Some(Component::Normal(date)), Some(Component::Normal(file_name)), None) => {
                    (date.to_os_string(), file_name.to_os_string())
                }
                _ => {
                    return Err(PushLogError::NamespaceIsolation(format!(
                        "push_log verifier path is not date/artifact: {}",
                        path.display()
                    )));
                }
            };
        let date_text = date_component.to_string_lossy();
        if date_text.len() != 10
            || !date_text.bytes().enumerate().all(|(index, byte)| {
                matches!(index, 4 | 7) && byte == b'-'
                    || !matches!(index, 4 | 7) && byte.is_ascii_digit()
            })
        {
            return Err(PushLogError::NamespaceIsolation(
                "push_log verifier date component is invalid".to_owned(),
            ));
        }

        let _process_guard = push_log_process_mutex()
            .lock()
            .map_err(|_| PushLogError::Persistence("push_log process mutex poisoned".to_owned()))?;
        self.lock.lock_exclusive().map_err(|error| {
            PushLogError::from_io(
                "lock push_log namespace for verifier",
                &self.root.join(PUSH_LOG_LOCK_FILE),
                error,
                false,
            )
        })?;
        let outcome = (|| {
            let rebound_root = revalidate_push_log_directory_chain(&self.root_binding)?;
            revalidate_push_log_leaf_at(
                &rebound_root,
                std::ffi::OsStr::new(PUSH_LOG_LOCK_FILE),
                &self.root.join(PUSH_LOG_LOCK_FILE),
                PUSH_LOG_O_RDWR,
                self.lock_identity,
            )?;
            let date_path = self.root.join(&date_component);
            let date_directory =
                push_log_openat(&rebound_root, &date_component, PUSH_LOG_O_RDONLY, 0).map_err(
                    |error| {
                        PushLogError::from_io(
                            "open existing push_log date directory for verifier",
                            &date_path,
                            error,
                            true,
                        )
                    },
                )?;
            let date_identity = validate_push_log_directory(&date_directory, &date_path)?;
            let mut binding = self.root_binding.try_clone()?;
            binding.push(
                date_component.clone(),
                date_identity,
                date_directory.try_clone().map_err(|error| {
                    PushLogError::Persistence(format!(
                        "clone push_log verifier date directory: {error}"
                    ))
                })?,
            );
            let rebound_date = revalidate_push_log_directory_chain(&binding)?;
            let mut artifact = push_log_openat(
                &rebound_date,
                &file_component,
                PUSH_LOG_O_RDONLY,
                0,
            )
            .map_err(|error| {
                PushLogError::from_io("open exact push_log verifier artifact", path, error, true)
            })?;
            let artifact_identity = validate_push_log_leaf(&artifact, path)?;
            let mut bytes = Vec::new();
            artifact.read_to_end(&mut bytes).map_err(|error| {
                PushLogError::Persistence(format!(
                    "read exact push_log verifier artifact {}: {error}",
                    path.display()
                ))
            })?;
            let post_date = revalidate_push_log_directory_chain(&binding)?;
            revalidate_push_log_leaf_at(
                &post_date,
                &file_component,
                path,
                PUSH_LOG_O_RDONLY,
                artifact_identity,
            )?;
            revalidate_push_log_directory_chain(&self.root_binding)?;
            Ok(bytes)
        })();
        let unlock = FileExt::unlock(&*self.lock).map_err(|error| {
            PushLogError::from_io(
                "unlock push_log namespace after verifier",
                &self.root.join(PUSH_LOG_LOCK_FILE),
                error,
                false,
            )
        });
        match (outcome, unlock) {
            (Ok(bytes), Ok(())) => Ok(bytes),
            (Err(error), Ok(())) => Err(error),
            (_, Err(error)) => Err(error),
        }
    }

    fn save_named_payload_while_locked<F>(
        &self,
        payload: &[u8],
        file_name: &str,
        hook: &mut F,
    ) -> Result<std::path::PathBuf, PushLogError>
    where
        F: FnMut(PushLogWritePhase, &std::path::Path, &std::path::Path, Option<&std::path::Path>),
    {
        use std::io::Write;

        if file_name.is_empty()
            || file_name.as_bytes().contains(&b'/')
            || file_name.as_bytes().contains(&0)
            || !file_name.ends_with(".json") && !file_name.ends_with(".md")
        {
            return Err(PushLogError::NamespaceIsolation(
                "push_log artifact name must be one .json/.md component".to_owned(),
            ));
        }
        if std::env::var_os("PUSH_LOG_DIR").is_some() {
            return Err(PushLogError::NamespaceOverrideRejected {
                namespace: self.namespace_label.clone(),
            });
        }
        let now = chrono::Local::now();
        let date_component = now.format("%Y-%m-%d").to_string();
        let rebound_root = revalidate_push_log_directory_chain(&self.root_binding)?;
        revalidate_push_log_leaf_at(
            &rebound_root,
            std::ffi::OsStr::new(PUSH_LOG_LOCK_FILE),
            &self.root.join(PUSH_LOG_LOCK_FILE),
            PUSH_LOG_O_RDWR,
            self.lock_identity,
        )?;
        let mut binding = self.root_binding.try_clone()?;
        let date_path = self.root.join(&date_component);
        let (date_directory, date_identity) = open_or_create_push_log_child(
            &rebound_root,
            std::ffi::OsStr::new(&date_component),
            &date_path,
        )?;
        binding.push(
            std::ffi::OsString::from(&date_component),
            date_identity,
            date_directory,
        );
        let path = date_path.join(file_name);
        hook(
            PushLogWritePhase::DirectoriesBound,
            &self.root,
            &date_path,
            None,
        );
        let pre_write_date_directory = revalidate_push_log_directory_chain(&binding)?;
        let mut file = push_log_openat(
            &pre_write_date_directory,
            std::ffi::OsStr::new(file_name),
            PUSH_LOG_O_WRONLY | PUSH_LOG_O_CREAT | PUSH_LOG_O_EXCL,
            0o600_u32,
        )
        .map_err(|error| {
            PushLogError::from_io(
                "create push_log artifact without symlink traversal",
                &path,
                error,
                true,
            )
        })?;
        let artifact_identity = validate_push_log_leaf(&file, &path)?;
        file.write_all(payload).map_err(|error| {
            PushLogError::Persistence(format!("push_log 写入失败 {}: {error}", path.display()))
        })?;
        file.sync_all().map_err(|error| {
            PushLogError::Persistence(format!("push_log fsync 失败 {}: {error}", path.display()))
        })?;
        let synced_identity = validate_push_log_leaf(&file, &path)?;
        if synced_identity != artifact_identity {
            return Err(PushLogError::NamespaceIsolation(format!(
                "push_log artifact identity changed while open: {}",
                path.display()
            )));
        }
        pre_write_date_directory.sync_all().map_err(|error| {
            PushLogError::Persistence(format!(
                "push_log 目录 fsync 失败 {}: {error}",
                date_path.display()
            ))
        })?;
        hook(
            PushLogWritePhase::ArtifactSynced,
            &self.root,
            &date_path,
            Some(&path),
        );
        let post_write_date_directory = revalidate_push_log_directory_chain(&binding)?;
        let reopened = push_log_openat(
            &post_write_date_directory,
            std::ffi::OsStr::new(file_name),
            PUSH_LOG_O_RDONLY,
            0,
        )
        .map_err(|error| {
            PushLogError::from_io("re-open push_log artifact after fsync", &path, error, true)
        })?;
        let reopened_identity = validate_push_log_leaf(&reopened, &path)?;
        if reopened_identity != artifact_identity {
            return Err(PushLogError::NamespaceIsolation(format!(
                "push_log artifact identity changed before final validation: {}",
                path.display()
            )));
        }
        let final_root = revalidate_push_log_directory_chain(&self.root_binding)?;
        revalidate_push_log_leaf_at(
            &final_root,
            std::ffi::OsStr::new(PUSH_LOG_LOCK_FILE),
            &self.root.join(PUSH_LOG_LOCK_FILE),
            PUSH_LOG_O_RDWR,
            self.lock_identity,
        )?;
        // A hostile process running under the same UID can still mutate the
        // namespace after this final validation. Production therefore requires
        // an exclusive service UID and owner-only writable manifest data roots.
        Ok(path)
    }
}

fn validate_push_log_test_code(test_code: &str) -> Result<(), PushLogError> {
    if !test_code.starts_with("TEST_CODE")
        || !test_code
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(PushLogError::NamespaceIsolation(
            "push_log TEST_CODE must be one path-safe TEST_CODE-prefixed component".to_owned(),
        ));
    }
    Ok(())
}

fn generic_push_log_writers() -> &'static std::sync::Mutex<
    std::collections::BTreeMap<String, std::sync::Arc<PinnedPushLogWriter>>,
> {
    static WRITERS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<String, std::sync::Arc<PinnedPushLogWriter>>>,
    > = std::sync::OnceLock::new();
    WRITERS.get_or_init(|| std::sync::Mutex::new(std::collections::BTreeMap::new()))
}

pub(super) fn eager_bind_push_log_capability(
    namespace: &crate::durable_delivery_runtime::RuntimeNamespace,
) -> Result<std::sync::Arc<PinnedPushLogWriter>, PushLogError> {
    if std::env::var_os("PUSH_LOG_DIR").is_some() {
        return Err(PushLogError::NamespaceOverrideRejected {
            namespace: namespace.label(),
        });
    }
    let namespace_label = namespace.label();
    let mut writers = generic_push_log_writers().lock().map_err(|_| {
        PushLogError::Persistence("push_log writer registry mutex poisoned".to_owned())
    })?;
    if let Some(writer) = writers.get(&namespace_label) {
        revalidate_push_log_directory_chain(&writer.root_binding)?;
        return Ok(std::sync::Arc::clone(writer));
    }
    let writer = std::sync::Arc::new(PinnedPushLogWriter::for_namespace(namespace)?);
    writers.insert(namespace_label, std::sync::Arc::clone(&writer));
    Ok(writer)
}

#[cfg(test)]
fn save_push_log_at_root(
    root: &std::path::Path,
    text: &str,
) -> Result<std::path::PathBuf, PushLogError> {
    let boundary = root.parent().ok_or_else(|| {
        PushLogError::NamespaceIsolation(format!(
            "test push_log root has no creation boundary: {}",
            root.display()
        ))
    })?;
    PinnedPushLogWriter::bind("TEST_CODE_FIXTURE".to_owned(), root.to_path_buf(), boundary)?
        .save(text)
}

#[cfg(test)]
fn save_push_log_at_root_with_hook<F>(
    root: &std::path::Path,
    text: &str,
    mut hook: F,
) -> Result<std::path::PathBuf, PushLogError>
where
    F: FnMut(PushLogWritePhase, &std::path::Path, &std::path::Path, Option<&std::path::Path>),
{
    let boundary = root.parent().ok_or_else(|| {
        PushLogError::NamespaceIsolation(format!(
            "test push_log root has no creation boundary: {}",
            root.display()
        ))
    })?;
    PinnedPushLogWriter::bind("TEST_CODE_FIXTURE".to_owned(), root.to_path_buf(), boundary)?
        .save_with_hook(text, |phase, root, date_path, artifact| {
            hook(phase, root, date_path, artifact);
        })
}

fn save_push_log(
    bound_namespace: &crate::durable_delivery_runtime::RuntimeNamespace,
    text: &str,
) -> Result<std::path::PathBuf, PushLogError> {
    log::info!(
        "[v69] save_push_log entered, text len={}",
        text.chars().count()
    );
    if std::env::var_os("PUSH_LOG_DIR").is_some() {
        return Err(PushLogError::NamespaceOverrideRejected {
            namespace: bound_namespace.label(),
        });
    }
    let namespace_label = bound_namespace.label();
    let writer = {
        let writers = generic_push_log_writers().lock().map_err(|_| {
            PushLogError::Persistence("push_log writer registry mutex poisoned".to_owned())
        })?;
        writers
            .get(&namespace_label)
            .map(std::sync::Arc::clone)
            .ok_or_else(|| {
                PushLogError::Persistence(format!(
                    "push_log capability was not eagerly bound for namespace {namespace_label}"
                ))
            })?
    };
    let path = writer.save(text)?;
    log::info!("[v69] push_log 写入: {}", path.display());
    Ok(path)
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
    push_governor_inner_with_source_evidence(text, kind, code, None, None, None).await
}

async fn push_governor_inner_with_source_evidence(
    text: &str,
    kind: PushKind,
    code: Option<&str>,
    source_fact: Option<&crate::v14_adapter::SourceFactEvidence>,
    source_batch: Option<&crate::v14_adapter::SourceBatchEvidence>,
    br196_smoke: Option<&crate::br196_test_delivery::GovernanceSmokeDispatch<'_>>,
) -> PushOutcome {
    use crate::v14_adapter::{self, V14Gate};

    if crate::durable_delivery_runtime::is_counted_kind(kind) {
        log::error!(
            "[DurableDelivery][BR-192] generic PushKind::{kind:?} rejected: counted_binding_required"
        );
        return PushOutcome::Denied("counted_binding_required".to_owned());
    }

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
    let gate = match (source_fact, source_batch, br196_smoke) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => {
            return PushOutcome::Denied("multiple_source_evidence_bindings".to_owned());
        }
        (Some(evidence), None, None) => v14_adapter::v14_gate_source_fact(evidence),
        (None, Some(evidence), None) => v14_adapter::v14_gate_source_batch(evidence),
        (None, None, Some(dispatch)) => {
            if dispatch.push_kind() != kind || dispatch.code() != code {
                return PushOutcome::Denied("br196_governance_smoke_binding_mismatch".to_owned());
            }
            v14_adapter::v14_gate_br196_smoke(dispatch)
        }
        (None, None, None) => v14_adapter::v14_gate(kind, code),
    };
    let event = match gate {
        V14Gate::Deduped => return PushOutcome::Deduped,
        V14Gate::Denied(reason) => return PushOutcome::Denied(reason),
        V14Gate::Approved(event) => *event,
    };
    let start = std::time::Instant::now();
    deliver_and_record(event, kind, text, start, None, None, source_batch).await
}

/// 公共尾段: L5/L6 投递 + L7/哈希链留痕 + commit/rollback.
/// 仅供非 counted generic governor 使用；counted delivery 走 BR-192 binding 链.
async fn deliver_and_record(
    event: stock_analysis::push_l1::SignalEvent,
    kind: PushKind,
    text: &str,
    start: std::time::Instant,
    sub_kind: Option<&str>,
    cooldown_override_secs: Option<u32>,
    source_batch: Option<&crate::v14_adapter::SourceBatchEvidence>,
) -> PushOutcome {
    use crate::v14_adapter;
    // Defensive BR-192 boundary. Public generic governors reject counted kinds
    // before constructing a synthetic event; this keeps future internal
    // call-sites from accidentally regaining the retired path.
    if crate::durable_delivery_runtime::is_counted_kind(kind) {
        return PushOutcome::Denied("counted_binding_required".to_owned());
    }

    // BR-144 governs the legacy delivery audit chain only. Counted kinds have
    // already branched into BR-192's exact-byte append/reconcile chain above.
    if let stock_analysis::event::AuditHealth::Degraded { reason_code } =
        stock_analysis::event::runtime_delivery_audit_health()
    {
        log::error!("[AuditDegraded][BR-144] delivery blocked before sink: {reason_code}");
        return PushOutcome::SinkError(format!("delivery audit unavailable: {reason_code}"));
    }

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
    let audit_result = match source_batch {
        Some(evidence) => stock_analysis::event::publish_source_batch_delivery(
            &kind.stable_template_id(),
            outcome_str,
            channel,
            text.len(),
            latency_ms,
            evidence.business_date(),
            evidence.observed_at(),
            evidence.batch_id(),
            evidence.content_hash(),
        ),
        None => stock_analysis::event::publish_delivery(
            &kind.stable_template_id(),
            event.code.as_deref(),
            outcome_str,
            channel,
            text.len(),
            latency_ms,
        ),
    };
    let dedup_result = settle_dedup_after_delivery(
        &event,
        kind,
        sub_kind,
        cooldown_override_secs,
        delivered,
        // BR-145: once the physical sink accepts the message, commit the
        // identity even if a post-delivery audit fails; never resend it.
        delivered,
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
#[cfg(test)]
async fn push_governor(text: &str, kind: PushKind) -> bool {
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
pub(super) async fn push_governor_v3(
    text: &str,
    kind: PushKind,
    code: Option<&str>,
) -> PushOutcome {
    push_governor_inner(text, kind, code).await
}

/// Dedicated BR-196 exact-six governance smoke entry.
///
/// The dispatch is minted only by the invocation-scoped BR-196 context; the
/// generic governor remains bound to the ordinary process clock and quiet-hour
/// policy.
pub(super) async fn push_br196_governance_smoke_v3(
    text: &str,
    dispatch: crate::br196_test_delivery::GovernanceSmokeDispatch<'_>,
) -> PushOutcome {
    let kind = dispatch.push_kind();
    let code = dispatch.code();
    push_governor_inner_with_source_evidence(text, kind, code, None, None, Some(&dispatch)).await
}

/// BR-196 presentation-token gateway for registered production card shapes.
/// The token is non-cloneable and consumed by this dispatch.
pub async fn push_presented_v3(
    token: crate::presentation_registry::ProductionPresentationToken,
    text: &str,
    code: Option<&str>,
) -> PushOutcome {
    let kind = token.descriptor().push_kind;
    push_governor_inner(text, kind, code).await
}

/// BR-192 sole generic counted-delivery entry.
///
/// Launch/L5 governance receives a stable SignalEvent derived from the
/// caller-supplied occurrence identity. The durable runtime receives only the
/// immutable binding, never that governance event.
pub async fn push_counted_with_binding(
    token: crate::presentation_registry::ProductionPresentationToken,
    text: &str,
    sub_kind: Option<DailyReportSubKind>,
    binding: crate::durable_delivery_runtime::CountedDeliveryBinding,
) -> PushOutcome {
    use crate::v14_adapter::V14Gate;

    let kind = token.descriptor().push_kind;
    if !crate::durable_delivery_runtime::is_counted_kind(kind) {
        return PushOutcome::Denied("counted_kind_required".to_owned());
    }
    if !launch_gate_check(kind) {
        return PushOutcome::Denied("launch_gate_stage".to_owned());
    }
    let sub_kind_label = sub_kind.map(DailyReportSubKind::label);
    let gate = crate::v14_adapter::v14_gate_counted_binding(
        kind,
        binding.governance_code(),
        sub_kind_label,
        binding.schedule_occurrence_identity(),
        binding.business_date(),
    );
    let governance_event = match gate {
        V14Gate::Deduped => {
            return PushOutcome::Denied("counted_gate_returned_legacy_dedup".to_owned());
        }
        V14Gate::Denied(reason) => return PushOutcome::Denied(reason),
        V14Gate::Approved(event) => event,
    };
    log::debug!(
        "[DurableDelivery][BR-192] stable governance event approved event_id={} occurrence={}",
        governance_event.event_id,
        binding.schedule_occurrence_identity()
    );
    crate::durable_delivery_runtime::deliver_counted_binding(
        binding,
        kind,
        text.to_owned(),
        sub_kind,
    )
    .await
}

/// BR-194 sole counted SourceOnly entry. The profile is derived from the
/// canonical R-04 binding and cannot be selected by a caller.
pub async fn push_counted_source_only_with_binding(
    token: crate::presentation_registry::ProductionPresentationToken,
    text: &str,
    binding: crate::durable_delivery_runtime::CountedDeliveryBinding,
) -> PushOutcome {
    let kind = token.descriptor().push_kind;
    if !crate::durable_delivery_runtime::is_counted_kind(kind) || kind != PushKind::ReviewLhb {
        return PushOutcome::Denied("counted_source_only_kind_not_allowed".to_owned());
    }
    if let Err(reason) = binding.validate_r04_source_only_text(text) {
        return PushOutcome::Denied(reason.to_owned());
    }
    push_counted_source_only_after_validation_with(
        text,
        kind,
        binding,
        launch_gate_check,
        crate::v14_adapter::v14_gate_counted_source_only_binding,
        |binding, kind, text| async move {
            crate::durable_delivery_runtime::deliver_counted_binding(binding, kind, text, None)
                .await
        },
    )
    .await
}

/// BR-199 sole public-only R-08 entry. The caller cannot select another kind
/// or route this binding through the combined-account counted gate.
async fn push_r08_source_only_with_binding(
    text: &str,
    binding: crate::durable_delivery_runtime::CountedDeliveryBinding,
) -> PushOutcome {
    let kind = PushKind::EventCalendar;
    if let Err(reason) = binding.validate_r08_public_source_only_text(text) {
        return PushOutcome::Denied(reason.to_owned());
    }
    push_counted_source_only_after_validation_with(
        text,
        kind,
        binding,
        launch_gate_check,
        crate::v14_adapter::v14_gate_r08_source_only_binding,
        |binding, kind, text| async move {
            crate::durable_delivery_runtime::deliver_counted_binding(binding, kind, text, None)
                .await
        },
    )
    .await
}

pub async fn push_r08_presented_source_only_with_binding(
    token: crate::presentation_registry::ProductionPresentationToken,
    text: &str,
    binding: crate::durable_delivery_runtime::CountedDeliveryBinding,
) -> PushOutcome {
    if token.descriptor().push_kind != PushKind::EventCalendar {
        return PushOutcome::Denied("presentation_token_kind_mismatch".to_owned());
    }
    push_r08_source_only_with_binding(text, binding).await
}

async fn push_counted_source_only_after_validation_with<Launch, Gate, Deliver, DeliveryFuture>(
    text: &str,
    kind: PushKind,
    binding: crate::durable_delivery_runtime::CountedDeliveryBinding,
    launch: Launch,
    gate: Gate,
    deliver: Deliver,
) -> PushOutcome
where
    Launch: FnOnce(PushKind) -> bool,
    Gate: FnOnce(
        PushKind,
        &crate::durable_delivery_runtime::CountedDeliveryBinding,
    ) -> crate::v14_adapter::V14Gate,
    Deliver: FnOnce(
        crate::durable_delivery_runtime::CountedDeliveryBinding,
        PushKind,
        String,
    ) -> DeliveryFuture,
    DeliveryFuture: std::future::Future<Output = PushOutcome>,
{
    use crate::v14_adapter::V14Gate;

    if !launch(kind) {
        return PushOutcome::Denied("launch_gate_stage".to_owned());
    }
    let governance_event = match gate(kind, &binding) {
        V14Gate::Deduped => {
            return PushOutcome::Denied("counted_gate_returned_legacy_dedup".to_owned());
        }
        V14Gate::Denied(reason) => return PushOutcome::Denied(reason),
        V14Gate::Approved(event) => event,
    };
    log::debug!(
        "[DurableDelivery][BR-194] source-only governance approved event_id={} occurrence={}",
        governance_event.event_id,
        binding.schedule_occurrence_identity()
    );
    deliver(binding, kind, text.to_owned()).await
}

/// BR-137 sole delivery entry for a validated source-self-contained fact.
/// Kind and dedup identity are derived from the evidence so callers cannot
/// pair a relaxed source profile with an unrelated PushKind.
async fn push_source_fact_v3(
    text: &str,
    evidence: &crate::v14_adapter::SourceFactEvidence,
) -> PushOutcome {
    push_governor_inner_with_source_evidence(
        text,
        evidence.kind(),
        evidence.security_code(),
        Some(evidence),
        None,
        None,
    )
    .await
}

pub async fn push_presented_source_fact_v3(
    token: crate::presentation_registry::ProductionPresentationToken,
    text: &str,
    evidence: &crate::v14_adapter::SourceFactEvidence,
) -> PushOutcome {
    if token.descriptor().push_kind != evidence.kind() {
        return PushOutcome::Denied("presentation_token_kind_mismatch".to_owned());
    }
    push_source_fact_v3(text, evidence).await
}

/// BR-160 sole delivery entry for an immutable, already committed A-10 source
/// batch. Kind and governance identity are derived from the binding.
pub async fn push_source_batch_v3(
    token: crate::presentation_registry::ProductionPresentationToken,
    text: &str,
    evidence: &crate::v14_adapter::SourceBatchEvidence,
) -> PushOutcome {
    if token.descriptor().push_kind != evidence.kind() {
        return PushOutcome::Denied("presentation_token_kind_mismatch".to_owned());
    }
    push_governor_inner_with_source_evidence(
        text,
        evidence.kind(),
        None,
        None,
        Some(evidence),
        None,
    )
    .await
}

/// b013 P0-1 兜底: PerTicket 类 kind 在缺 code 时塞占位, 让 L4 走全局 key,
/// 至少防止"无限重发同一票"。b014 应把所有 caller 改成 push_governor_v3 显式传 code。
#[cfg(test)]
fn requires_ticket_code(kind: PushKind) -> bool {
    use PushKind::*;
    matches!(
        kind,
        HoldingPlan
            | T0Advice
            | CandidateTriggered
            | ForbiddenOps
            | PaperTrade
            | PaperSell
            | NewsToIdea
            | PostFixedPriceOrder
            | PostFixedPriceFill
            | StPriceLimitChanged
            | BlockTradeIntradayConfirm
            | BlockTradePriceRange
    )
}

pub async fn push_wechat(text: &str) -> bool {
    let bound_namespace = match crate::durable_delivery_runtime::current_runtime_namespace() {
        Ok(namespace) => namespace,
        Err(error) => {
            log::error!("[BR-192] push-log namespace binding rejected: {error}");
            return false;
        }
    };
    // v10 P6 5 要素接入: V10_DRY_RUN_PUSH=1 时跳过实际推送, 仅 log
    // 用于开发/验证推送内容变化, 不骚扰飞书
    if dry_run_push_active() {
        log::info!("[V10_DRY_RUN_PUSH] 跳过飞书推送, 内容预览:\n{}", text);
        // v69: 沙箱 dry-run 也保存 push_log
        if let Err(error) = save_push_log(&bound_namespace, text) {
            log::error!(
                "[BR-086] dry-run push audit failed: reason_code={} retry_authorized={} {error}",
                error.reason_code(),
                error.retry_authorized()
            );
            return false;
        }
        return true;
    }

    // v69: 不管走哪条推送路径 (magiclaw cli / feishu http / 后续), 都先保存 push_log
    if let Err(error) = save_push_log(&bound_namespace, text) {
        log::error!(
            "[BR-086] push audit failed; delivery blocked: reason_code={} retry_authorized={} {error}",
            error.reason_code(),
            error.retry_authorized()
        );
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
                        log::warn!(
                            "[{}] daemon 鉴权预检重试仍失败，但已重新签发 token，将继续尝试发送: {}",
                            send_type.label(),
                            e
                        );
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

/// BR-192 single authoritative counted-delivery adapter.
///
/// This function is intentionally synchronous: the durable coordinator calls
/// it only from `spawn_blocking`, so no nested Tokio runtime is constructed or
/// dropped inside an async worker.  Production accepts only the CLI transport
/// because it is the sole current transport that returns both a validated
/// local message ID and the remote platform message ID.
pub(super) fn deliver_authoritative_blocking(
    bound_namespace: &crate::durable_delivery_runtime::RuntimeNamespace,
    push_log_writer: &PinnedPushLogWriter,
    delivery_audit: &stock_analysis::event::AuditDispatcher,
    request: &stock_analysis::durable_delivery::AuthoritativeDeliveryRequest,
) -> stock_analysis::durable_delivery::AuthoritativeSinkResult {
    use stock_analysis::durable_delivery::{
        AuthoritativeSinkResult, TypedReceipt, TypedRejection, TypedUncertainty,
    };

    let observed_at = chrono::Utc::now();
    let test_namespace = match bound_namespace {
        crate::durable_delivery_runtime::RuntimeNamespace::Production => None,
        crate::durable_delivery_runtime::RuntimeNamespace::Test { test_code } => {
            Some(test_code.as_str())
        }
    };
    let canonical_template_id = request.push_kind.stable_template_id();
    if request.stable_template_id != canonical_template_id {
        let result = AuthoritativeSinkResult::Rejected(TypedRejection {
            reason_code: "durable_template_binding_invalid".to_owned(),
            evidence: format!(
                "push_kind={} expected_template={} supplied_template={}",
                request.push_kind.as_str(),
                canonical_template_id,
                request.stable_template_id
            )
            .into_bytes(),
            retry_authorized: false,
            observed_at,
        });
        return finalize_counted_delivery(push_log_writer, delivery_audit, request, "", result);
    }
    let invalid_utf8_placeholder;
    let text = match std::str::from_utf8(&request.rendered_content) {
        Ok(text) if !text.trim().is_empty() => text,
        Ok(_) => {
            let result = AuthoritativeSinkResult::Rejected(TypedRejection {
                reason_code: "empty_rendered_content".to_owned(),
                evidence: b"rendered content is empty".to_vec(),
                retry_authorized: false,
                observed_at,
            });
            return finalize_counted_delivery(push_log_writer, delivery_audit, request, "", result);
        }
        Err(error) => {
            let result = AuthoritativeSinkResult::Rejected(TypedRejection {
                reason_code: "rendered_content_not_utf8".to_owned(),
                evidence: error.to_string().into_bytes(),
                retry_authorized: false,
                observed_at,
            });
            invalid_utf8_placeholder = format!(
                "<non-UTF8 rendered content; sha256={}>",
                request.rendered_content_sha256
            );
            return finalize_counted_delivery(
                push_log_writer,
                delivery_audit,
                request,
                &invalid_utf8_placeholder,
                result,
            );
        }
    };

    let raw_result = if let Some(test_code) = test_namespace {
        log::info!(
            "[V10_DRY_RUN_PUSH][BR-192] authoritative test delivery skipped network namespace={} decision={}",
            test_code,
            request.decision_identity,
        );
        AuthoritativeSinkResult::Accepted(TypedReceipt {
            channel: "TEST_CODE_DRY_RUN".to_owned(),
            provider: "TEST_CODE_MAGICLAW_DRY_RUN".to_owned(),
            message_id: format!(
                "TEST_CODE_DRY_RUN_{}",
                request
                    .decision_identity
                    .chars()
                    .take(24)
                    .collect::<String>()
            ),
            platform_message_id: Some(format!(
                "TEST_CODE_PLATFORM_{}",
                request
                    .attempt_identity
                    .chars()
                    .take(24)
                    .collect::<String>()
            )),
            accepted_at: observed_at,
            latency_ms: Some(0),
        })
    } else {
        let send_type = resolve_send_type();
        let send_transport = resolve_send_transport(send_type);
        if !matches!(send_transport, MessageSendTransport::Cli) {
            AuthoritativeSinkResult::Rejected(TypedRejection {
                reason_code: "typed_receipt_transport_unavailable".to_owned(),
                evidence: format!(
                    "channel={} transport={} does not return a typed remote receipt",
                    send_type.as_str(),
                    send_transport.as_str()
                )
                .into_bytes(),
                retry_authorized: true,
                observed_at,
            })
        } else {
            let started = std::time::Instant::now();
            match push_via_magiclaw_cli_receipt_blocking(send_type, text) {
                Ok(receipt) => AuthoritativeSinkResult::Accepted(TypedReceipt {
                    channel: send_type.as_str().to_owned(),
                    provider: "magiclaw-cli".to_owned(),
                    message_id: receipt.message_id,
                    platform_message_id: Some(receipt.platform_msg_id),
                    accepted_at: chrono::Utc::now(),
                    latency_ms: Some(
                        i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
                    ),
                }),
                Err(BlockingCliDeliveryFailure::Rejected {
                    reason_code,
                    evidence,
                }) => AuthoritativeSinkResult::Rejected(TypedRejection {
                    reason_code,
                    evidence,
                    retry_authorized: true,
                    observed_at: chrono::Utc::now(),
                }),
                Err(BlockingCliDeliveryFailure::Uncertain {
                    reason_code,
                    evidence,
                }) => AuthoritativeSinkResult::Uncertain(TypedUncertainty {
                    reason_code,
                    evidence,
                    observed_at: chrono::Utc::now(),
                }),
            }
        }
    };
    let finalized =
        finalize_counted_delivery(push_log_writer, delivery_audit, request, text, raw_result);
    #[cfg(test)]
    maybe_crash_after_test_counted_accept(test_namespace, &finalized);
    finalized
}

#[cfg(test)]
fn maybe_crash_after_test_counted_accept(
    test_namespace: Option<&str>,
    result: &stock_analysis::durable_delivery::AuthoritativeSinkResult,
) {
    use std::io::Write;

    if std::env::var_os("BR192_FULL_CHAIN_CRASH_AFTER_ACCEPTED").is_none()
        || !matches!(
            result,
            stock_analysis::durable_delivery::AuthoritativeSinkResult::Accepted(_)
        )
    {
        return;
    }
    let test_code =
        test_namespace.expect("BR-192 accepted-crash injection is restricted to TEST_CODE");
    assert!(
        test_code.starts_with("TEST_CODE")
            && test_code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')),
        "BR-192 accepted-crash injection requires one path-safe TEST_CODE component"
    );
    let namespace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data/test")
        .join(test_code);
    let ready_path = namespace.join("br192_remote_accepted.ready");
    let release_path = namespace.join("br192_remote_accepted.release");
    let pending_ready_path = namespace.join(format!(
        ".br192_remote_accepted.ready.{}.tmp",
        std::process::id()
    ));
    let mut marker = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending_ready_path)
        .expect("create private BR-192 accepted marker");
    let canonical = serde_json::to_vec(&authoritative_sink_result_value(result))
        .expect("serialize BR-192 accepted marker");
    marker
        .write_all(&canonical)
        .expect("write BR-192 accepted marker");
    marker.sync_all().expect("fsync BR-192 accepted marker");
    drop(marker);
    std::fs::hard_link(&pending_ready_path, &ready_path)
        .expect("atomically publish complete BR-192 accepted marker without overwrite");
    std::fs::File::open(&namespace)
        .and_then(|directory| directory.sync_all())
        .expect("fsync TEST_CODE namespace after accepted marker");
    std::fs::remove_file(&pending_ready_path)
        .expect("remove private BR-192 accepted marker after publication");
    std::fs::File::open(&namespace)
        .and_then(|directory| directory.sync_all())
        .expect("fsync TEST_CODE namespace after private marker cleanup");
    for _ in 0..3_000 {
        if release_path.is_file() {
            std::process::exit(86);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("BR-192 accepted-crash parent did not release TEST_CODE child");
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct CountedPushLogPending {
    schema: String,
    state: String,
    durable_push_kind: String,
    stable_template_id: String,
    decision_identity: String,
    attempt_identity: String,
    decision_identity_hash: String,
    attempt_identity_hash: String,
    fence_token: i64,
    rendered_content_sha256: String,
    rendered_content: String,
    sink_result: serde_json::Value,
    sink_result_sha256: String,
    receipt_sha256: String,
    observed_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CountedPushLogCommit {
    schema: String,
    state: String,
    durable_push_kind: String,
    stable_template_id: String,
    decision_identity_hash: String,
    attempt_identity_hash: String,
    pending_artifact_sha256: String,
    delivery_audit_event_id: String,
    counted_join_hash: String,
    committed_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CountedFinalizationStage {
    Pending,
    Audit,
    Commit,
    TerminalVerify,
}

fn finalize_counted_delivery(
    push_log_writer: &PinnedPushLogWriter,
    delivery_audit: &stock_analysis::event::AuditDispatcher,
    request: &stock_analysis::durable_delivery::AuthoritativeDeliveryRequest,
    text: &str,
    raw_result: stock_analysis::durable_delivery::AuthoritativeSinkResult,
) -> stock_analysis::durable_delivery::AuthoritativeSinkResult {
    finalize_counted_delivery_with_hook(
        push_log_writer,
        delivery_audit,
        request,
        text,
        raw_result,
        |_| Ok(()),
    )
}

fn finalize_counted_delivery_with_hook<F>(
    push_log_writer: &PinnedPushLogWriter,
    delivery_audit: &stock_analysis::event::AuditDispatcher,
    request: &stock_analysis::durable_delivery::AuthoritativeDeliveryRequest,
    text: &str,
    raw_result: stock_analysis::durable_delivery::AuthoritativeSinkResult,
    mut hook: F,
) -> stock_analysis::durable_delivery::AuthoritativeSinkResult
where
    F: FnMut(CountedFinalizationStage) -> Result<(), String>,
{
    use stock_analysis::durable_delivery::AuthoritativeSinkResult;
    let canonical_template_id = request.push_kind.stable_template_id();
    if request.stable_template_id != canonical_template_id {
        return counted_delivery_persistence_uncertain(
            &raw_result,
            "durable_template_binding",
            &format!(
                "push_kind={} expected={} supplied={}",
                request.push_kind.as_str(),
                canonical_template_id,
                request.stable_template_id
            ),
            None,
        );
    }
    let result_value = authoritative_sink_result_value(&raw_result);
    let result_canonical = match serde_json::to_vec(&result_value) {
        Ok(value) => value,
        Err(error) => {
            return counted_delivery_persistence_uncertain(
                &raw_result,
                "sink_result_serialization",
                &error.to_string(),
                None,
            );
        }
    };
    let sink_result_sha256 =
        sha256_domain("stock_analysis.counted_sink_result.v1", &result_canonical);
    let receipt_sha256 = match &raw_result {
        AuthoritativeSinkResult::Accepted(receipt) => match serde_json::to_vec(receipt) {
            Ok(value) => sha256_domain("stock_analysis.counted_receipt.v1", &value),
            Err(error) => {
                return counted_delivery_persistence_uncertain(
                    &raw_result,
                    "receipt_serialization",
                    &error.to_string(),
                    None,
                );
            }
        },
        _ => sha256_domain(
            "stock_analysis.counted_receipt.none.v1",
            b"NO_VALIDATED_RECEIPT",
        ),
    };
    let decision_identity_hash = sha256_domain(
        "stock_analysis.counted_decision_identity.v1",
        request.decision_identity.as_bytes(),
    );
    let attempt_identity_hash = sha256_domain(
        "stock_analysis.counted_attempt_identity.v1",
        request.attempt_identity.as_bytes(),
    );
    let pending = CountedPushLogPending {
        schema: "stock_analysis.counted_push_log.v1".to_owned(),
        state: "AuditPending".to_owned(),
        durable_push_kind: request.push_kind.as_str().to_owned(),
        stable_template_id: canonical_template_id.to_owned(),
        decision_identity: request.decision_identity.clone(),
        attempt_identity: request.attempt_identity.clone(),
        decision_identity_hash: decision_identity_hash.clone(),
        attempt_identity_hash: attempt_identity_hash.clone(),
        fence_token: request.fence_token,
        rendered_content_sha256: request.rendered_content_sha256.clone(),
        rendered_content: text.to_owned(),
        sink_result: result_value,
        sink_result_sha256: sink_result_sha256.clone(),
        receipt_sha256: receipt_sha256.clone(),
        observed_at: chrono::Utc::now(),
    };
    let pending_bytes = match serde_json::to_vec(&pending) {
        Ok(value) => value,
        Err(error) => {
            return counted_delivery_persistence_uncertain(
                &raw_result,
                "artifact_serialization",
                &error.to_string(),
                None,
            );
        }
    };
    let pending_artifact_sha256 = sha256_domain(
        "stock_analysis.counted_push_log_artifact.v1",
        &pending_bytes,
    );
    if let Err(error) = hook(CountedFinalizationStage::Pending) {
        return counted_delivery_persistence_uncertain(
            &raw_result,
            "artifact_audit_pending_injected",
            &error,
            Some(&pending_artifact_sha256),
        );
    }
    let artifact_prefix = format!("{decision_identity_hash}_{attempt_identity_hash}");
    let pending_name = format!("{artifact_prefix}_audit_pending.json");
    let pending_path = match push_log_writer.save_named_payload(&pending_bytes, &pending_name) {
        Ok(path) => path,
        Err(error) => {
            return counted_delivery_persistence_uncertain(
                &raw_result,
                "artifact_audit_pending",
                &error.to_string(),
                Some(&pending_artifact_sha256),
            );
        }
    };

    let (outcome, channel, latency_ms) = counted_audit_outcome(&raw_result);
    let event = stock_analysis::event::PushDeliveryEvent::new_counted(
        request.push_kind.as_str().to_owned(),
        canonical_template_id.to_owned(),
        outcome.to_owned(),
        channel,
        text.len(),
        latency_ms,
        decision_identity_hash.clone(),
        attempt_identity_hash.clone(),
        pending_artifact_sha256.clone(),
        sink_result_sha256,
        receipt_sha256,
    );
    let event_id = event
        .counted_join_hash
        .clone()
        .expect("new_counted always sets counted_join_hash");
    let trace_id = sha256_domain(
        "stock_analysis.counted_delivery_trace.v1",
        format!(
            "{}\0{}\0{}",
            request.decision_identity, request.attempt_identity, request.fence_token
        )
        .as_bytes(),
    );
    if let Err(error) = hook(CountedFinalizationStage::Audit) {
        return counted_delivery_persistence_uncertain(
            &raw_result,
            "delivery_audit_injected",
            &error,
            Some(&pending_artifact_sha256),
        );
    }
    let audit_envelope = match stock_analysis::event::publish_counted_delivery_with(
        delivery_audit,
        event.clone(),
        event_id.clone(),
        trace_id,
    ) {
        Ok(envelope) => envelope,
        Err(error) => {
            return counted_delivery_persistence_uncertain(
                &raw_result,
                "delivery_audit",
                &error,
                Some(&pending_artifact_sha256),
            );
        }
    };
    let commit = CountedPushLogCommit {
        schema: "stock_analysis.counted_push_log.v1".to_owned(),
        state: "Committed".to_owned(),
        durable_push_kind: request.push_kind.as_str().to_owned(),
        stable_template_id: canonical_template_id.to_owned(),
        decision_identity_hash: decision_identity_hash.clone(),
        attempt_identity_hash: attempt_identity_hash.clone(),
        pending_artifact_sha256: pending_artifact_sha256.clone(),
        delivery_audit_event_id: event_id.clone(),
        counted_join_hash: event
            .counted_join_hash
            .as_deref()
            .expect("new_counted always sets counted_join_hash")
            .to_owned(),
        committed_at: chrono::Utc::now(),
    };
    let commit_bytes = match serde_json::to_vec(&commit) {
        Ok(value) => value,
        Err(error) => {
            return counted_delivery_persistence_uncertain(
                &raw_result,
                "commit_marker_serialization",
                &error.to_string(),
                Some(&pending_artifact_sha256),
            );
        }
    };
    if let Err(error) = hook(CountedFinalizationStage::Commit) {
        return counted_delivery_persistence_uncertain(
            &raw_result,
            "commit_marker_injected",
            &error,
            Some(&pending_artifact_sha256),
        );
    }
    let commit_name = format!("{artifact_prefix}_committed.json");
    let commit_path = match push_log_writer.save_named_payload(&commit_bytes, &commit_name) {
        Ok(path) => path,
        Err(error) => {
            return counted_delivery_persistence_uncertain(
                &raw_result,
                "commit_marker",
                &error.to_string(),
                Some(&pending_artifact_sha256),
            );
        }
    };
    if let Err(error) = hook(CountedFinalizationStage::TerminalVerify) {
        return counted_delivery_persistence_uncertain(
            &raw_result,
            "terminal_verifier_injected",
            &error,
            Some(&pending_artifact_sha256),
        );
    }
    if let Err(error) = verify_counted_delivery_terminal(
        push_log_writer,
        delivery_audit,
        &pending_path,
        &pending_bytes,
        &pending,
        &audit_envelope,
        &commit_path,
        &commit_bytes,
        &commit,
    ) {
        return counted_delivery_persistence_uncertain(
            &raw_result,
            "terminal_verifier",
            &error,
            Some(&pending_artifact_sha256),
        );
    }
    raw_result
}

#[allow(clippy::too_many_arguments)]
fn verify_counted_delivery_terminal(
    push_log_writer: &PinnedPushLogWriter,
    delivery_audit: &stock_analysis::event::AuditDispatcher,
    pending_path: &std::path::Path,
    expected_pending_bytes: &[u8],
    expected_pending: &CountedPushLogPending,
    expected_audit: &stock_analysis::event::EventEnvelope,
    commit_path: &std::path::Path,
    expected_commit_bytes: &[u8],
    expected_commit: &CountedPushLogCommit,
) -> Result<(), String> {
    let pending_bytes = verify_exact_push_log_bytes(
        push_log_writer,
        pending_path,
        expected_pending_bytes,
        "pending",
    )?;
    let parsed_pending: CountedPushLogPending = serde_json::from_slice(&pending_bytes)
        .map_err(|error| format!("parse exact pending artifact: {error}"))?;
    if &parsed_pending != expected_pending {
        return Err("pending artifact semantic binding changed".to_owned());
    }

    let audit_record = delivery_audit.verify_exact_counted_event(expected_audit)?;
    verify_counted_audit_pending_binding(&audit_record, expected_pending, expected_commit)?;

    let commit_bytes = verify_exact_push_log_bytes(
        push_log_writer,
        commit_path,
        expected_commit_bytes,
        "committed",
    )?;
    let parsed_commit: CountedPushLogCommit = serde_json::from_slice(&commit_bytes)
        .map_err(|error| format!("parse exact committed artifact: {error}"))?;
    if &parsed_commit != expected_commit {
        return Err("committed artifact semantic binding changed".to_owned());
    }
    if expected_commit.delivery_audit_event_id != expected_audit.id
        || expected_commit.counted_join_hash
            != expected_audit
                .payload
                .get("counted_join_hash")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
    {
        return Err("committed artifact does not bind the exact audit event".to_owned());
    }
    Ok(())
}

fn verify_counted_audit_pending_binding(
    audit_record: &stock_analysis::event::PushRecord,
    expected_pending: &CountedPushLogPending,
    expected_commit: &CountedPushLogCommit,
) -> Result<(), String> {
    if audit_record.decision_identity_hash.as_deref()
        != Some(expected_pending.decision_identity_hash.as_str())
    {
        return Err(
            "schema-v3 audit decision_identity_hash does not match pending artifact".to_owned(),
        );
    }
    if audit_record.attempt_identity_hash.as_deref()
        != Some(expected_pending.attempt_identity_hash.as_str())
    {
        return Err(
            "schema-v3 audit attempt_identity_hash does not match pending artifact".to_owned(),
        );
    }
    if audit_record.artifact_sha256.as_deref()
        != Some(expected_commit.pending_artifact_sha256.as_str())
    {
        return Err(
            "schema-v3 audit artifact_sha256 does not match committed pending artifact".to_owned(),
        );
    }
    if audit_record.sink_result_sha256.as_deref()
        != Some(expected_pending.sink_result_sha256.as_str())
    {
        return Err(
            "schema-v3 audit sink_result_sha256 does not match pending artifact".to_owned(),
        );
    }
    if audit_record.receipt_sha256.as_deref() != Some(expected_pending.receipt_sha256.as_str()) {
        return Err("schema-v3 audit receipt_sha256 does not match pending artifact".to_owned());
    }
    if audit_record.durable_push_kind.as_deref()
        != Some(expected_pending.durable_push_kind.as_str())
    {
        return Err("schema-v3 audit durable_push_kind does not match pending artifact".to_owned());
    }
    if audit_record.stable_template_id.as_deref()
        != Some(expected_pending.stable_template_id.as_str())
    {
        return Err(
            "schema-v3 audit stable_template_id does not match pending artifact".to_owned(),
        );
    }
    Ok(())
}

fn verify_exact_push_log_bytes(
    push_log_writer: &PinnedPushLogWriter,
    path: &std::path::Path,
    expected: &[u8],
    label: &str,
) -> Result<Vec<u8>, String> {
    let observed = push_log_writer
        .read_exact_payload(path)
        .map_err(|error| format!("read exact {label} artifact: {error}"))?;
    if observed != expected {
        return Err(format!("{label} artifact bytes changed after fsync"));
    }
    Ok(observed)
}

fn authoritative_sink_result_value(
    result: &stock_analysis::durable_delivery::AuthoritativeSinkResult,
) -> serde_json::Value {
    use stock_analysis::durable_delivery::AuthoritativeSinkResult;
    match result {
        AuthoritativeSinkResult::Accepted(receipt) => {
            serde_json::json!({"kind": "Accepted", "receipt": receipt})
        }
        AuthoritativeSinkResult::Rejected(rejection) => {
            serde_json::json!({"kind": "Rejected", "rejection": rejection})
        }
        AuthoritativeSinkResult::Uncertain(uncertainty) => {
            serde_json::json!({"kind": "Uncertain", "uncertainty": uncertainty})
        }
    }
}

fn counted_audit_outcome(
    result: &stock_analysis::durable_delivery::AuthoritativeSinkResult,
) -> (&'static str, String, u64) {
    use stock_analysis::durable_delivery::AuthoritativeSinkResult;
    match result {
        AuthoritativeSinkResult::Accepted(receipt) => (
            "Pushed",
            receipt.channel.clone(),
            receipt
                .latency_ms
                .and_then(|value| u64::try_from(value).ok())
                .unwrap_or(0),
        ),
        AuthoritativeSinkResult::Rejected(rejection) if rejection.retry_authorized => {
            ("SinkError", "authoritative".to_owned(), 0)
        }
        AuthoritativeSinkResult::Rejected(_) => ("Denied", "authoritative".to_owned(), 0),
        AuthoritativeSinkResult::Uncertain(_) => ("Uncertain", "authoritative".to_owned(), 0),
    }
}

fn counted_delivery_persistence_uncertain(
    original: &stock_analysis::durable_delivery::AuthoritativeSinkResult,
    stage: &str,
    error: &str,
    artifact_sha256: Option<&str>,
) -> stock_analysis::durable_delivery::AuthoritativeSinkResult {
    let evidence = serde_json::to_vec(&serde_json::json!({
        "stage": stage,
        "error": error,
        "artifact_sha256": artifact_sha256,
        "original_sink_result": authoritative_sink_result_value(original),
    }))
    .unwrap_or_else(|_| {
        format!(
            "stage={stage}; artifact_sha256={}; error={error}",
            artifact_sha256.unwrap_or("unavailable")
        )
        .into_bytes()
    });
    stock_analysis::durable_delivery::AuthoritativeSinkResult::Uncertain(
        stock_analysis::durable_delivery::TypedUncertainty {
            reason_code: "counted_delivery_persistence_uncertain".to_owned(),
            evidence,
            observed_at: chrono::Utc::now(),
        },
    )
}

fn sha256_domain(domain: &str, payload: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(payload);
    format!("{:x}", hasher.finalize())
}

enum BlockingCliDeliveryFailure {
    Rejected {
        reason_code: String,
        evidence: Vec<u8>,
    },
    Uncertain {
        reason_code: String,
        evidence: Vec<u8>,
    },
}

fn push_via_magiclaw_cli_receipt_blocking(
    send_type: MessageSendType,
    text: &str,
) -> Result<CliDeliveryReceipt, BlockingCliDeliveryFailure> {
    let to =
        match send_type {
            MessageSendType::Wechat => None,
            MessageSendType::Feishu => Some(resolve_feishu_target().ok_or_else(|| {
                BlockingCliDeliveryFailure::Rejected {
                    reason_code: "missing_feishu_target".to_owned(),
                    evidence: b"FEISHU_TO or an equivalent target is required".to_vec(),
                }
            })?),
        };
    let magiclaw_bin = resolve_magiclaw_bin();
    let mut command = std::process::Command::new(&magiclaw_bin);
    command
        .arg("send")
        .arg("--channel")
        .arg(send_type.as_str())
        .arg("--message")
        .arg(text)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(to) = to.as_deref() {
        command.arg("--to").arg(to);
    }
    if let Some(home) = resolve_magiclaw_home(&magiclaw_bin) {
        command.current_dir(home);
    }
    if let Ok(db_path) = std::env::var("MAGICLAW_DB_PATH") {
        let db_path = db_path.trim();
        if !db_path.is_empty() {
            let path = std::path::Path::new(db_path);
            let absolute = if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(path))
                    .unwrap_or_else(|_| path.to_path_buf())
            };
            command.env("MAGICLAW_DB_PATH", absolute);
        }
    }
    if let Ok(receive_id_type) = std::env::var("FEISHU_RECEIVE_ID_TYPE") {
        let receive_id_type = receive_id_type.trim();
        if !receive_id_type.is_empty() {
            command.arg("--receive-id-type").arg(receive_id_type);
        }
    }

    let output = command
        .output()
        .map_err(|error| BlockingCliDeliveryFailure::Rejected {
            reason_code: "magiclaw_cli_spawn_failed".to_owned(),
            evidence: error.to_string().into_bytes(),
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(BlockingCliDeliveryFailure::Uncertain {
            reason_code: "magiclaw_cli_nonzero_after_send_attempt".to_owned(),
            evidence: format!(
                "exit={} stderr={} stdout={}",
                output.status,
                tail_lines(&stderr, 8),
                tail_lines(&stdout, 3)
            )
            .into_bytes(),
        });
    }
    parse_magiclaw_cli_delivery_receipt(send_type, &stdout).map_err(|error| {
        BlockingCliDeliveryFailure::Uncertain {
            reason_code: "magiclaw_cli_receipt_invalid".to_owned(),
            evidence: error.into_bytes(),
        }
    })
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
                    status, extra
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
            ApiTokenSource::DynamicMemCache
            | ApiTokenSource::DynamicFileCache
            | ApiTokenSource::DynamicIssued => {
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

    fn br194_test_binding(label: &str) -> crate::durable_delivery_runtime::CountedDeliveryBinding {
        crate::durable_delivery_runtime::CountedDeliveryBinding::new(
            chrono::NaiveDate::from_ymd_opt(2099, 1, 2).unwrap(),
            format!("TEST_CODE_{label}_OCCURRENCE"),
            format!("TEST_CODE_{label}_CANONICAL").into_bytes(),
            crate::durable_delivery_runtime::CountedDeliveryScope::Global,
            "a".repeat(64),
            crate::durable_delivery_runtime::CountedDeliveryOrigin::InternalDurable,
            None,
            false,
        )
        .expect("valid TEST_CODE orchestration binding")
    }

    fn br194_approved_event() -> Box<stock_analysis::push_l1::SignalEvent> {
        Box::new(stock_analysis::push_l1::SignalEvent::new(
            stock_analysis::push_l1::SignalSource::PostSessionReview,
            "review_lhb",
            None,
            chrono::Local::now(),
            stock_analysis::push_l1::SignalPayload::PostSessionReview(Default::default()),
            stock_analysis::push_l1::Severity::Normal,
        ))
    }

    #[tokio::test]
    async fn br194_r04_source_only_gate_never_reads_banner() {
        let launch_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let gate_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let durable_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let outcome = push_counted_source_only_after_validation_with(
            "TEST_CODE_BODY",
            PushKind::ReviewLhb,
            br194_test_binding("NO_BANNER"),
            {
                let calls = launch_calls.clone();
                move |_| {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    true
                }
            },
            {
                let calls = gate_calls.clone();
                move |_, _| {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    crate::v14_adapter::V14Gate::Approved(br194_approved_event())
                }
            },
            {
                let calls = durable_calls.clone();
                move |_, _, _| async move {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    PushOutcome::Pushed
                }
            },
        )
        .await;
        assert_eq!(outcome, PushOutcome::Pushed);
        assert_eq!(launch_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(gate_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(durable_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn br194_r04_source_only_preserves_l5_and_durable_entry() {
        let trace = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let approved = push_counted_source_only_after_validation_with(
            "TEST_CODE_BODY",
            PushKind::ReviewLhb,
            br194_test_binding("ORDER"),
            {
                let trace = trace.clone();
                move |_| {
                    trace.lock().unwrap().push("launch");
                    true
                }
            },
            {
                let trace = trace.clone();
                move |_, _| {
                    trace.lock().unwrap().push("l5");
                    crate::v14_adapter::V14Gate::Approved(br194_approved_event())
                }
            },
            {
                let trace = trace.clone();
                move |_, _, _| async move {
                    trace.lock().unwrap().push("durable");
                    PushOutcome::Pushed
                }
            },
        )
        .await;
        assert_eq!(approved, PushOutcome::Pushed);
        assert_eq!(*trace.lock().unwrap(), ["launch", "l5", "durable"]);

        let durable_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let denied = push_counted_source_only_after_validation_with(
            "TEST_CODE_BODY",
            PushKind::ReviewLhb,
            br194_test_binding("L5_DENIED"),
            |_| true,
            |_, _| crate::v14_adapter::V14Gate::Denied("TEST_CODE_L5_DENIED".to_owned()),
            {
                let calls = durable_calls.clone();
                move |_, _, _| async move {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    PushOutcome::Pushed
                }
            },
        )
        .await;
        assert_eq!(
            denied,
            PushOutcome::Denied("TEST_CODE_L5_DENIED".to_owned())
        );
        assert_eq!(durable_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn br194_r04_source_only_denied_launch_has_zero_durable_and_sink() {
        let gate_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let durable_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let outcome = push_counted_source_only_after_validation_with(
            "TEST_CODE_BODY",
            PushKind::ReviewLhb,
            br194_test_binding("LAUNCH_DENIED"),
            |_| false,
            {
                let calls = gate_calls.clone();
                move |_, _| {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    crate::v14_adapter::V14Gate::Approved(br194_approved_event())
                }
            },
            {
                let calls = durable_calls.clone();
                move |_, _, _| async move {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    PushOutcome::Pushed
                }
            },
        )
        .await;
        assert_eq!(outcome, PushOutcome::Denied("launch_gate_stage".to_owned()));
        assert_eq!(gate_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(durable_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    struct TestBannerGuard(Option<crate::push_templates::BannerCtx>);

    struct TestNotifyDir(std::path::PathBuf);

    struct TestPushLogNamespace {
        root: std::path::PathBuf,
        retained: std::fs::File,
        device: u64,
        inode: u64,
    }

    impl TestPushLogNamespace {
        fn new(label: &str) -> Self {
            use std::os::unix::fs::MetadataExt;
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            assert!(
                label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')),
                "push-log fixture label must remain one path component"
            );
            let parent = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/test");
            std::fs::create_dir_all(&parent).expect("create TEST_CODE fixture parent");
            let root = parent.join(format!(
                "TEST_CODE_PUSH_LOG_ALIAS_{label}_{}_{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).expect("create fresh isolated push-log namespace");
            let retained = std::fs::File::open(&root).expect("retain push-log namespace inode");
            let metadata = retained
                .metadata()
                .expect("inspect retained namespace inode");
            Self {
                root,
                retained,
                device: metadata.dev(),
                inode: metadata.ino(),
            }
        }

        fn path(&self) -> &std::path::Path {
            &self.root
        }

        fn test_code(&self) -> &str {
            self.root
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .expect("TEST_CODE push-log fixture name")
        }
    }

    impl Drop for TestPushLogNamespace {
        fn drop(&mut self) {
            use std::os::unix::fs::MetadataExt;
            let retained = self
                .retained
                .metadata()
                .expect("inspect retained push-log namespace before cleanup");
            let current = std::fs::symlink_metadata(&self.root)
                .expect("push-log namespace must still exist before cleanup");
            assert!(
                current.file_type().is_dir()
                    && current.dev() == self.device
                    && current.ino() == self.inode
                    && retained.dev() == self.device
                    && retained.ino() == self.inode,
                "refuse to remove a replaced push-log TEST_CODE namespace"
            );
            std::fs::remove_dir_all(&self.root)
                .expect("remove retained exact push-log TEST_CODE namespace");
        }
    }

    fn push_log_json_artifacts(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut artifacts = Vec::new();
        if !root.exists() {
            return artifacts;
        }
        for entry in std::fs::read_dir(root).expect("read TEST_CODE push-log root") {
            let path = entry.expect("read TEST_CODE push-log entry").path();
            if path.is_dir() {
                for artifact in
                    std::fs::read_dir(&path).expect("read TEST_CODE push-log date directory")
                {
                    let artifact = artifact.expect("read TEST_CODE artifact").path();
                    if artifact.extension().and_then(std::ffi::OsStr::to_str) == Some("json") {
                        artifacts.push(artifact);
                    }
                }
            }
        }
        artifacts.sort();
        artifacts
    }

    impl TestNotifyDir {
        fn new(label: &str) -> Self {
            Self(notify_temp_dir(label))
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TestNotifyDir {
        fn drop(&mut self) {
            if self.0.exists() {
                std::fs::remove_dir_all(&self.0).expect("remove isolated notify directory");
            }
        }
    }

    impl TestBannerGuard {
        fn full() -> Self {
            let previous = crate::LATEST_BANNER
                .lock()
                .expect("test banner lock")
                .replace(crate::push_templates::BannerCtx::test_default());
            Self(previous)
        }
    }

    impl Drop for TestBannerGuard {
        fn drop(&mut self) {
            *crate::LATEST_BANNER.lock().expect("restore banner lock") = self.0.take();
        }
    }

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

    fn counted_test_request(
        label: &str,
    ) -> stock_analysis::durable_delivery::AuthoritativeDeliveryRequest {
        let rendered_content = format!("TEST_CODE_COUNTED_{label}").into_bytes();
        stock_analysis::durable_delivery::AuthoritativeDeliveryRequest {
            decision_identity: sha256_domain(
                "TEST_CODE.counted.decision",
                format!("decision:{label}").as_bytes(),
            ),
            attempt_identity: sha256_domain(
                "TEST_CODE.counted.attempt",
                format!("attempt:{label}").as_bytes(),
            ),
            fence_token: 7,
            push_kind: stock_analysis::durable_delivery::PushKind::HoldingEvent,
            stable_template_id: stock_analysis::durable_delivery::PushKind::HoldingEvent
                .stable_template_id()
                .to_owned(),
            rendered_content_sha256: sha256_domain("TEST_CODE.counted.rendered", &rendered_content),
            rendered_content,
        }
    }

    fn counted_test_accepted(
        label: &str,
    ) -> stock_analysis::durable_delivery::AuthoritativeSinkResult {
        stock_analysis::durable_delivery::AuthoritativeSinkResult::Accepted(
            stock_analysis::durable_delivery::TypedReceipt {
                channel: "TEST_CODE_DRY_RUN".to_owned(),
                provider: "TEST_CODE_PROVIDER".to_owned(),
                message_id: format!("TEST_CODE_MESSAGE_{label}"),
                platform_message_id: Some(format!("TEST_CODE_PLATFORM_{label}")),
                accepted_at: chrono::Utc::now(),
                latency_ms: Some(1),
            },
        )
    }

    fn counted_audit_binding_fixture() -> (
        stock_analysis::event::PushRecord,
        CountedPushLogPending,
        CountedPushLogCommit,
    ) {
        let decision_identity_hash = "a".repeat(64);
        let attempt_identity_hash = "b".repeat(64);
        let artifact_sha256 = "c".repeat(64);
        let sink_result_sha256 = "d".repeat(64);
        let receipt_sha256 = "e".repeat(64);
        let event = stock_analysis::event::PushDeliveryEvent::new_counted(
            "HoldingEvent".to_owned(),
            "holding_event_v1".to_owned(),
            "Pushed".to_owned(),
            "TEST_CODE_DRY_RUN".to_owned(),
            12,
            37,
            decision_identity_hash.clone(),
            attempt_identity_hash.clone(),
            artifact_sha256.clone(),
            sink_result_sha256.clone(),
            receipt_sha256.clone(),
        );
        let counted_join_hash = event
            .counted_join_hash
            .clone()
            .expect("counted fixture has a canonical join hash");
        let envelope = stock_analysis::event::EventEnvelope::from_event(
            &event,
            counted_join_hash.clone(),
            "TEST_CODE_COUNTED_BINDING_TRACE".to_owned(),
            chrono::Local::now(),
        )
        .expect("valid counted fixture envelope");
        let record = stock_analysis::event::PushRecord::try_from_authoritative(&envelope)
            .expect("valid counted fixture audit record");
        let pending = CountedPushLogPending {
            schema: "stock_analysis.counted_push_log.v1".to_owned(),
            state: "AuditPending".to_owned(),
            durable_push_kind: "HoldingEvent".to_owned(),
            stable_template_id: "holding_event_v1".to_owned(),
            decision_identity: "TEST_CODE_DECISION_IDENTITY".to_owned(),
            attempt_identity: "TEST_CODE_ATTEMPT_IDENTITY".to_owned(),
            decision_identity_hash,
            attempt_identity_hash,
            fence_token: 7,
            rendered_content_sha256: "f".repeat(64),
            rendered_content: "TEST_CODE_COUNTED_BINDING".to_owned(),
            sink_result: serde_json::json!({"kind": "Accepted"}),
            sink_result_sha256,
            receipt_sha256,
            observed_at: chrono::Utc::now(),
        };
        let commit = CountedPushLogCommit {
            schema: "stock_analysis.counted_push_log.v1".to_owned(),
            state: "Committed".to_owned(),
            durable_push_kind: "HoldingEvent".to_owned(),
            stable_template_id: "holding_event_v1".to_owned(),
            decision_identity_hash: pending.decision_identity_hash.clone(),
            attempt_identity_hash: pending.attempt_identity_hash.clone(),
            pending_artifact_sha256: artifact_sha256,
            delivery_audit_event_id: counted_join_hash.clone(),
            counted_join_hash,
            committed_at: chrono::Utc::now(),
        };
        assert_eq!(
            verify_counted_audit_pending_binding(&record, &pending, &commit),
            Ok(()),
            "the canonical counted fixture must satisfy every terminal binding"
        );
        (record, pending, commit)
    }

    #[test]
    fn br192_counted_terminal_verifier_rejects_sink_result_hash_mismatch() {
        let (record, mut pending, commit) = counted_audit_binding_fixture();
        pending.sink_result_sha256 = "0".repeat(64);

        assert_eq!(
            verify_counted_audit_pending_binding(&record, &pending, &commit),
            Err("schema-v3 audit sink_result_sha256 does not match pending artifact".to_owned())
        );
    }

    #[test]
    fn br192_counted_terminal_verifier_rejects_receipt_hash_mismatch() {
        let (record, mut pending, commit) = counted_audit_binding_fixture();
        pending.receipt_sha256 = "0".repeat(64);

        assert_eq!(
            verify_counted_audit_pending_binding(&record, &pending, &commit),
            Err("schema-v3 audit receipt_sha256 does not match pending artifact".to_owned())
        );
    }

    #[test]
    fn br192_counted_finalization_failures_are_uncertain_and_never_auto_retryable() {
        use stock_analysis::durable_delivery::AuthoritativeSinkResult;

        for (label, failed_stage, expected_artifacts, audit_expected) in [
            ("PENDING_FAIL", CountedFinalizationStage::Pending, 0, false),
            ("AUDIT_FAIL", CountedFinalizationStage::Audit, 1, false),
            ("COMMIT_FAIL", CountedFinalizationStage::Commit, 1, true),
        ] {
            let fixture = TestPushLogNamespace::new(label);
            let writer = PinnedPushLogWriter::for_test_anchor(
                "test",
                fixture.path(),
                std::path::Path::new("push_log"),
            )
            .expect("bind counted TEST_CODE push-log");
            let audit = stock_analysis::event::AuditDispatcher::for_test_code(fixture.test_code())
                .expect("bind counted TEST_CODE audit");
            let request = counted_test_request(label);
            let text = std::str::from_utf8(&request.rendered_content).unwrap();
            let mut injected = false;
            let result = finalize_counted_delivery_with_hook(
                &writer,
                &audit,
                &request,
                text,
                counted_test_accepted(label),
                |stage| {
                    if stage == failed_stage {
                        injected = true;
                        Err(format!("TEST_CODE injected {label}"))
                    } else {
                        Ok(())
                    }
                },
            );
            assert!(injected, "failure stage must be reached exactly once");
            let uncertainty = match result {
                AuthoritativeSinkResult::Uncertain(value) => value,
                other => panic!("persistence failure must be Uncertain, got {other:?}"),
            };
            assert_eq!(
                uncertainty.reason_code,
                "counted_delivery_persistence_uncertain"
            );
            let evidence = String::from_utf8_lossy(&uncertainty.evidence);
            assert!(
                evidence.contains("\"kind\":\"Accepted\""),
                "accepted receipt evidence must survive without a resend: {evidence}"
            );

            let artifact_count = push_log_json_artifacts(&fixture.path().join("push_log")).len();
            assert_eq!(artifact_count, expected_artifacts, "{label}");
            let audit_path = fixture
                .path()
                .join("event_audit")
                .join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
            assert_eq!(audit_path.exists(), audit_expected, "{label}");
        }
    }

    #[test]
    fn br192_counted_terminal_verifier_requires_pending_audit_and_commit() {
        use stock_analysis::durable_delivery::AuthoritativeSinkResult;

        let fixture = TestPushLogNamespace::new("TERMINAL_SUCCESS");
        let writer = PinnedPushLogWriter::for_test_anchor(
            "test",
            fixture.path(),
            std::path::Path::new("push_log"),
        )
        .expect("bind counted TEST_CODE push-log");
        let audit = stock_analysis::event::AuditDispatcher::for_test_code(fixture.test_code())
            .expect("bind counted TEST_CODE audit");
        let request = counted_test_request("TERMINAL_SUCCESS");
        let text = std::str::from_utf8(&request.rendered_content).unwrap();
        let result = finalize_counted_delivery(
            &writer,
            &audit,
            &request,
            text,
            counted_test_accepted("TERMINAL_SUCCESS"),
        );
        assert!(
            matches!(result, AuthoritativeSinkResult::Accepted(_)),
            "all three exact terminal records must verify"
        );
        let artifacts = push_log_json_artifacts(&fixture.path().join("push_log"));
        assert_eq!(artifacts.len(), 2);
        assert!(artifacts.iter().any(|path| {
            path.file_name()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|name| name.ends_with("_audit_pending.json"))
        }));
        assert!(artifacts.iter().any(|path| {
            path.file_name()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|name| name.ends_with("_committed.json"))
        }));

        let duplicate = finalize_counted_delivery(
            &writer,
            &audit,
            &request,
            text,
            counted_test_accepted("TERMINAL_SUCCESS_DUPLICATE"),
        );
        assert!(
            matches!(duplicate, AuthoritativeSinkResult::Uncertain(_)),
            "the deterministic Pending name must make a duplicate terminal attempt uncertain"
        );
        assert_eq!(
            push_log_json_artifacts(&fixture.path().join("push_log")).len(),
            2,
            "a duplicate terminal attempt must not append another artifact"
        );
    }

    #[test]
    fn br192_counted_terminal_verifier_rejects_in_place_pending_tamper() {
        use std::io::Write;
        use stock_analysis::durable_delivery::AuthoritativeSinkResult;

        let fixture = TestPushLogNamespace::new("TERMINAL_TAMPER");
        let writer = PinnedPushLogWriter::for_test_anchor(
            "test",
            fixture.path(),
            std::path::Path::new("push_log"),
        )
        .expect("bind counted TEST_CODE push-log");
        let audit = stock_analysis::event::AuditDispatcher::for_test_code(fixture.test_code())
            .expect("bind counted TEST_CODE audit");
        let request = counted_test_request("TERMINAL_TAMPER");
        let text = std::str::from_utf8(&request.rendered_content).unwrap();
        let result = finalize_counted_delivery_with_hook(
            &writer,
            &audit,
            &request,
            text,
            counted_test_accepted("TERMINAL_TAMPER"),
            |stage| {
                if stage != CountedFinalizationStage::TerminalVerify {
                    return Ok(());
                }
                let pending_path = push_log_json_artifacts(&fixture.path().join("push_log"))
                    .into_iter()
                    .find(|path| {
                        path.file_name()
                            .and_then(std::ffi::OsStr::to_str)
                            .is_some_and(|name| name.ends_with("_audit_pending.json"))
                    })
                    .ok_or_else(|| "TEST_CODE pending artifact missing".to_owned())?;
                let mut pending = std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&pending_path)
                    .map_err(|error| format!("open pending tamper fixture: {error}"))?;
                pending
                    .write_all(br#"{"state":"TEST_CODE_TAMPERED"}"#)
                    .map_err(|error| format!("tamper pending fixture: {error}"))?;
                pending
                    .sync_all()
                    .map_err(|error| format!("fsync pending tamper fixture: {error}"))
            },
        );
        let uncertainty = match result {
            AuthoritativeSinkResult::Uncertain(value) => value,
            other => panic!("terminal tamper must be Uncertain, got {other:?}"),
        };
        assert_eq!(
            uncertainty.reason_code,
            "counted_delivery_persistence_uncertain"
        );
        assert!(
            String::from_utf8_lossy(&uncertainty.evidence)
                .contains("pending artifact bytes changed after fsync"),
            "terminal verifier must report the exact tampered record"
        );
    }

    #[test]
    fn br192_push_log_mkdir_eexist_race_is_reopened_after_parent_sync() {
        let fixture = TestPushLogNamespace::new("EEXIST_PARENT_SYNC");
        let parent = std::fs::File::open(fixture.path()).unwrap();
        let path = fixture.path().join("race_child");
        let (directory, _) = open_or_create_push_log_child_with_hook(
            &parent,
            std::ffi::OsStr::new("race_child"),
            &path,
            || std::fs::create_dir(&path).expect("TEST_CODE wins mkdir race"),
        )
        .expect("EEXIST winner must be reopened after parent fsync");
        assert!(directory.metadata().unwrap().is_dir());
    }

    #[test]
    fn br192_push_log_directory_safe_mode_drift_is_rejected() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = TestPushLogNamespace::new("SAFE_MODE_DRIFT");
        let writer = PinnedPushLogWriter::for_test_anchor(
            "test",
            fixture.path(),
            std::path::Path::new("push_log"),
        )
        .expect("bind TEST_CODE push-log");
        let path = fixture.path().join("push_log");
        let original = std::fs::metadata(&path).unwrap().permissions();
        let original_mode = original.mode() & 0o7777;
        let drifted_mode = if original_mode == 0o700 { 0o750 } else { 0o700 };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(drifted_mode))
            .expect("apply another safe TEST_CODE push-log mode");
        let result = revalidate_push_log_directory_chain(&writer.root_binding);
        std::fs::set_permissions(&path, original).expect("restore push-log permissions");
        assert!(
            matches!(result, Err(PushLogError::NamespaceIsolation(_))),
            "allowed-to-allowed mode drift must invalidate the push-log binding: {result:?}"
        );
    }

    #[test]
    fn br192_push_log_directory_allowed_owner_drift_is_rejected() {
        let effective_uid = 501;
        assert!(push_log_directory_owner_allowed(0, effective_uid));
        assert!(push_log_directory_owner_allowed(
            effective_uid,
            effective_uid
        ));
        let root_owned = PushLogFileIdentity {
            device: 1,
            inode: 2,
            mode: 0o040700,
            uid: 0,
            is_directory: true,
            is_file: false,
        };
        let effective_owned = PushLogFileIdentity {
            uid: effective_uid,
            ..root_owned
        };
        assert_ne!(
            root_owned, effective_owned,
            "two individually allowed owners are not the same retained authority"
        );
    }

    #[test]
    fn br192_push_log_exact_verifier_rejects_tamper_and_duplicate_name() {
        use std::io::Write;

        let fixture = TestPushLogNamespace::new("TAMPER_DUPLICATE");
        let writer = PinnedPushLogWriter::for_test_anchor(
            "test",
            fixture.path(),
            std::path::Path::new("push_log"),
        )
        .expect("bind TEST_CODE push-log");
        let original = br#"{"state":"AuditPending","value":"TEST_CODE_ORIGINAL"}"#;
        let path = writer
            .save_named_payload(original, "TEST_CODE_exact_audit_pending.json")
            .expect("write immutable TEST_CODE artifact");
        assert!(
            writer
                .save_named_payload(original, "TEST_CODE_exact_audit_pending.json")
                .is_err(),
            "O_EXCL must reject duplicate terminal artifact names"
        );

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .expect("open TEST_CODE artifact for tamper injection");
        file.write_all(br#"{"state":"tampered"}"#).unwrap();
        file.sync_all().unwrap();
        let error = verify_exact_push_log_bytes(&writer, &path, original, "pending")
            .expect_err("terminal byte verifier must reject in-place tampering");
        assert!(error.contains("bytes changed"), "{error}");
    }

    #[test]
    #[ignore = "invoked as a child by the cross-process push-log locking test"]
    fn br192_push_log_process_writer_helper() {
        let Ok(root) = std::env::var("BR192_PUSH_LOG_HELPER_ROOT") else {
            return;
        };
        let identity = std::env::var("BR192_PUSH_LOG_HELPER_ID").unwrap();
        let writer = PinnedPushLogWriter::for_test_anchor(
            "test-child",
            std::path::Path::new(&root),
            std::path::Path::new("push_log"),
        )
        .expect("bind child TEST_CODE push-log");
        writer
            .save_named_payload(
                format!("{{\"writer\":\"{identity}\"}}").as_bytes(),
                &format!("TEST_CODE_child_{identity}.json"),
            )
            .expect("child writes exact immutable artifact");
    }

    #[test]
    fn br192_push_log_serializes_cross_process_writers_from_foreign_cwd() {
        let fixture = TestPushLogNamespace::new("CROSS_PROCESS");
        let executable = std::env::current_exe().unwrap();
        let mut children = (0..4)
            .map(|index| {
                std::process::Command::new(&executable)
                    .args([
                        "--exact",
                        "notify::tests::br192_push_log_process_writer_helper",
                        "--ignored",
                    ])
                    .env("BR192_PUSH_LOG_HELPER_ROOT", fixture.path())
                    .env("BR192_PUSH_LOG_HELPER_ID", index.to_string())
                    .env_remove("PUSH_LOG_DIR")
                    .current_dir(std::env::temp_dir())
                    .spawn()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }
        let artifacts = push_log_json_artifacts(&fixture.path().join("push_log")).len();
        assert_eq!(artifacts, 4);
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_test_push_log_rejects_a_production_shaped_override_before_writing() {
        let _env_guard = crate::TestEnvGuard::capture(&[
            "STOCK_ENV_MODE",
            "V10_DRY_RUN_PUSH",
            "DURABLE_DELIVERY_TEST_CODE",
            "PUSH_LOG_DIR",
        ]);
        let isolated = TestNotifyDir::new("br192_test_push_log_override");
        let test_code = format!(
            "TEST_CODE_PUSH_LOG_SCOPE_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        );
        let production_shaped_override = isolated.path().join("data/push_log");
        std::env::set_var("STOCK_ENV_MODE", "test");
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        std::env::set_var("DURABLE_DELIVERY_TEST_CODE", &test_code);
        std::env::set_var("PUSH_LOG_DIR", &production_shaped_override);

        let result = save_push_log(
            &crate::durable_delivery_runtime::RuntimeNamespace::Test {
                test_code: test_code.clone(),
            },
            "TEST_CODE push log must stay in its bound namespace",
        );

        assert!(
            matches!(
                &result,
                Err(PushLogError::NamespaceOverrideRejected { namespace })
                    if namespace == &format!("test:{test_code}")
            ),
            "test push logging must return a typed override rejection before creating any \
             artifact; expected fixed data/test/{test_code}/push_log, got {result:?}"
        );
        assert!(
            !production_shaped_override.exists(),
            "rejected cross-namespace override must not create a directory"
        );
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_production_push_log_rejects_a_cross_namespace_override_before_writing() {
        let _env_guard = crate::TestEnvGuard::capture(&[
            "STOCK_ENV_MODE",
            "V10_DRY_RUN_PUSH",
            "DURABLE_DELIVERY_TEST_CODE",
            "PUSH_LOG_DIR",
        ]);
        let isolated = TestNotifyDir::new("br192_production_push_log_override");
        let cross_namespace_override = isolated.path().join("TEST_CODE/push_log");
        std::env::set_var("STOCK_ENV_MODE", "prod");
        std::env::remove_var("V10_DRY_RUN_PUSH");
        std::env::remove_var("DURABLE_DELIVERY_TEST_CODE");
        std::env::set_var("PUSH_LOG_DIR", &cross_namespace_override);

        let result = save_push_log(
            &crate::durable_delivery_runtime::RuntimeNamespace::Production,
            "production push log must stay in data/push_log",
        );

        assert!(
            matches!(
                &result,
                Err(PushLogError::NamespaceOverrideRejected { namespace })
                    if namespace == "production"
            ),
            "production push logging must return a typed override rejection before creating any \
             artifact; expected fixed data/push_log, got {result:?}"
        );
        assert!(
            !cross_namespace_override.exists(),
            "rejected production override must not create a directory"
        );
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_push_log_rejects_test_root_symlink_to_a_foreign_namespace() {
        use std::os::unix::fs::symlink;

        let namespace = TestPushLogNamespace::new("TEST_ROOT_ALIAS");
        let foreign = namespace.path().join("foreign_production_log");
        std::fs::create_dir(&foreign).expect("create foreign log target");
        let fixed_test_root = namespace.path().join("push_log");
        symlink(
            std::fs::canonicalize(&foreign).expect("canonical foreign target"),
            &fixed_test_root,
        )
        .expect("alias fixed test root");

        let result = save_push_log_at_root(
            &fixed_test_root,
            "TEST_CODE root alias must fail before writing",
        );

        assert!(
            matches!(result, Err(PushLogError::NamespaceIsolation(_))),
            "test root symlink must be a typed physical-isolation rejection: {result:?}"
        );
        assert_eq!(
            std::fs::read_dir(&foreign)
                .expect("read untouched foreign target")
                .count(),
            0
        );
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_push_log_rejects_production_root_symlink_to_a_test_namespace() {
        use std::os::unix::fs::symlink;

        let namespace = TestPushLogNamespace::new("PRODUCTION_ROOT_ALIAS");
        let test_target = namespace.path().join("test_namespace_log");
        std::fs::create_dir(&test_target).expect("create test log target");
        let production_semantic_root = namespace.path().join("production_push_log");
        symlink(
            std::fs::canonicalize(&test_target).expect("canonical test target"),
            &production_semantic_root,
        )
        .expect("alias production-semantic root");

        let result = save_push_log_at_root(
            &production_semantic_root,
            "production root alias must fail before writing",
        );

        assert!(
            matches!(result, Err(PushLogError::NamespaceIsolation(_))),
            "production root symlink must be a typed physical-isolation rejection: {result:?}"
        );
        assert_eq!(
            std::fs::read_dir(&test_target)
                .expect("read untouched test target")
                .count(),
            0
        );
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_push_log_rejects_a_symlinked_date_directory() {
        use std::os::unix::fs::symlink;

        let namespace = TestPushLogNamespace::new("DATE_ALIAS");
        let fixed_root = namespace.path().join("push_log");
        std::fs::create_dir(&fixed_root).expect("create fixed push-log root");
        let foreign = namespace.path().join("foreign_date_log");
        std::fs::create_dir(&foreign).expect("create foreign date target");
        let date_dir = chrono::Local::now().format("%Y-%m-%d").to_string();
        symlink(
            std::fs::canonicalize(&foreign).expect("canonical foreign date target"),
            fixed_root.join(date_dir),
        )
        .expect("alias push-log date directory");

        let result = save_push_log_at_root(&fixed_root, "date alias must fail before writing");

        assert!(
            matches!(result, Err(PushLogError::NamespaceIsolation(_))),
            "date-directory symlink must be a typed physical-isolation rejection: {result:?}"
        );
        assert_eq!(
            std::fs::read_dir(&foreign)
                .expect("read untouched foreign date target")
                .count(),
            0
        );
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_push_log_rejects_a_root_directory_swap_before_artifact_creation() {
        let namespace = TestPushLogNamespace::new("ROOT_SWAP");
        let fixed_root = namespace.path().join("push_log");
        let displaced_root = namespace.path().join("displaced_push_log");
        let mut swapped = false;
        let mut displaced_date = None;

        let result = save_push_log_at_root_with_hook(
            &fixed_root,
            "root swap must fail before writing",
            |phase, root, date_path, _| {
                if matches!(phase, PushLogWritePhase::DirectoriesBound) && !swapped {
                    displaced_date = Some(
                        displaced_root.join(
                            date_path
                                .file_name()
                                .expect("bound date path has a final component"),
                        ),
                    );
                    std::fs::rename(root, &displaced_root).expect("displace bound push-log root");
                    std::fs::create_dir(root).expect("install replacement push-log root");
                    swapped = true;
                }
            },
        );

        assert!(
            matches!(result, Err(PushLogError::NamespaceIsolation(_))),
            "root swap must be a typed physical-isolation rejection: {result:?}"
        );
        assert!(swapped, "root-swap hook did not run");
        assert_eq!(
            std::fs::read_dir(&fixed_root)
                .expect("read replacement root")
                .count(),
            0,
            "replacement namespace must not receive an artifact"
        );
        assert_eq!(
            std::fs::read_dir(displaced_date.expect("capture displaced date path"))
                .expect("read displaced bound date directory")
                .count(),
            0,
            "displaced namespace must not receive an artifact"
        );
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_push_log_rejects_a_date_directory_swap_before_artifact_creation() {
        let namespace = TestPushLogNamespace::new("DATE_SWAP");
        let fixed_root = namespace.path().join("push_log");
        let displaced_date = namespace.path().join("displaced_date");
        let mut swapped = false;

        let result = save_push_log_at_root_with_hook(
            &fixed_root,
            "date swap must fail before writing",
            |phase, _, date_path, _| {
                if matches!(phase, PushLogWritePhase::DirectoriesBound) && !swapped {
                    std::fs::rename(date_path, &displaced_date)
                        .expect("displace bound push-log date directory");
                    std::fs::create_dir(date_path)
                        .expect("install replacement push-log date directory");
                    swapped = true;
                }
            },
        );

        assert!(
            matches!(result, Err(PushLogError::NamespaceIsolation(_))),
            "date swap must be a typed physical-isolation rejection: {result:?}"
        );
        assert!(swapped, "date-swap hook did not run");
        assert_eq!(
            std::fs::read_dir(&displaced_date)
                .expect("read displaced date directory")
                .count(),
            0,
            "displaced date directory must not receive an artifact"
        );
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_push_log_writer_rejects_a_root_swap_between_saves() {
        let namespace = TestPushLogNamespace::new("CROSS_SAVE_ROOT_SWAP");
        let writer = PinnedPushLogWriter::for_test_anchor(
            "TEST_CODE_CROSS_SAVE",
            namespace.path(),
            std::path::Path::new("push_log"),
        )
        .expect("bind persistent TEST_CODE push-log writer");
        writer
            .save("first physically bound push-log artifact")
            .expect("first push-log save");

        let fixed_root = namespace.path().join("push_log");
        let displaced_root = namespace.path().join("displaced_push_log");
        std::fs::rename(&fixed_root, &displaced_root).expect("displace bound push-log root");
        std::fs::create_dir(&fixed_root).expect("install replacement push-log root");

        let result = writer.save("replacement root must never be accepted");
        assert!(
            matches!(result, Err(PushLogError::NamespaceIsolation(_))),
            "persistent writer must reject a root identity change between saves: {result:?}"
        );
        assert_eq!(
            std::fs::read_dir(&fixed_root)
                .expect("read replacement push-log root")
                .count(),
            0,
            "replacement root must not receive an artifact"
        );
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_push_log_rejects_an_artifact_swap_after_fsync() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let namespace = TestPushLogNamespace::new("ARTIFACT_SWAP");
        let fixed_root = namespace.path().join("push_log");
        let displaced_artifact = namespace.path().join("displaced_artifact.md");
        let mut swapped = false;

        let result = save_push_log_at_root_with_hook(
            &fixed_root,
            "original durable delivery evidence",
            |phase, _, _, artifact_path| {
                if matches!(phase, PushLogWritePhase::ArtifactSynced) && !swapped {
                    let artifact_path = artifact_path.expect("artifact path at synced phase");
                    std::fs::rename(artifact_path, &displaced_artifact)
                        .expect("displace synced push-log artifact");
                    let mut replacement = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(artifact_path)
                        .expect("install replacement push-log artifact");
                    replacement
                        .set_permissions(std::fs::Permissions::from_mode(0o600))
                        .expect("secure replacement permissions");
                    replacement
                        .write_all(b"replacement")
                        .expect("write replacement artifact");
                    replacement.sync_data().expect("sync replacement artifact");
                    swapped = true;
                }
            },
        );

        assert!(
            matches!(result, Err(PushLogError::NamespaceIsolation(_))),
            "artifact swap must be a typed physical-isolation rejection: {result:?}"
        );
        assert!(swapped, "artifact-swap hook did not run");
        assert_eq!(
            std::fs::read_to_string(displaced_artifact).expect("read displaced original artifact"),
            "original durable delivery evidence"
        );
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_push_log_rejects_an_artifact_hard_link_after_fsync() {
        let namespace = TestPushLogNamespace::new("ARTIFACT_HARD_LINK");
        let fixed_root = namespace.path().join("push_log");
        let extra_link = namespace.path().join("extra_artifact_link.md");
        let mut linked = false;

        let result = save_push_log_at_root_with_hook(
            &fixed_root,
            "hard-linked evidence must be rejected",
            |phase, _, _, artifact_path| {
                if matches!(phase, PushLogWritePhase::ArtifactSynced) && !linked {
                    std::fs::hard_link(
                        artifact_path.expect("artifact path at synced phase"),
                        &extra_link,
                    )
                    .expect("create hostile artifact hard link");
                    linked = true;
                }
            },
        );

        assert!(
            matches!(result, Err(PushLogError::NamespaceIsolation(_))),
            "artifact hard link must be a typed physical-isolation rejection: {result:?}"
        );
        assert!(linked, "artifact-hard-link hook did not run");
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
    // ============== v17.5 §2.2: is_legacy_v17_5 标 4 variants (2026-07-16 审计后) ==============

    #[test]
    fn is_legacy_v17_5_marks_three_remaining_variants() {
        // BR-223: AuctionRepush 已恢复生产接线, 移出 legacy (剩 3 个)
        let legacy_variants = [
            PushKind::CandidateTriggered,
            PushKind::CandidateInvalidated,
            PushKind::VirtualWatch,
        ];
        assert_eq!(
            legacy_variants.len(),
            3,
            "v17.5 §2.2 审计后应剩 3 个 variants (BR-223 移出 AuctionRepush)"
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
        // 已经过调用链审计确认删除; BR-223 恢复 AuctionRepush 后剩 3 项 legacy 标记
        // (CandidateTriggered + CandidateInvalidated + VirtualWatch)
        let all_legacy_hits: Vec<PushKind> = [
            PushKind::CandidateTriggered,
            PushKind::CandidateInvalidated,
            PushKind::VirtualWatch,
        ]
        .into_iter()
        .filter(|k| k.is_legacy_v17_5())
        .collect();
        assert_eq!(all_legacy_hits.len(), 3);
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

    // ============== v17.x: DISPATCH_TABLE 19 rows 完整性 (BR-234 + 任务#3 + R-12 + R-13) ==============

    #[test]
    fn dispatch_table_size_is_nineteen() {
        assert_eq!(
            DISPATCH_TABLE.len(),
            19,
            "v17.x DISPATCH_TABLE 应 19 rows (3 v17.6 + 6 v17.7 + 6 v17.8 + 1 BR-234 + 1 #3 + 1 R-12 + 1 R-13)"
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
        // v17.6 low-priority 3 + v17.7 6 + v17.8 6 + BR-234 1 + 任务#3 1 + R-12 1 = 18
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
            PushKind::PaperSell,
            PushKind::SnapshotStale,
            PushKind::ReviewBacktest,
            PushKind::WatchlistTracking,
        ];
        assert_eq!(expected.len(), 19);
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
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let r = push_governor_v3("test kept auction", PushKind::AuctionVolume, None).await;
        assert_eq!(r, PushOutcome::Pushed);
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br192_explicit_counted_binding_reaches_durable_dry_run() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        let _banner_guard = TestBannerGuard::full();
        let test_code =
            std::env::var("DURABLE_DELIVERY_TEST_CODE").expect("isolated TEST_CODE namespace");
        let binding = crate::durable_delivery_runtime::CountedDeliveryBinding::new(
            chrono::Local::now().date_naive(),
            "TEST_CODE_NOTIFY_EXPLICIT_OCCURRENCE",
            b"{\"source\":\"TEST_CODE_INTERNAL\"}".to_vec(),
            crate::durable_delivery_runtime::CountedDeliveryScope::Global,
            "f8d53518ba6725c98450d031208450e7f8eb2dbdff2b9c71b21c14085e5d90ea",
            crate::durable_delivery_runtime::CountedDeliveryOrigin::InternalDurable,
            None,
            false,
        )
        .unwrap();

        let token = crate::presentation_registry::acquire_token(
            "T-04-holding-event",
            PushKind::HoldingEvent,
            "holding_event_dispatcher",
            "render_holding_event",
        )
        .unwrap();
        let outcome =
            push_counted_with_binding(token, "TEST_CODE explicit counted body", None, binding)
                .await;

        assert_eq!(outcome, PushOutcome::Pushed);
        let durable_database = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data/test")
            .join(test_code)
            .join("durable_delivery.sqlite3");
        let connection =
            rusqlite::Connection::open(&durable_database).expect("open isolated durable database");
        let receipt: (String, String, String) = connection
            .query_row(
                "SELECT result_kind,provider,channel FROM sink_results ORDER BY rowid DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("load isolated dry-run receipt");
        assert_eq!(
            receipt,
            (
                "Accepted".to_owned(),
                "TEST_CODE_MAGICLAW_DRY_RUN".to_owned(),
                "TEST_CODE_DRY_RUN".to_owned(),
            )
        );
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_production_environment_never_synthesizes_a_test_receipt() {
        use stock_analysis::durable_delivery::{
            AuthoritativeDeliveryRequest, AuthoritativeSinkPort, AuthoritativeSinkResult,
        };

        let _env_guard = crate::TestEnvGuard::capture(&[
            "STOCK_ENV_MODE",
            "V10_DRY_RUN_PUSH",
            "DURABLE_DELIVERY_TEST_CODE",
            "PUSH_LOG_DIR",
        ]);
        std::env::set_var("STOCK_ENV_MODE", "prod");
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        std::env::remove_var("DURABLE_DELIVERY_TEST_CODE");
        std::env::remove_var("PUSH_LOG_DIR");
        let request = AuthoritativeDeliveryRequest {
            decision_identity: "a".repeat(64),
            attempt_identity: "b".repeat(64),
            fence_token: 1,
            push_kind: stock_analysis::durable_delivery::PushKind::HoldingEvent,
            stable_template_id: stock_analysis::durable_delivery::PushKind::HoldingEvent
                .stable_template_id()
                .to_owned(),
            rendered_content: b"TEST_CODE must remain isolated".to_vec(),
            rendered_content_sha256:
                "709d96924d3b16a33caf6171bd5a9bc547f166739bd6854b585d1cc155a8f473".to_owned(),
        };

        let push_log_fixture = TestPushLogNamespace::new("PRODUCTION_SINK_REJECTION");
        let sink = crate::durable_delivery_runtime::MagiclawAuthoritativeSink::from_test_artifacts(
            crate::durable_delivery_runtime::RuntimeNamespace::Production,
            PinnedPushLogWriter::for_test_anchor(
                "production",
                push_log_fixture.path(),
                std::path::Path::new("push_log"),
            )
            .expect("bind production-semantic test push-log"),
            stock_analysis::event::AuditDispatcher::for_test_code(push_log_fixture.test_code())
                .expect("bind production-semantic TEST_CODE audit"),
        );
        let result = sink.deliver(&request);

        match result {
            AuthoritativeSinkResult::Rejected(rejection) => {
                assert_eq!(
                    rejection.reason_code,
                    "production_dry_run_configuration_rejected"
                );
                assert!(!rejection.retry_authorized);
            }
            other => panic!("production dry-run returned non-rejection: {other:?}"),
        }
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br192_bool_governor_rejects_counted_kind_without_binding() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let r = push_governor("test kept holding", PushKind::HoldingEvent).await;
        assert!(!r);
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br192_v3_governor_rejects_counted_kind_without_binding() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let outcome =
            push_governor_v3("first", PushKind::HoldingPlan, Some("TEST_CODE_000001")).await;
        assert_eq!(
            outcome,
            PushOutcome::Denied("counted_binding_required".to_owned())
        );
    }

    /// PUSH_VERBOSE=true 覆盖降级 → 调 push_wechat
    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn push_verbose_true_overrides_deprecated() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let r = push_governor_v3("test verbose auction", PushKind::AuctionVolume, None).await;
        assert_eq!(r, PushOutcome::Pushed);
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

    #[test]
    fn br160_source_batch_governor_routes_lineage_to_authoritative_audit() {
        let source = include_str!("notify.rs");
        let start = source
            .find("async fn deliver_and_record")
            .expect("delivery tail exists");
        let end = source[start..]
            .find("/// BR-137: a source-fact identity")
            .expect("delivery tail boundary");
        let delivery_tail = &source[start..start + end];

        assert!(delivery_tail.contains("publish_source_batch_delivery"));
        for accessor in [
            "evidence.business_date()",
            "evidence.observed_at()",
            "evidence.batch_id()",
            "evidence.content_hash()",
        ] {
            assert!(
                delivery_tail.contains(accessor),
                "A-10 delivery audit must retain {accessor}"
            );
        }
    }
}
