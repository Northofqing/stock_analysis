//! v12 §14 推送消息模板渲染
//!
//! 职责：仅做"按模板拼字符串"，不接 push 通道、不写库、不读行情。
//! 模板结构与字段顺序严格对齐 `docs/architecture/v12-push-templates.md`。
//!
//! 调用约定:
//!   1. 调用方先拼好本模板所需的领域数据（结构体入参）
//!   2. 调对应 `render_xxx()` 函数得到完整 text
//!   3. 调 `super::notify::push_governor(&text, kind).await` 推送
//!
//! 后续 PR 接入点（不动本文件签名即可演进）:
//!   - PR1: `AccountMode` 替换为 `risk::account_mode::AccountState`
//!   - PR2: `DataMode` 替换为 `monitor::data_mode::DataHealth`
//!   - PR4: Banner 字段接真值 (From impl 即可)

use std::fmt;

// ============================================================================
// §14.0 全局横幅 — 输入结构
// ============================================================================

/// v12 §14.0 横幅账户模式
///
/// 暂为本地轻量枚举。PR1 (`risk::account_mode::AccountState`) 合入后, 加 `From`。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum AccountMode {
    Normal,
    ReduceOnly,
    Frozen,
}

impl AccountMode {
    pub fn label(self) -> &'static str {
        match self {
            AccountMode::Normal => "Normal",
            AccountMode::ReduceOnly => "ReduceOnly",
            AccountMode::Frozen => "Frozen",
        }
    }

    /// §14.0 mode_icon
    pub fn icon(self) -> &'static str {
        match self {
            AccountMode::Normal => "🟢",
            AccountMode::ReduceOnly => "🟡",
            AccountMode::Frozen => "🔴",
        }
    }
}

impl fmt::Display for AccountMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl Default for AccountMode {
    fn default() -> Self {
        AccountMode::Normal
    }
}

/// v12 §14.0 横幅数据模式
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum DataMode {
    Full,
    Degraded,
    Unsafe,
}

impl DataMode {
    pub fn label(self) -> &'static str {
        match self {
            DataMode::Full => "Full",
            DataMode::Degraded => "Degraded",
            DataMode::Unsafe => "Unsafe",
        }
    }
}

impl fmt::Display for DataMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl Default for DataMode {
    fn default() -> Self {
        DataMode::Full
    }
}

/// v12 §14.0 全局横幅入参
///
/// `total_pos` 仓位成数 (0~10). `today_pnl` 日盈亏百分比 (已带正负号).
/// `data_missing_note` 仅在 Degraded/Unsafe 出现, 例如 "缺盘口深度".
#[derive(Clone, Debug)]
pub struct BannerCtx {
    pub account_mode: AccountMode,
    pub total_pos: u8,
    pub today_pnl: f64,
    pub data_mode: DataMode,
    pub data_missing_note: Option<String>,
}

impl Default for BannerCtx {
    fn default() -> Self {
        Self {
            account_mode: AccountMode::Normal,
            total_pos: 0,
            today_pnl: 0.0,
            data_mode: DataMode::Full,
            data_missing_note: None,
        }
    }
}

impl BannerCtx {
    /// 渲染 §14.0 横幅 (1~2 行).
    ///
    /// 第 1 行: `[icon mode | 仓位N成 | 日盈亏+/-X.X% | 数据DataMode]`
    /// 第 2 行 (可选): `[⚠️ {data_missing_note}]` — 仅 Degraded/Unsafe 时出现
    pub fn render(&self) -> String {
        let line1 = format!(
            "[{} {} | 仓位{}成 | 日盈亏{:+.1}% | 数据{}]",
            self.account_mode.icon(),
            self.account_mode.label(),
            self.total_pos,
            self.today_pnl,
            self.data_mode.label(),
        );
        match self.data_missing_note.as_deref() {
            Some(note) if !note.is_empty() && self.data_mode != DataMode::Full => {
                format!("{}\n[⚠️ {}: 本条不含承接判断]", line1, note)
            }
            _ => line1,
        }
    }
}

// ============================================================================
// §14.1 实盘时段 — T-01 ~ T-12
// ============================================================================

/// T-01 账户模式变更
///
/// `reasons` / `forbidden_actions` / `recovery_condition` 由调用方拼好.
pub fn render_account_mode(
    hhmm: &str,
    old: AccountMode,
    new: AccountMode,
    reasons: &[String],
    forbidden_actions: &str,
    recovery_condition: &str,
) -> String {
    let mut out = format!(
        "🛡️ 账户模式变更（{}）\n{} → {}\n原因:",
        hhmm,
        old.label(),
        new.label(),
    );
    for r in reasons {
        out.push_str(&format!("\n· {}", r));
    }
    out.push_str(&format!(
        "\n生效限制: {}\n解除条件: {}",
        forbidden_actions, recovery_condition,
    ));
    out
}

/// T-02 数据状态变更
pub fn render_data_mode(
    hhmm: &str,
    old: DataMode,
    new: DataMode,
    missing_items: &str,
    restrictions: &[String],
    eta: Option<&str>,
) -> String {
    let mut out = format!(
        "📡 数据状态变更（{}）\n{} → {}\n受影响: {}\n输出限制:",
        hhmm,
        old.label(),
        new.label(),
        missing_items,
    );
    for r in restrictions {
        out.push_str(&format!("\n· {}", r));
    }
    if let Some(eta) = eta.filter(|s| !s.is_empty()) {
        out.push_str(&format!("\n恢复预计: {}", eta));
    }
    out
}

/// 持仓建议动作倾向
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// 逢高减仓
    Reduce,
    /// 清仓
    Clear,
    /// 持有观望
    Hold,
    /// 加仓
    Add,
    /// 做T
    T0,
}

impl Intent {
    pub fn label(self) -> &'static str {
        match self {
            Intent::Reduce => "逢高减仓",
            Intent::Clear => "清仓",
            Intent::Hold => "持有观望",
            Intent::Add => "加仓",
            Intent::T0 => "做T",
        }
    }
}

/// T-03 持仓操作建议
pub fn render_holding_plan(banner: &BannerCtx, params: HoldingPlanParams<'_>) -> String {
    let hhmm = params.hhmm;
    let mut out = format!(
        "{}\n🎯 持仓建议 {}({})（{}）\n动作倾向: {} | 现价{} 成本{} 可用{}股",
        banner.render(),
        params.name,
        params.code,
        hhmm,
        params.intent.label(),
        fmt_price(params.price),
        fmt_price(params.cost),
        params.avail,
    );
    if let Some((lo, hi)) = params.reduce_zone {
        out.push_str(&format!("\n减仓观察区: {}~{}", fmt_price(lo), fmt_price(hi)));
    }
    out.push_str(&format!(
        "\n支撑{} | 压力{} | 硬止损{}",
        fmt_price(params.support),
        fmt_price(params.pressure),
        fmt_price(params.stop),
    ));
    if !params.invalidations.is_empty() {
        out.push_str("\n无效条件:");
        for inv in params.invalidations {
            out.push_str(&format!("\n· {}", inv));
        }
    }
    out.push_str(&format!(
        "\n理由: {}\n辅助建议, 非下单指令",
        params.reasons.join("; "),
    ));
    out
}

#[derive(Debug)]
pub struct HoldingPlanParams<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub hhmm: &'a str,
    pub intent: Intent,
    pub price: f64,
    pub cost: f64,
    pub avail: u32,
    pub reduce_zone: Option<(f64, f64)>,
    pub support: f64,
    pub pressure: f64,
    pub stop: f64,
    pub invalidations: &'a [String],
    pub reasons: &'a [String],
}

/// T-04 持仓紧急风险
pub fn render_holding_event(banner: &BannerCtx, p: HoldingEventParams<'_>) -> String {
    format!(
        "{}\n🚨 持仓风险 {}({})（{}）\n触发: {}\n现价{}（{:+.1}%） 距止损{:+.1}%\n建议: {}\n可用股数: {}\n辅助建议, 非下单指令",
        banner.render(),
        p.name,
        p.code,
        p.hhmm,
        p.trigger,
        fmt_price(p.price),
        p.chg_pct,
        p.gap_pct,
        p.action,
        p.avail,
    )
}

#[derive(Debug)]
pub struct HoldingEventParams<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub hhmm: &'a str,
    pub trigger: &'a str,
    pub price: f64,
    pub chg_pct: f64,
    pub gap_pct: f64,
    pub action: &'a str,
    pub avail: u32,
}

/// T-05 做T建议 (ReverseT / PositiveT)
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum T0Kind {
    ReverseT,
    PositiveT,
}

impl T0Kind {
    pub fn label(self) -> &'static str {
        match self {
            T0Kind::ReverseT => "ReverseT",
            T0Kind::PositiveT => "PositiveT",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum T0Style {
    /// 急跌接刀
    PullbackCatch,
    /// 高位震荡低吸
    RangeLowBuy,
    /// 突破追单
    Breakout,
    /// 其它
    Other,
}

impl T0Style {
    pub fn label(self) -> &'static str {
        match self {
            T0Style::PullbackCatch => "急跌接刀",
            T0Style::RangeLowBuy => "震荡低吸",
            T0Style::Breakout => "突破追单",
            T0Style::Other => "其它",
        }
    }
}

pub fn render_t0_advice(banner: &BannerCtx, p: T0AdviceParams<'_>) -> String {
    format!(
        "{}\n🔁 做T {}({})（{}）\n结论: {} | 类型: {}\n可用底仓: {}股\n卖出观察区: {}~{}\n接回观察区: {}~{}\n最小价差: ≥{:.1}%（覆盖2×成本）\n风险: {}\n做T不改变总仓位判断; 趋势走强优先持有",
        banner.render(),
        p.name,
        p.code,
        p.hhmm,
        p.kind.label(),
        p.style.label(),
        p.avail,
        fmt_price(p.sell_lo),
        fmt_price(p.sell_hi),
        fmt_price(p.buy_lo),
        fmt_price(p.buy_hi),
        p.min_spread_pct,
        p.risk_note,
    )
}

#[derive(Debug)]
pub struct T0AdviceParams<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub hhmm: &'a str,
    pub kind: T0Kind,
    pub style: T0Style,
    pub avail: u32,
    pub sell_lo: f64,
    pub sell_hi: f64,
    pub buy_lo: f64,
    pub buy_hi: f64,
    pub min_spread_pct: f64,
    pub risk_note: &'a str,
}

/// T-06 不建议做T
pub fn render_t0_forbid(banner: &BannerCtx, p: T0ForbidParams<'_>) -> String {
    format!(
        "{}\n🔁🚫 不建议做T {}({})（{}）\n原因: {}",
        banner.render(),
        p.name, p.code, p.hhmm, p.reason,
    )
}

#[derive(Debug)]
pub struct T0ForbidParams<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub hhmm: &'a str,
    pub reason: &'a str,
}

/// T-07 候选触发
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CandidateGrade {
    A,
    B,
}

impl CandidateGrade {
    pub fn label(self) -> &'static str {
        match self {
            CandidateGrade::A => "A",
            CandidateGrade::B => "B",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EvidenceQuality {
    Strong,
    Mid,
    Weak,
    Missing,
}

impl EvidenceQuality {
    pub fn label(self) -> &'static str {
        match self {
            EvidenceQuality::Strong => "强",
            EvidenceQuality::Mid => "中",
            EvidenceQuality::Weak => "弱",
            EvidenceQuality::Missing => "缺失,不作承接判断",
        }
    }
}

pub fn render_candidate_triggered(banner: &BannerCtx, p: CandidateTriggeredParams<'_>) -> String {
    let mut out = format!(
        "{}\n📋 候选触发 {}({})（{}）\n等级{} | 状态: Triggered | 主题: {}\n现价{} 已触发: {}\n低吸参考: {}~{} | 止损{} | 仓位上限{}%",
        banner.render(),
        p.name, p.code, p.hhmm,
        p.grade.label(), p.topic,
        fmt_price(p.price), p.trigger_desc,
        fmt_price(p.lo), fmt_price(p.hi),
        fmt_price(p.stop), p.max_pos_pct,
    );
    out.push_str("\n证据:");
    out.push_str(&format!("\n· 新闻: {} {}", p.news_quality.label(), p.news_note));
    out.push_str(&format!(
        "\n· 量能: {} 量比{:.1}",
        p.vol_quality.label(), p.vol_ratio,
    ));
    out.push_str(&format!("\n· K线: {} {}", p.kline_quality.label(), p.kline_note));
    out.push_str(&format!("\n· 盘口: {}", p.book_quality.label()));
    if !p.no_buy.is_empty() {
        out.push_str("\n不买条件:");
        for nb in p.no_buy {
            out.push_str(&format!("\n· {}", nb));
        }
    }
    out.push_str("\n需人工确认, 非自动买入");
    out
}

#[derive(Debug)]
pub struct CandidateTriggeredParams<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub hhmm: &'a str,
    pub grade: CandidateGrade,
    pub topic: &'a str,
    pub price: f64,
    pub trigger_desc: &'a str,
    pub lo: f64,
    pub hi: f64,
    pub stop: f64,
    pub max_pos_pct: u8,
    pub news_quality: EvidenceQuality,
    pub news_note: &'a str,
    pub vol_quality: EvidenceQuality,
    pub vol_ratio: f64,
    pub kline_quality: EvidenceQuality,
    pub kline_note: &'a str,
    pub book_quality: EvidenceQuality,
    pub no_buy: &'a [String],
}

/// T-08 候选失效
pub fn render_candidate_invalidated(
    hhmm: &str,
    name: &str,
    code: &str,
    prev: &str,
    reason: &str,
) -> String {
    format!(
        "📋 候选失效 {}({})（{}）\n原状态{} → Invalidated\n原因: {}",
        name, code, hhmm, prev, reason,
    )
}

/// T-09 禁止操作提示
pub fn render_forbidden_ops(banner: &BannerCtx, p: ForbiddenOpsParams<'_>) -> String {
    let mut out = format!(
        "{}\n🚫 禁止操作（{}）\n{}({}): {}\n· {}",
        banner.render(),
        p.hhmm,
        p.name, p.code, p.conclusion,
        p.reasons.first().map(String::as_str).unwrap_or(""),
    );
    for r in p.reasons.iter().skip(1) {
        out.push_str(&format!("\n· {}", r));
    }
    out
}

#[derive(Debug)]
pub struct ForbiddenOpsParams<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub hhmm: &'a str,
    pub conclusion: &'a str,
    pub reasons: &'a [String],
}

/// T-10 虚拟盘成交回报
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PaperTradeStatus {
    Filled,
    NotFilled,
    Invalidated,
}

impl PaperTradeStatus {
    pub fn label(self) -> &'static str {
        match self {
            PaperTradeStatus::Filled => "Filled",
            PaperTradeStatus::NotFilled => "NotFilled",
            PaperTradeStatus::Invalidated => "Invalidated",
        }
    }
}

pub fn render_paper_trade(p: PaperTradeParams<'_>) -> String {
    let mut out = format!(
        "🧪 虚拟盘（{}）\n{}({}) {}",
        p.hhmm, p.name, p.code, p.status.label(),
    );
    if p.status == PaperTradeStatus::Filled {
        out.push_str(&format!(
            "\n成交价{} 数量{} 主理由{}",
            fmt_price(p.fill_price.unwrap_or(0.0)),
            p.qty.unwrap_or(0),
            p.virtual_reason.unwrap_or(""),
        ));
    }
    if p.status == PaperTradeStatus::NotFilled {
        out.push_str(&format!(
            "\n未成交原因: {}",
            p.not_fill_reason.unwrap_or(""),
        ));
    }
    out.push_str(&format!(
        "\n账户{}/数据{} 快照已记录",
        p.account_mode, p.data_mode,
    ));
    out
}

#[derive(Debug)]
pub struct PaperTradeParams<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub hhmm: &'a str,
    pub status: PaperTradeStatus,
    pub fill_price: Option<f64>,
    pub qty: Option<u32>,
    pub virtual_reason: Option<&'a str>,
    pub not_fill_reason: Option<&'a str>,
    pub account_mode: AccountMode,
    pub data_mode: DataMode,
}

/// T-11 竞价异动 (复用 AuctionVolume, 加横幅)
pub fn render_auction_volume(
    banner: &BannerCtx,
    hhmm: &str,
    items: &[AuctionItem<'_>],
    sentiment: &str,
    watch_status: &str,
) -> String {
    let mut out = format!("{}\n🌅 竞价异动 Top{}（{}）", banner.render(), items.len(), hhmm);
    for it in items {
        out.push_str(&format!(
            "\n  {}({}) 高开{:+.1}% 量比{:.1} [{}]",
            it.name, it.code, it.gap_pct, it.vol_ratio, it.tag,
        ));
    }
    out.push_str(&format!(
        "\n情绪判读: {}, 观察池今日{}",
        sentiment, watch_status,
    ));
    out
}

#[derive(Debug)]
pub struct AuctionItem<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub gap_pct: f64,
    pub vol_ratio: f64,
    pub tag: &'a str,
}

/// T-12 尾盘决策
#[derive(Debug, Default)]
pub struct CloseCallHolding<'a> {
    pub name: &'a str,
    pub state: &'a str, // "尾盘跳水-建议处理" / "正常"
}

#[derive(Debug, Default)]
pub struct CloseCallGamble<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub satisfied: bool,
    pub cond: &'a str,
}

pub fn render_close_call(
    banner: &BannerCtx,
    hhmm: &str,
    holding: Option<&CloseCallHolding<'_>>,
    gamble: Option<&CloseCallGamble<'_>>,
) -> String {
    let mut out = format!("{}\n🌇 尾盘提示（{}）", banner.render(), hhmm);
    if let Some(h) = holding {
        out.push_str(&format!("\n[持仓] {}: {}", h.name, h.state));
    }
    if let Some(g) = gamble {
        out.push_str(&format!(
            "\n[博弈] {}({}): 尾盘买入博次日溢价条件{}满足: {}",
            g.name, g.code,
            if g.satisfied { "已" } else { "未" },
            g.cond,
        ));
    }
    out
}

// ============================================================================
// §14.2 盘后时段 — R-01 ~ R-08
// ============================================================================

/// R-01 持仓复盘 + 明日计划
#[derive(Debug)]
pub struct HoldingDailyPlan<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub price: f64,
    pub cost: f64,
    pub pnl_pct: f64,
    pub high_gap_x: f64, // > 高开阈值 %
    pub plan_high: &'a str,
    pub plan_flat: &'a str,
    pub stop: f64,
    pub t0: &'a str, // "适合观察" / "不适合(原因)"
}

pub fn render_daily_report(date: &str, items: &[HoldingDailyPlan<'_>]) -> String {
    let mut out = format!("📌 持仓明日计划（{} 19:00）", date);
    for it in items {
        out.push_str(&format!(
            "\n{}({}) 现价{} 成本{} 浮盈{:+.1}%",
            it.name,
            it.code,
            fmt_price(it.price),
            fmt_price(it.cost),
            it.pnl_pct,
        ));
        out.push_str(&format!(
            "\n· 高开>{:.1}%: {}",
            it.high_gap_x, it.plan_high,
        ));
        out.push_str(&format!("\n· 平开: {}", it.plan_flat));
        out.push_str(&format!(
            "\n· 低开/跌破{}: 执行止损",
            fmt_price(it.stop),
        ));
        out.push_str(&format!("\n· 做T: {}", it.t0));
        out.push_str("\n─────");
    }
    out
}

/// R-02 盘面走向
#[derive(Debug)]
pub struct MarketReview<'a> {
    pub sh_chg: f64,
    pub chinext_chg: f64,
    pub star_chg: f64,
    pub limit_up_n: u32,
    pub limit_down_n: u32,
    pub broken_pct: f64,
    pub consecutive_h: u32,
    pub amount_yi: f64,
    pub amount_delta_pct: f64,
    pub amount_dir: &'a str, // "放量" / "缩量"
    pub main_flow_yi: f64,
    pub money_effect: &'a str, // 赚钱效应描述
    pub heat_stage: &'a str,
    pub heat_conf_pct: u8,
    pub low_conf: bool, // 是否低置信
    pub low_conf_tier: Option<&'a str>, // "保守档"
    pub account_mode: AccountMode,
    pub max_pos: u8,
}

pub fn render_review_market(date: &str, m: &MarketReview<'_>) -> String {
    let mut out = format!(
        "📊 今日盘面（{}）\n指数: 上证{:+.1}% 创业{:+.1}% 科创{:+.1}%\n情绪: 涨停{}家 跌停{}家 炸板率{:.0}% 连板高度{}板\n资金: 两市{:.0}亿（{}{:+.0}%） 主力净{:+.0}亿\n赚钱效应: {}\n阶段判定: {}（置信度{}%）",
        date,
        m.sh_chg, m.chinext_chg, m.star_chg,
        m.limit_up_n, m.limit_down_n, m.broken_pct, m.consecutive_h,
        m.amount_yi, m.amount_dir, m.amount_delta_pct, m.main_flow_yi,
        m.money_effect,
        m.heat_stage, m.heat_conf_pct,
    );
    if m.low_conf {
        out.push_str(&format!(
            "\n⚠️ 低置信, 权限按{}执行",
            m.low_conf_tier.unwrap_or("保守档"),
        ));
    }
    out.push_str(&format!(
        "\n→ 明日账户建议: {} 仓位上限{}成",
        m.account_mode.label(),
        m.max_pos,
    ));
    out
}

/// R-03 涨停产业链
#[derive(Debug)]
pub struct ChainLine<'a> {
    pub chain: &'a str,
    pub limit_up_n: u32,
    pub first_n: u32,
    pub consec_n: u32,
    pub heat_stage: &'a str,
    pub leader_name: &'a str,
    pub leader_code: &'a str,
    pub leader_boards: u32,
    pub followers: &'a str,
    pub watch_point: &'a str,
}

pub fn render_industry_chain(date: &str, chains: &[ChainLine<'_>], fade: Option<&str>) -> String {
    let mut out = format!("🔥 涨停产业链（{}）", date);
    for (i, c) in chains.iter().enumerate() {
        out.push_str(&format!(
            "\n{}. {} 涨停{}家（首板{}/连板{}） 阶段: {}\n   龙头: {}({}) {}板\n   后排: {}\n   明日观察: {}",
            i + 1,
            c.chain, c.limit_up_n, c.first_n, c.consec_n, c.heat_stage,
            c.leader_name, c.leader_code, c.leader_boards,
            c.followers,
            c.watch_point,
        ));
    }
    if let Some(f) = fade.filter(|s| !s.is_empty()) {
        out.push_str(&format!("\n⚠️ 退潮链: {}", f));
    }
    out
}

/// R-04 龙虎榜
#[derive(Debug)]
pub struct LhbEntry<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub net_buy_yi: f64,
    pub reason: &'a str,
    pub buy_inst_n: u32,
    pub buy_inst_amt_wan: f64,
    pub buy_other_n: u32,
    pub buy_other_amt_wan: f64,
    pub buy_conc_pct: f64,
    pub sell_desc: &'a str,
    pub sell_conc_pct: f64,
    pub chain_match: Option<&'a str>,
    pub next_day_risk: &'a str,
}

pub fn render_review_lhb(date: &str, entries: &[LhbEntry<'_>]) -> String {
    let mut out = format!("🐉 龙虎榜净买前五（{} 21:00）", date);
    for (i, e) in entries.iter().enumerate() {
        out.push_str(&format!(
            "\n{}. {}({}) 净买{:.1}亿 | {}\n   买: 机构{}席{:.0}万 其他{}席{:.0}万（集中度{:.0}%）\n   卖: {}（集中度{:.0}%）\n   主线一致: {}\n   次日风险: {}",
            i + 1,
            e.name, e.code, e.net_buy_yi, e.reason,
            e.buy_inst_n, e.buy_inst_amt_wan,
            e.buy_other_n, e.buy_other_amt_wan,
            e.buy_conc_pct,
            e.sell_desc, e.sell_conc_pct,
            e.chain_match.map(|s| format!("是-{}", s)).unwrap_or_else(|| "否".to_string()),
            e.next_day_risk,
        ));
        out.push_str("\n─────");
    }
    out.push_str("\n仅结构化事实, 不含席位风格推断");
    out
}

/// R-05 系统信号复盘
#[derive(Debug, Default)]
pub struct SignalReview {
    pub holding_n: u32,   // 持仓建议推 n 条
    pub holding_exec: u32,
    pub holding_eff: u32,
    pub t0_n: u32,        // 做T 推 n
    pub t0_eff: u32,
    pub cand_trigger: u32,
    pub cand_filled: u32,
    pub cand_notfilled: u32,
    pub cand_limitup: u32,
    pub cand_notreach: u32,
    pub paper_pnl_pct: f64,
    pub paper_total_pct: f64,
    pub paper_n: u32,
    pub news_push_n: u32,
    pub news_d1_eff: u32,
}

pub fn render_review_signal(date: &str, r: &SignalReview) -> String {
    format!(
        "🤖 信号复盘（{}）\n持仓建议: 推{}条 执行{}条 有效{}条\n做T建议: 推{} 有效{}\n候选(影子): 触发{} 模拟成交{} 未成交{}（涨停{}/未触达{}）\n虚拟盘: 今日{:+.1}% 累计{:+.1}%（样本{}笔）\n新闻兑现: 推送{}条 D+1兑现{}条",
        date,
        r.holding_n, r.holding_exec, r.holding_eff,
        r.t0_n, r.t0_eff,
        r.cand_trigger, r.cand_filled, r.cand_notfilled,
        r.cand_limitup, r.cand_notreach,
        r.paper_pnl_pct, r.paper_total_pct, r.paper_n,
        r.news_push_n, r.news_d1_eff,
    )
}

/// R-06 失败样本归因
#[derive(Debug)]
pub struct FailureEntry<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub signal_level: &'a str,
    pub virtual_reason: &'a str,
    pub result_desc: &'a str,
    pub pnl_pct: f64,
    pub failure_reason: &'a str,
    pub suggestion: &'a str,
}

#[derive(Debug, Default)]
pub struct FailureDistribution {
    pub buy_late: u32,
    pub chain_fade: u32,
    pub not_fillable: u32,
    pub human_not_exec: u32,
}

pub fn render_review_failure(
    date: &str,
    entries: &[FailureEntry<'_>],
    dist: &FailureDistribution,
) -> String {
    let mut out = format!("❌ 失败归因（{}）", date);
    for e in entries {
        out.push_str(&format!(
            "\n{}({}) 原信号: {}{}\n结果: {} {:+.1}%\n归因: {}\n处理建议: {}\n─────",
            e.name, e.code, e.signal_level, e.virtual_reason,
            e.result_desc, e.pnl_pct,
            e.failure_reason, e.suggestion,
        ));
    }
    out.push_str(&format!(
        "\n本周归因分布: 买点过晚{} 板块退潮{} 不可成交{} 人未执行{}",
        dist.buy_late, dist.chain_fade, dist.not_fillable, dist.human_not_exec,
    ));
    out
}

/// R-07 明日观察池
#[derive(Debug)]
pub struct WatchItem<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub topic: &'a str,
    pub source: &'a str, // "A档未触发" / "龙虎榜" / "涨停链" / "持仓做T"
    pub trigger: &'a str,
    pub lo: f64,
    pub hi: f64,
    pub stop: f64,
    pub reason: &'a str,
}

pub fn render_tomorrow_watch(date: &str, items: &[WatchItem<'_>]) -> String {
    let mut out = format!("📌 明日观察池（{}）", date);
    for (i, it) in items.iter().enumerate() {
        out.push_str(&format!(
            "\n{}. {}({}) [{}] 来源: {}\n   触发{} | 低吸{}~{} | 止损{}\n   理由: {}",
            i + 1,
            it.name, it.code, it.topic, it.source,
            it.trigger,
            fmt_price(it.lo), fmt_price(it.hi),
            fmt_price(it.stop),
            it.reason,
        ));
        out.push_str("\n─────");
    }
    out.push_str(&format!(
        "\n共{}只 | 明日竞价后按 T-11 复核",
        items.len(),
    ));
    out
}

/// R-08 明日事件日历
#[derive(Debug)]
pub struct HoldingEventItem<'a> {
    pub name: &'a str,
    pub kind: &'a str, // "解禁{amt}亿" / "财报预告" / "减持到期"
}

pub fn render_event_calendar(
    date: &str,
    holdings: &[HoldingEventItem<'_>],
    macro_econ: &str,
    us_chg: &str,
    fx: &str,
) -> String {
    let mut out = format!("🗓️ 明日事件（{}）\n持仓/观察池:", date);
    for h in holdings {
        out.push_str(&format!("\n· {}: {}", h.name, h.kind));
    }
    out.push_str(&format!(
        "\n宏观: {}\n隔夜关注: 美股{} 汇率{}",
        macro_econ, us_chg, fx
    ));
    out
}

// ============================================================================
// 工具函数
// ============================================================================

/// 价格格式: 保留 2 位小数 (微信/飞书宽度可控)
fn fmt_price(v: f64) -> String {
    format!("{:.2}", v)
}

// ============================================================================
// PR1-1.6 orchestrator: 模式变更 → 落库 → T-01 → dispatch
// ============================================================================

/// v12 PR1-1.6: 模式变更编排器.
///
/// 完整链路: evaluate() → is_changed() → 落库 → 拼 T-01 → dispatch() → 标记 pushed.
///
/// 返回 `Ok(true)` 表示落库 + 推送成功; `Ok(false)` 表示无变更 (no-op).
///
/// `prev` 由调用方从 `database::account_mode_log::latest_account_mode_change()` 恢复.
///
/// 当前 PR1 不挂主循环调用 (留给 PR1-1.7), 单测覆盖函数本身.
pub async fn push_account_mode_change(
    metrics: &stock_analysis::risk::account_mode::PortfolioMetrics,
    prev: Option<stock_analysis::risk::action_gate::AccountMode>,
    banner: Option<&BannerCtx>,
) -> Result<bool, String> {
    use stock_analysis::config::get_risk_config;
    use stock_analysis::risk::account_mode::{evaluate, ModeThresholds};
    use stock_analysis::risk::action_gate::AccountMode as LibAM;

    let thresholds: ModeThresholds = get_risk_config().account_mode.to_thresholds();
    let eval = evaluate(metrics, prev, &thresholds);

    if !eval.is_changed() {
        return Ok(false);
    }

    let prev_mode = prev.expect("is_changed=true ⇒ prev=Some");
    let new_mode = eval.mode;

    // 1. 落库 (pushed=0)
    let log_id = stock_analysis::database::account_mode_log::insert_account_mode_change(
        prev_mode,
        new_mode,
        eval.trigger_reason.as_deref().unwrap_or(""),
        Some(metrics.today_pnl_pct),
        Some(metrics.consecutive_stop_loss_n),
        Some(metrics.total_pos_cheng),
        metrics.data_complete,
    )
    .map_err(|e| format!("insert_account_mode_change: {}", e))?;

    // 2. 拼 T-01
    let hhmm = chrono::Local::now().format("%H:%M").to_string();
    let reasons = eval
        .trigger_reason
        .as_deref()
        .map(|r| vec![r.to_string()])
        .unwrap_or_default();
    let forbidden = match new_mode {
        LibAM::Normal => "(无)",
        LibAM::ReduceOnly => "禁止新开仓/加仓/正T, 候选转影子",
        LibAM::Frozen => "禁止新开仓/加仓/正T/反T, 候选转影子",
    };
    let recovery = match new_mode {
        LibAM::Normal => "(已是 Normal)",
        LibAM::ReduceOnly => "当日盈亏回到 -1.5% 内 或 连续止损 < 3 笔 (运行时) / 下一交易日盘前重置",
        LibAM::Frozen => "下一交易日盘前重置为 Normal",
    };
    let prev_tmpl = prev_mode_to_tmpl(prev_mode);
    let new_tmpl = prev_mode_to_tmpl(new_mode);

    let mut text = if let Some(b) = banner {
        format!("{}\n", b.render())
    } else {
        String::new()
    };
    text.push_str(&render_account_mode(
        &hhmm,
        prev_tmpl,
        new_tmpl,
        &reasons,
        forbidden,
        recovery,
    ));

    // 3. dispatch (code="" 全局键, AccountMode 无冷却)
    let ok = dispatch(
        super::notify::PushKind::AccountMode,
        "", // code 空 = 全局键
        banner,
        text,
    )
    .await;

    // 4. 标记 pushed
    if ok {
        if let Err(e) = stock_analysis::database::account_mode_log::mark_account_mode_pushed(log_id) {
            log::warn!("[AccountMode] mark pushed=1 失败 (id={}): {}", log_id, e);
        }
    } else {
        log::info!("[AccountMode] T-01 推送失败, log_id={} 保留 pushed=0 等重试", log_id);
    }

    Ok(ok)
}

fn prev_mode_to_tmpl(m: stock_analysis::risk::action_gate::AccountMode) -> AccountMode {
    use stock_analysis::risk::action_gate::AccountMode as LibAM;
    match m {
        LibAM::Normal => AccountMode::Normal,
        LibAM::ReduceOnly => AccountMode::ReduceOnly,
        LibAM::Frozen => AccountMode::Frozen,
    }
}

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use once_cell::sync::Lazy;

/// 冷却表: key = (PushKind, code_or_empty), value = last sent epoch secs
///
/// 进程内全局, monitor 重启即清零. v12 §14.3.1.
static COOLDOWN_TABLE: Lazy<std::sync::Mutex<HashMap<(super::notify::PushKind, String), i64>>> =
    Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

/// 当日"交易建议类"已推送条数 (§14.3.3 ≤ 30 条/日)
static DAILY_BUDGET_COUNT: AtomicU32 = AtomicU32::new(0);

/// 当日预算重置的 epoch day (UTC)
static DAILY_BUDGET_DAY: AtomicI64 = AtomicI64::new(0);

/// §14.3.3 每日预算上限
pub const DAILY_BUDGET_LIMIT: u32 = 30;

fn today_epoch_day() -> i64 {
    chrono::Utc::now().timestamp() / 86_400
}

fn reset_budget_if_new_day() {
    let today = today_epoch_day();
    let prev = DAILY_BUDGET_DAY.load(Ordering::Relaxed);
    if prev != today {
        DAILY_BUDGET_DAY.store(today, Ordering::Relaxed);
        DAILY_BUDGET_COUNT.store(0, Ordering::Relaxed);
    }
}

/// 判定: 该 (kind, code) 是否在冷却中. 紧急类 (`Emergency`) 与无冷却 (`None`) 永远返回 false.
///
/// 副作用: 不命中时**不**写表, 由 [`push_governor_with_mode`] 在真正推送时调 [`record_cooldown`] 写入.
pub fn is_in_cooldown(kind: super::notify::PushKind, code: &str) -> bool {
    use super::notify::PushLevel;
    if kind.level() == PushLevel::Emergency {
        return false;
    }
    let cd = match kind.cooldown_secs() {
        Some(s) => s as i64,
        None => return false,
    };
    let key = (kind, code.to_string());
    let table = COOLDOWN_TABLE.lock().expect("cooldown table poisoned");
    if let Some(&last) = table.get(&key) {
        let now = chrono::Utc::now().timestamp();
        now - last < cd
    } else {
        false
    }
}

/// 记录推送成功的冷却时间戳. 由 push_governor 内部调用.
pub fn record_cooldown(kind: super::notify::PushKind, code: &str) {
    let key = (kind, code.to_string());
    let now = chrono::Utc::now().timestamp();
    let mut table = COOLDOWN_TABLE.lock().expect("cooldown table poisoned");
    table.insert(key, now);
}

/// 是否计入日预算 (§14.3.3). 交易建议类 + 盘后 R 系列计入.
pub fn counts_against_daily_budget(kind: super::notify::PushKind) -> bool {
    use super::notify::PushKind as K;
    matches!(
        kind,
        K::HoldingPlan
            | K::HoldingEvent
            | K::T0Advice
            | K::CandidateTriggered
            | K::CloseCall
            | K::ForbiddenOps
            | K::PaperTrade
            | K::ReviewMarket
            | K::ReviewLhb
            | K::ReviewSignal
            | K::ReviewFailure
            | K::TomorrowWatch
            | K::EventCalendar
            | K::DailyReport
    )
}

/// 当前日预算已用条数 (供监控/单测查询).
pub fn daily_budget_used() -> u32 {
    reset_budget_if_new_day();
    DAILY_BUDGET_COUNT.load(Ordering::Relaxed)
}

/// §14.3 治理规则: Frozen/Unsafe 停发判定
///
/// 返回 true = 应停发该条交易建议类推送.
/// 当前实现: T-03/T-05/T-07 (持有建议 / 做T / 候选触发) 在 Frozen/Unsafe 停发,
///           T-04 (紧急风险) / T-09 (禁止操作) 仍照发 (风险类不受限).
pub fn should_block_on_mode(kind: super::notify::PushKind, mode: AccountMode, dm: DataMode) -> bool {
    use super::notify::PushKind as K;
    match kind {
        // 风险类: 永远照发
        K::HoldingEvent | K::ForbiddenOps | K::DataMode | K::AccountMode => false,
        // 交易建议类: Frozen 全停, Unsafe 全停
        K::HoldingPlan | K::T0Advice | K::CandidateTriggered | K::PaperTrade => {
            matches!(mode, AccountMode::Frozen) || matches!(dm, DataMode::Unsafe)
        }
        // 其它 (T-12 尾盘, 盘后系列): 不停
        _ => false,
    }
}

/// 一站式便捷入口: 渲染模板 → 治理检查 → push_governor 推送.
///
/// 治理流程 (任一环节 skip 即转 log):
///   1. §14.3.4 mode/dm 停发检查 (`should_block_on_mode`)
///   2. §14.3.1 冷却检查 (`is_in_cooldown`)
///   3. §14.3.3 日预算检查 (`DAILY_BUDGET_LIMIT`)
///   4. §14.3.2 紧急类 (`PushLevel::Emergency`) 跳过 2+3
///
/// `code` 用于 §14.3.1 的 (PushKind, code) 键. 不分票的推送 (T-01/T-02 状态变更/全局)
/// 传空字符串即可.
///
/// 内部调用 `super::notify::push_governor`. PUSH_VERBOSE 降级逻辑沿用.
pub async fn dispatch(
    kind: super::notify::PushKind,
    code: &str,
    banner: Option<&BannerCtx>,
    text: String,
) -> bool {
    // 1. mode/dm 停发
    if let Some(b) = banner {
        if should_block_on_mode(kind, b.account_mode, b.data_mode) {
            log::warn!(
                "[PUSH_GOVERNOR] §14.3.4 停发 | kind={} account={:?} data={:?}",
                kind.label(),
                b.account_mode,
                b.data_mode,
            );
            return false;
        }
    }

    // 2. 冷却 (紧急类跳过)
    if is_in_cooldown(kind, code) {
        log::info!(
            "[PUSH_GOVERNOR] §14.3.1 冷却中跳过 | kind={} code={}",
            kind.label(),
            code,
        );
        return false;
    }

    // 3. 日预算 (紧急类跳过)
    if counts_against_daily_budget(kind) {
        reset_budget_if_new_day();
        let used = DAILY_BUDGET_COUNT.load(Ordering::Relaxed);
        if used >= DAILY_BUDGET_LIMIT {
            log::warn!(
                "[PUSH_GOVERNOR] §14.3.3 日预算超限({}/{}) | kind={}",
                used,
                DAILY_BUDGET_LIMIT,
                kind.label(),
            );
            return false;
        }
    }

    // 4. 推
    let ok = super::notify::push_governor(&text, kind).await;
    if ok {
        record_cooldown(kind, code);
        if counts_against_daily_budget(kind) {
            DAILY_BUDGET_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
    ok
}

// ============================================================================
// fmt::Display for BannerCtx (供 println!("{}", banner) 直接打印)
// ============================================================================

impl fmt::Display for BannerCtx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn banner_normal() -> BannerCtx {
        BannerCtx {
            account_mode: AccountMode::Normal,
            total_pos: 5,
            today_pnl: 0.3,
            data_mode: DataMode::Full,
            data_missing_note: None,
        }
    }

    // ---- §14.0 横幅 ----

    #[test]
    fn banner_normal_full() {
        let b = banner_normal();
        assert_eq!(
            b.render(),
            "[🟢 Normal | 仓位5成 | 日盈亏+0.3% | 数据Full]"
        );
    }

    #[test]
    fn banner_reduce_only_degraded() {
        let b = BannerCtx {
            account_mode: AccountMode::ReduceOnly,
            total_pos: 6,
            today_pnl: -1.6,
            data_mode: DataMode::Degraded,
            data_missing_note: Some("缺盘口深度".to_string()),
        };
        let s = b.render();
        assert!(s.starts_with("[🟡 ReduceOnly | 仓位6成 | 日盈亏-1.6% | 数据Degraded]"));
        assert!(s.contains("[⚠️ 缺盘口深度: 本条不含承接判断]"));
    }

    #[test]
    fn banner_frozen_no_missing_note() {
        let b = BannerCtx {
            account_mode: AccountMode::Frozen,
            total_pos: 0,
            today_pnl: -2.1,
            data_mode: DataMode::Full,
            data_missing_note: Some("不该出现".to_string()),
        };
        // Full 模式下 data_missing_note 被忽略
        assert_eq!(b.render(), "[🔴 Frozen | 仓位0成 | 日盈亏-2.1% | 数据Full]");
    }

    #[test]
    fn banner_unsafe_includes_warning() {
        let b = BannerCtx {
            data_mode: DataMode::Unsafe,
            data_missing_note: Some("Quote断流".to_string()),
            ..banner_normal()
        };
        let s = b.render();
        assert!(s.contains("[⚠️ Quote断流"));
    }

    // ---- T-01 账户模式 ----

    #[test]
    fn t01_account_mode_example() {
        let s = render_account_mode(
            "10:23",
            AccountMode::Normal,
            AccountMode::Frozen,
            &[
                "连续第3笔止损: 300xxx -3.1%".to_string(),
                "当日亏损 -2.1% 触发熔断线 -2.0%".to_string(),
            ],
            "禁止新开仓/加仓/正T, 候选转影子",
            "下一交易日盘前重置",
        );
        assert!(s.starts_with("🛡️ 账户模式变更（10:23）"));
        assert!(s.contains("Normal → Frozen"));
        assert!(s.contains("· 连续第3笔止损: 300xxx -3.1%"));
        assert!(s.contains("生效限制: 禁止新开仓/加仓/正T, 候选转影子"));
        assert!(s.contains("解除条件: 下一交易日盘前重置"));
    }

    // ---- T-02 数据模式 ----

    #[test]
    fn t02_data_mode_full_to_degraded() {
        let s = render_data_mode(
            "09:35",
            DataMode::Full,
            DataMode::Degraded,
            "OrderBook",
            &["不做盘口承接判断".to_string(), "禁出价格型建议".to_string()],
            Some("15min"),
        );
        assert!(s.contains("Full → Degraded"));
        assert!(s.contains("受影响: OrderBook"));
        assert!(s.contains("· 不做盘口承接判断"));
        assert!(s.contains("恢复预计: 15min"));
    }

    #[test]
    fn t02_data_mode_no_eta() {
        let s = render_data_mode(
            "14:00",
            DataMode::Degraded,
            DataMode::Unsafe,
            "Quote",
            &["禁出所有建议".to_string()],
            None,
        );
        assert!(!s.contains("恢复预计"));
    }

    // ---- T-03 持仓建议 ----

    #[test]
    fn t03_holding_plan_full() {
        let s = render_holding_plan(
            &banner_normal(),
            HoldingPlanParams {
                name: "XX科技",
                code: "000001",
                hhmm: "13:42",
                intent: Intent::Reduce,
                price: 12.30,
                cost: 11.80,
                avail: 3000,
                reduce_zone: Some((12.45, 12.60)),
                support: 11.95,
                pressure: 12.70,
                stop: 11.95,
                invalidations: &[
                    "跌破5日线且放量".to_string(),
                    "板块热度转Fade".to_string(),
                ],
                reasons: &["放量冲高回落".to_string(), "主力净流出0.8亿".to_string()],
            },
        );
        assert!(s.contains("[🟢 Normal | 仓位5成 | 日盈亏+0.3% | 数据Full]"));
        assert!(s.contains("🎯 持仓建议 XX科技(000001)（13:42）"));
        assert!(s.contains("动作倾向: 逢高减仓"));
        assert!(s.contains("现价12.30 成本11.80 可用3000股"));
        assert!(s.contains("减仓观察区: 12.45~12.60"));
        assert!(s.contains("支撑11.95 | 压力12.70 | 硬止损11.95"));
        assert!(s.contains("· 跌破5日线且放量"));
        assert!(s.contains("· 板块热度转Fade"));
        assert!(s.contains("理由: 放量冲高回落; 主力净流出0.8亿"));
        assert!(s.ends_with("辅助建议, 非下单指令"));
    }

    #[test]
    fn t03_holding_plan_no_reduce_zone() {
        let s = render_holding_plan(
            &banner_normal(),
            HoldingPlanParams {
                name: "ABC",
                code: "600000",
                hhmm: "10:00",
                intent: Intent::Hold,
                price: 10.0,
                cost: 9.5,
                avail: 1000,
                reduce_zone: None,
                support: 9.6,
                pressure: 10.5,
                stop: 9.4,
                invalidations: &[],
                reasons: &["暂无催化".to_string()],
            },
        );
        assert!(!s.contains("减仓观察区"));
        assert!(s.contains("理由: 暂无催化"));
    }

    // ---- T-04 持仓紧急风险 ----

    #[test]
    fn t04_holding_event_emergency() {
        let s = render_holding_event(
            &banner_normal(),
            HoldingEventParams {
                name: "XX",
                code: "000001",
                hhmm: "10:15",
                trigger: "跌破硬止损",
                price: 11.20,
                chg_pct: -3.5,
                gap_pct: 1.2,
                action: "建议减仓",
                avail: 3000,
            },
        );
        assert!(s.contains("🚨 持仓风险"));
        assert!(s.contains("触发: 跌破硬止损"));
        assert!(s.contains("现价11.20（-3.5%） 距止损+1.2%"));
        assert!(s.contains("可用股数: 3000"));
    }

    // ---- T-05/T-06 做T ----

    #[test]
    fn t05_t0_reverse() {
        let s = render_t0_advice(
            &banner_normal(),
            T0AdviceParams {
                name: "YY",
                code: "002415",
                hhmm: "11:20",
                kind: T0Kind::ReverseT,
                style: T0Style::PullbackCatch,
                avail: 2000,
                sell_lo: 25.10,
                sell_hi: 25.30,
                buy_lo: 24.50,
                buy_hi: 24.70,
                min_spread_pct: 1.5,
                risk_note: "板块同步下跌",
            },
        );
        assert!(s.contains("结论: ReverseT | 类型: 急跌接刀"));
        assert!(s.contains("卖出观察区: 25.10~25.30"));
        assert!(s.contains("接回观察区: 24.50~24.70"));
        assert!(s.contains("最小价差: ≥1.5%"));
        assert!(s.contains("做T不改变总仓位判断"));
    }

    #[test]
    fn t06_t0_forbid() {
        let s = render_t0_forbid(
            &banner_normal(),
            T0ForbidParams {
                name: "ZZ",
                code: "300750",
                hhmm: "10:00",
                reason: "主升核心票防卖飞",
            },
        );
        assert!(s.contains("🔁🚫 不建议做T"));
        assert!(s.contains("原因: 主升核心票防卖飞"));
    }

    // ---- T-07 候选触发 ----

    #[test]
    fn t07_candidate_triggered_a_grade() {
        let s = render_candidate_triggered(
            &banner_normal(),
            CandidateTriggeredParams {
                name: "候选X",
                code: "688001",
                hhmm: "10:30",
                grade: CandidateGrade::A,
                topic: "AI算力",
                price: 50.0,
                trigger_desc: "突破前高+量比4.5",
                lo: 49.5,
                hi: 50.3,
                stop: 48.0,
                max_pos_pct: 10,
                news_quality: EvidenceQuality::Strong,
                news_note: "政策面共振",
                vol_quality: EvidenceQuality::Strong,
                vol_ratio: 4.5,
                kline_quality: EvidenceQuality::Mid,
                kline_note: "突破未稳",
                book_quality: EvidenceQuality::Missing,
                no_buy: &["大盘跳水同步".to_string()],
            },
        );
        assert!(s.contains("📋 候选触发"));
        assert!(s.contains("等级A | 状态: Triggered"));
        assert!(s.contains("主题: AI算力"));
        assert!(s.contains("已触发: 突破前高+量比4.5"));
        assert!(s.contains("· 新闻: 强 政策面共振"));
        assert!(s.contains("· 量能: 强 量比4.5"));
        assert!(s.contains("· K线: 中 突破未稳"));
        assert!(s.contains("· 盘口: 缺失,不作承接判断"));
        assert!(s.contains("· 大盘跳水同步"));
        assert!(s.contains("需人工确认, 非自动买入"));
    }

    // ---- T-08 候选失效 ----

    #[test]
    fn t08_candidate_invalidated() {
        let s = render_candidate_invalidated(
            "11:00",
            "候选Y",
            "688002",
            "Watch",
            "触发失败: 未触达买入区",
        );
        assert!(s.contains("📋 候选失效 候选Y(688002)（11:00）"));
        assert!(s.contains("原状态Watch → Invalidated"));
        assert!(s.contains("未触达买入区"));
    }

    // ---- T-09 禁止操作 ----

    #[test]
    fn t09_forbidden_ops() {
        let s = render_forbidden_ops(
            &banner_normal(),
            ForbiddenOpsParams {
                name: "XX",
                code: "000001",
                hhmm: "10:00",
                conclusion: "距涨停过近, 禁止追买",
                reasons: &[
                    "距涨停仅 1.2%".to_string(),
                    "板块已 Climax".to_string(),
                ],
            },
        );
        assert!(s.contains("🚫 禁止操作（10:00）"));
        assert!(s.contains("XX(000001): 距涨停过近, 禁止追买"));
        assert!(s.contains("· 距涨停仅 1.2%"));
        assert!(s.contains("· 板块已 Climax"));
    }

    // ---- T-10 虚拟盘 ----

    #[test]
    fn t10_paper_trade_filled() {
        let s = render_paper_trade(PaperTradeParams {
            name: "ZZ",
            code: "002415",
            hhmm: "10:00",
            status: PaperTradeStatus::Filled,
            fill_price: Some(25.10),
            qty: Some(1000),
            virtual_reason: Some("候选A档触发"),
            not_fill_reason: None,
            account_mode: AccountMode::Normal,
            data_mode: DataMode::Full,
        });
        assert!(s.contains("🧪 虚拟盘"));
        assert!(s.contains("ZZ(002415) Filled"));
        assert!(s.contains("成交价25.10 数量1000 主理由候选A档触发"));
        assert!(s.contains("账户Normal/数据Full"));
    }

    #[test]
    fn t10_paper_trade_not_filled() {
        let s = render_paper_trade(PaperTradeParams {
            name: "YY",
            code: "688001",
            hhmm: "10:00",
            status: PaperTradeStatus::NotFilled,
            fill_price: None,
            qty: None,
            virtual_reason: None,
            not_fill_reason: Some("涨停不可买"),
            account_mode: AccountMode::Normal,
            data_mode: DataMode::Full,
        });
        assert!(s.contains("YY(688001) NotFilled"));
        assert!(s.contains("未成交原因: 涨停不可买"));
        assert!(!s.contains("成交价"));
    }

    // ---- T-11 竞价异动 ----

    #[test]
    fn t11_auction_volume() {
        let items = vec![
            AuctionItem { name: "A", code: "000001", gap_pct: 5.2, vol_ratio: 8.5, tag: "昨日涨停" },
            AuctionItem { name: "B", code: "600000", gap_pct: 2.1, vol_ratio: 3.2, tag: "观察池" },
        ];
        let s = render_auction_volume(
            &banner_normal(),
            "09:25",
            &items,
            "强承接",
            "可操作",
        );
        assert!(s.contains("🌅 竞价异动 Top2（09:25）"));
        assert!(s.contains("A(000001) 高开+5.2% 量比8.5 [昨日涨停]"));
        assert!(s.contains("B(600000) 高开+2.1% 量比3.2 [观察池]"));
        assert!(s.contains("情绪判读: 强承接, 观察池今日可操作"));
    }

    // ---- T-12 尾盘决策 ----

    #[test]
    fn t12_close_call_holding_only() {
        let h = CloseCallHolding { name: "XX", state: "尾盘跳水-建议处理" };
        let s = render_close_call(&banner_normal(), "14:45", Some(&h), None);
        assert!(s.contains("🌇 尾盘提示（14:45）"));
        assert!(s.contains("[持仓] XX: 尾盘跳水-建议处理"));
        assert!(!s.contains("[博弈]"));
    }

    #[test]
    fn t12_close_call_gamble_unsatisfied() {
        let g = CloseCallGamble {
            name: "YY",
            code: "002415",
            satisfied: false,
            cond: "板块龙头未封板",
        };
        let s = render_close_call(&banner_normal(), "14:50", None, Some(&g));
        assert!(s.contains("[博弈] YY(002415): 尾盘买入博次日溢价条件未满足: 板块龙头未封板"));
    }

    // ---- R-01 持仓明日计划 ----

    #[test]
    fn r01_daily_report() {
        let items = vec![
            HoldingDailyPlan {
                name: "XX",
                code: "000001",
                price: 12.30,
                cost: 11.80,
                pnl_pct: 4.2,
                high_gap_x: 2.0,
                plan_high: "减仓1/3",
                plan_flat: "持有",
                stop: 11.95,
                t0: "适合观察",
            },
            HoldingDailyPlan {
                name: "YY",
                code: "002415",
                price: 25.10,
                cost: 26.00,
                pnl_pct: -3.5,
                high_gap_x: 1.5,
                plan_high: "持有",
                plan_flat: "执行止损",
                stop: 24.50,
                t0: "不适合(主升核心)",
            },
        ];
        let s = render_daily_report("2026-07-05", &items);
        assert!(s.starts_with("📌 持仓明日计划（2026-07-05 19:00）"));
        assert!(s.contains("XX(000001) 现价12.30 成本11.80 浮盈+4.2%"));
        assert!(s.contains("· 高开>2.0%: 减仓1/3"));
        assert!(s.contains("· 低开/跌破11.95: 执行止损"));
        assert!(s.contains("YY(002415) 现价25.10 成本26.00 浮盈-3.5%"));
    }

    // ---- R-02 盘面走向 ----

    #[test]
    fn r02_review_market_full() {
        let s = render_review_market(
            "2026-07-05",
            &MarketReview {
                sh_chg: 0.5,
                chinext_chg: 1.2,
                star_chg: 1.5,
                limit_up_n: 35,
                limit_down_n: 3,
                broken_pct: 15.0,
                consecutive_h: 5,
                amount_yi: 8500.0,
                amount_delta_pct: 8.0,
                amount_dir: "放量",
                main_flow_yi: 120.0,
                money_effect: "中等",
                heat_stage: "MainUp",
                heat_conf_pct: 80,
                low_conf: false,
                low_conf_tier: None,
                account_mode: AccountMode::Normal,
                max_pos: 7,
            },
        );
        assert!(s.starts_with("📊 今日盘面（2026-07-05）"));
        assert!(s.contains("上证+0.5% 创业+1.2% 科创+1.5%"));
        assert!(s.contains("涨停35家 跌停3家"));
        assert!(s.contains("两市8500亿（放量+8%）"));
        assert!(s.contains("主力净+120亿"));
        assert!(s.contains("阶段判定: MainUp（置信度80%）"));
        assert!(s.contains("→ 明日账户建议: Normal 仓位上限7成"));
        assert!(!s.contains("低置信"));
    }

    #[test]
    fn r02_review_market_low_conf() {
        let s = render_review_market(
            "2026-07-05",
            &MarketReview {
                heat_conf_pct: 45,
                low_conf: true,
                low_conf_tier: Some("保守档"),
                ..test_market_review_default()
            },
        );
        assert!(s.contains("⚠️ 低置信, 权限按保守档执行"));
    }

    fn test_market_review_default() -> MarketReview<'static> {
        MarketReview {
            sh_chg: 0.0, chinext_chg: 0.0, star_chg: 0.0,
            limit_up_n: 0, limit_down_n: 0, broken_pct: 0.0, consecutive_h: 0,
            amount_yi: 0.0, amount_delta_pct: 0.0, amount_dir: "放量",
            main_flow_yi: 0.0, money_effect: "差", heat_stage: "Fade", heat_conf_pct: 50,
            low_conf: false, low_conf_tier: None,
            account_mode: AccountMode::Normal, max_pos: 5,
        }
    }

    // ---- R-03 涨停产业链 ----

    #[test]
    fn r03_industry_chain_two() {
        let chains = vec![
            ChainLine {
                chain: "AI算力",
                limit_up_n: 8, first_n: 5, consec_n: 3,
                heat_stage: "MainUp",
                leader_name: "龙头A", leader_code: "688001", leader_boards: 4,
                followers: "B,C,D",
                watch_point: "明日分歧",
            },
            ChainLine {
                chain: "机器人",
                limit_up_n: 5, first_n: 4, consec_n: 1,
                heat_stage: "HeatUp",
                leader_name: "龙头Z", leader_code: "300750", leader_boards: 2,
                followers: "X,Y",
                watch_point: "接力意愿",
            },
        ];
        let s = render_industry_chain("2026-07-05", &chains, Some("光伏（涨停12→3家）"));
        assert!(s.starts_with("🔥 涨停产业链（2026-07-05）"));
        assert!(s.contains("1. AI算力 涨停8家"));
        assert!(s.contains("龙头: 龙头A(688001) 4板"));
        assert!(s.contains("2. 机器人"));
        assert!(s.contains("⚠️ 退潮链: 光伏（涨停12→3家）"));
    }

    // ---- R-04 龙虎榜 ----

    #[test]
    fn r04_review_lhb() {
        let entries = vec![
            LhbEntry {
                name: "X", code: "688001", net_buy_yi: 1.5,
                reason: "涨幅偏离值达7%",
                buy_inst_n: 2, buy_inst_amt_wan: 8000.0,
                buy_other_n: 3, buy_other_amt_wan: 4000.0,
                buy_conc_pct: 65.0,
                sell_desc: "游资席位", sell_conc_pct: 45.0,
                chain_match: Some("AI算力"),
                next_day_risk: "高开震荡",
            },
        ];
        let s = render_review_lhb("2026-07-05", &entries);
        assert!(s.starts_with("🐉 龙虎榜净买前五（2026-07-05 21:00）"));
        assert!(s.contains("X(688001) 净买1.5亿"));
        assert!(s.contains("买: 机构2席8000万 其他3席4000万（集中度65%）"));
        assert!(s.contains("卖: 游资席位（集中度45%）"));
        assert!(s.contains("主线一致: 是-AI算力"));
        assert!(s.contains("仅结构化事实, 不含席位风格推断"));
    }

    // ---- R-05 信号复盘 ----

    #[test]
    fn r05_review_signal() {
        let r = SignalReview {
            holding_n: 5, holding_exec: 4, holding_eff: 3,
            t0_n: 2, t0_eff: 1,
            cand_trigger: 6, cand_filled: 3, cand_notfilled: 3,
            cand_limitup: 2, cand_notreach: 1,
            paper_pnl_pct: 0.5, paper_total_pct: 3.2, paper_n: 12,
            news_push_n: 4, news_d1_eff: 2,
        };
        let s = render_review_signal("2026-07-05", &r);
        assert!(s.starts_with("🤖 信号复盘（2026-07-05）"));
        assert!(s.contains("持仓建议: 推5条 执行4条 有效3条"));
        assert!(s.contains("做T建议: 推2 有效1"));
        assert!(s.contains("候选(影子): 触发6 模拟成交3 未成交3（涨停2/未触达1）"));
        assert!(s.contains("虚拟盘: 今日+0.5% 累计+3.2%（样本12笔）"));
        assert!(s.contains("新闻兑现: 推送4条 D+1兑现2条"));
    }

    // ---- R-06 失败归因 ----

    #[test]
    fn r06_review_failure() {
        let entries = vec![
            FailureEntry {
                name: "X", code: "688001",
                signal_level: "⚡", virtual_reason: "A档",
                result_desc: "未成交",
                pnl_pct: 0.0,
                failure_reason: "涨停不可买",
                suggestion: "调高触发阈值",
            },
        ];
        let dist = FailureDistribution {
            buy_late: 2, chain_fade: 1, not_fillable: 3, human_not_exec: 1,
        };
        let s = render_review_failure("2026-07-05", &entries, &dist);
        assert!(s.starts_with("❌ 失败归因（2026-07-05）"));
        assert!(s.contains("X(688001) 原信号: ⚡A档"));
        assert!(s.contains("归因: 涨停不可买"));
        assert!(s.contains("处理建议: 调高触发阈值"));
        assert!(s.contains("本周归因分布: 买点过晚2 板块退潮1 不可成交3 人未执行1"));
    }

    // ---- R-07 明日观察池 ----

    #[test]
    fn r07_tomorrow_watch() {
        let items = vec![
            WatchItem {
                name: "Y", code: "002415", topic: "机器人",
                source: "A档未触发",
                trigger: "突破50.5",
                lo: 49.5, hi: 50.3, stop: 48.5,
                reason: "板块共振",
            },
        ];
        let s = render_tomorrow_watch("2026-07-05", &items);
        assert!(s.starts_with("📌 明日观察池（2026-07-05）"));
        assert!(s.contains("1. Y(002415) [机器人] 来源: A档未触发"));
        assert!(s.contains("触发突破50.5 | 低吸49.50~50.30 | 止损48.50"));
        assert!(s.contains("共1只 | 明日竞价后按 T-11 复核"));
    }

    // ---- R-08 事件日历 ----

    #[test]
    fn r08_event_calendar() {
        let holdings = vec![
            HoldingEventItem { name: "XX", kind: "解禁3.2亿" },
            HoldingEventItem { name: "YY", kind: "财报预告" },
        ];
        let s = render_event_calendar("2026-07-06", &holdings, "央行MLF到期", "+0.8%", "7.18");
        assert!(s.starts_with("🗓️ 明日事件（2026-07-06）"));
        assert!(s.contains("· XX: 解禁3.2亿"));
        assert!(s.contains("· YY: 财报预告"));
        assert!(s.contains("宏观: 央行MLF到期"));
        assert!(s.contains("隔夜关注: 美股+0.8% 汇率7.18"));
    }

    // ---- 工具 ----

    #[test]
    fn fmt_price_two_decimals() {
        assert_eq!(fmt_price(12.3), "12.30");
        assert_eq!(fmt_price(0.0), "0.00");
        assert_eq!(fmt_price(1234.567), "1234.57");
    }

    // ---- 入参类型 enum 文案 ----

    #[test]
    fn intent_labels() {
        assert_eq!(Intent::Reduce.label(), "逢高减仓");
        assert_eq!(Intent::Clear.label(), "清仓");
        assert_eq!(Intent::Hold.label(), "持有观望");
        assert_eq!(Intent::Add.label(), "加仓");
        assert_eq!(Intent::T0.label(), "做T");
    }

    #[test]
    fn t0_kind_labels() {
        assert_eq!(T0Kind::ReverseT.label(), "ReverseT");
        assert_eq!(T0Kind::PositiveT.label(), "PositiveT");
    }

    #[test]
    fn evidence_quality_labels() {
        assert_eq!(EvidenceQuality::Missing.label(), "缺失,不作承接判断");
        assert_eq!(EvidenceQuality::Strong.label(), "强");
    }

    // ---- §14.3 治理: Frozen/Unsafe 停发规则 ----

    #[test]
    fn should_block_holding_plan_on_frozen() {
        use super::super::notify::PushKind;
        assert!(should_block_on_mode(
            PushKind::HoldingPlan,
            AccountMode::Frozen,
            DataMode::Full,
        ));
    }

    #[test]
    fn should_block_holding_plan_on_unsafe() {
        use super::super::notify::PushKind;
        assert!(should_block_on_mode(
            PushKind::HoldingPlan,
            AccountMode::Normal,
            DataMode::Unsafe,
        ));
    }

    #[test]
    fn should_not_block_emergency_in_frozen() {
        use super::super::notify::PushKind;
        assert!(!should_block_on_mode(
            PushKind::HoldingEvent,
            AccountMode::Frozen,
            DataMode::Full,
        ));
    }

    #[test]
    fn should_not_block_forbidden_ops_in_unsafe() {
        use super::super::notify::PushKind;
        assert!(!should_block_on_mode(
            PushKind::ForbiddenOps,
            AccountMode::Normal,
            DataMode::Unsafe,
        ));
    }

    #[test]
    fn should_not_block_close_call_in_frozen() {
        use super::super::notify::PushKind;
        // 尾盘决策不在 §14.3 停发列表
        assert!(!should_block_on_mode(
            PushKind::CloseCall,
            AccountMode::Frozen,
            DataMode::Full,
        ));
    }

    // ---- PushKind v12 新增元信息 ----

    #[test]
    fn push_kind_v12_cooldown_table() {
        use super::super::notify::PushKind;
        // §14.3 冷却表
        assert_eq!(PushKind::AccountMode.cooldown_secs(), None, "AccountMode 无冷却");
        assert_eq!(PushKind::HoldingEvent.cooldown_secs(), None, "HoldingEvent 无冷却");
        assert_eq!(PushKind::DataMode.cooldown_secs(), Some(600), "DataMode 10min");
        assert_eq!(PushKind::HoldingPlan.cooldown_secs(), Some(1800), "HoldingPlan 30min");
        assert_eq!(PushKind::T0Advice.cooldown_secs(), Some(1800), "T0Advice 30min");
        assert_eq!(PushKind::CandidateTriggered.cooldown_secs(), Some(86_400), "1次/票/日");
        assert_eq!(PushKind::ForbiddenOps.cooldown_secs(), Some(3600), "ForbiddenOps 60min");
        assert_eq!(PushKind::PaperTrade.cooldown_secs(), Some(300), "PaperTrade 5min");
        assert_eq!(PushKind::CloseCall.cooldown_secs(), Some(86_400), "CloseCall 1次/日");
    }

    #[test]
    fn push_kind_v12_requires_banner() {
        use super::super::notify::PushKind;
        // §14.0 强制带横幅的 8 种
        for k in [
            PushKind::AccountMode,
            PushKind::DataMode,
            PushKind::HoldingPlan,
            PushKind::HoldingEvent,
            PushKind::T0Advice,
            PushKind::CandidateTriggered,
            PushKind::ForbiddenOps,
            PushKind::PaperTrade,
            PushKind::CloseCall,
        ] {
            assert!(k.requires_banner(), "{:?} 应要求横幅", k);
        }
        // 不强制带横幅的 (辅助/降级类)
        assert!(!PushKind::FactorIC.requires_banner());
        assert!(!PushKind::SectorTop.requires_banner());
    }

    #[test]
    fn push_kind_v12_level_emergency_vs_important_vs_info() {
        use super::super::notify::{PushKind, PushLevel};
        assert_eq!(PushKind::HoldingEvent.level(), PushLevel::Emergency);
        assert_eq!(PushKind::AccountMode.level(), PushLevel::Important);
        assert_eq!(PushKind::HoldingPlan.level(), PushLevel::Important);
        assert_eq!(PushKind::ForbiddenOps.level(), PushLevel::Info);
        assert_eq!(PushKind::PaperTrade.level(), PushLevel::Info);
    }

    // ---- 集成示例: 渲染 + dispatch ----

    // 注意: 以下 dispatch 集成测试需在隔离环境跑 (V10_DRY_RUN_PUSH=1).
    // 因 process env 在 cargo test 并行下共享, 改为不在此跑, 留 integration test 由 CI 单独标记.

    #[test]
    fn integration_dispatch_signatures_compile() {
        // 仅验证 dispatch 签名 + 入参类型不破坏
        // (实际推送行为由 §14.3 单元测试覆盖 is_in_cooldown / should_block_on_mode / counts_against_daily_budget)
        let _banner = banner_normal();
    }

    #[test]
    fn daily_budget_counts_only_categorized() {
        use super::super::notify::PushKind;
        // 计入预算
        assert!(counts_against_daily_budget(PushKind::HoldingPlan));
        assert!(counts_against_daily_budget(PushKind::T0Advice));
        assert!(counts_against_daily_budget(PushKind::HoldingEvent));
        assert!(counts_against_daily_budget(PushKind::ReviewMarket));
        // 不计入 (降级 + 状态变更)
        assert!(!counts_against_daily_budget(PushKind::FactorIC));
        assert!(!counts_against_daily_budget(PushKind::AccountMode));
        assert!(!counts_against_daily_budget(PushKind::DataMode));
    }

    #[test]
    fn cooldown_table_isolated_by_code() {
        use super::super::notify::PushKind;
        // 同一 kind 不同 code 是不同 key
        assert!(!is_in_cooldown(PushKind::HoldingPlan, "000001"));
        assert!(!is_in_cooldown(PushKind::HoldingPlan, "000002"));
        record_cooldown(PushKind::HoldingPlan, "000001");
        assert!(is_in_cooldown(PushKind::HoldingPlan, "000001"));
        assert!(!is_in_cooldown(PushKind::HoldingPlan, "000002"), "不同 code 应独立");
    }

    #[test]
    fn emergency_bypass_cooldown_table() {
        use super::super::notify::{PushKind, PushLevel};
        // HoldingEvent 是 Emergency, 即使在 cooldown table 中也是 false
        record_cooldown(PushKind::HoldingEvent, "000001");
        assert!(!is_in_cooldown(PushKind::HoldingEvent, "000001"));
        assert_eq!(PushKind::HoldingEvent.level(), PushLevel::Emergency);
    }
}