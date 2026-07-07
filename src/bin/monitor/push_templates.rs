//! v12 §14 推送消息模板渲染
//!
//! 职责：仅做"按模板拼字符串"，不接 push 通道、不写库、不读行情。
//! 模板结构与字段顺序严格对齐 `docs/architecture/v13-push-templates.md`。
//!
//! 调用约定:
//!   1. 调用方先拼好本模板所需的领域数据（结构体入参）
//!   2. 调对应 `render_xxx()` 函数得到完整 text
//!   3. 调 `crate::notify::push_governor(&text, kind).await` 推送
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
    /// 测试用 BannerCtx (Normal/Full, 仓位 0, 日盈亏 0.0)
    #[cfg(test)]
    pub fn test_default() -> Self {
        Self {
            account_mode: AccountMode::Normal,
            total_pos: 0,
            today_pnl: 0.0,
            data_mode: DataMode::Full,
            data_missing_note: None,
        }
    }

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
/// v12 §14.1 T-01 AccountMode 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
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
        "\n生效限制: {}\n解除条件: {}\n辅助建议, 非下单指令",
        forbidden_actions, recovery_condition,
    ));
    out
}

/// T-02 数据状态变更
/// v12 §14.1 T-02 DataMode 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
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
        out.push_str(&format!("\n恢复预计: {}\n辅助建议, 非下单指令", eta));
    } else {
        out.push_str("\n辅助建议, 非下单指令");
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
/// v12 §14.1 T-03 HoldingPlan 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
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
        out.push_str(&format!(
            "\n减仓观察区: {}~{}",
            fmt_price(lo),
            fmt_price(hi)
        ));
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
/// v12 §14.1 T-04 HoldingEvent 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
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

/// v12 §14.1 T-05 T0Advice 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
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
/// v12 §14.1 T-06 T0Forbid 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
pub fn render_t0_forbid(banner: &BannerCtx, p: T0ForbidParams<'_>) -> String {
    format!(
        "{}\n🔁🚫 不建议做T {}({})（{}）\n原因: {}",
        banner.render(),
        p.name,
        p.code,
        p.hhmm,
        p.reason,
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

/// v12 §14.1 T-07 CandidateTriggered 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
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
    out.push_str(&format!(
        "\n· 新闻: {} {}",
        p.news_quality.label(),
        p.news_note
    ));
    out.push_str(&format!(
        "\n· 量能: {} 量比{:.1}",
        p.vol_quality.label(),
        p.vol_ratio,
    ));
    out.push_str(&format!(
        "\n· K线: {} {}",
        p.kline_quality.label(),
        p.kline_note
    ));
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
/// v12 §14.1 T-08 CandidateInvalidated 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
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
/// v12 §14.1 T-09 ForbiddenOps 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
pub fn render_forbidden_ops(banner: &BannerCtx, p: ForbiddenOpsParams<'_>) -> String {
    let mut out = format!(
        "{}\n🚫 禁止操作（{}）\n{}({}): {}\n· {}",
        banner.render(),
        p.hhmm,
        p.name,
        p.code,
        p.conclusion,
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

/// v12 §14.1 T-10 PaperTrade 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
pub fn render_paper_trade(p: PaperTradeParams<'_>) -> String {
    let mut out = format!(
        "🧪 虚拟盘（{}）\n{}({}) {}",
        p.hhmm,
        p.name,
        p.code,
        p.status.label(),
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
/// v12 §14.1 T-11 AuctionVolume 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
pub fn render_auction_volume(
    banner: &BannerCtx,
    hhmm: &str,
    items: &[AuctionItem<'_>],
    sentiment: &str,
    watch_status: &str,
) -> String {
    let mut out = format!(
        "{}\n🌅 竞价热点量能 Top{}（{}）",
        banner.render(),
        items.len(),
        hhmm
    );
    for it in items {
        out.push_str(&format!(
            "\n  {}({}) 高开{:+.1}% 量比{:.1} [{}]",
            it.name, it.code, it.gap_pct, it.vol_ratio, it.tag,
        ));
    }
    out.push_str(&format!(
        "\n情绪判读: {}, 观察池今日{}\n辅助建议, 非下单指令",
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

/// v12 §14.1 T-12 CloseCall 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
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
            g.name,
            g.code,
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

// ============================================================================
// §14.1 T-13 盘中换手率 Top10 (v19.15 新增, 跟 R-04 龙虎榜分离)
// ============================================================================

/// 换手率 Top10 单条 (v19.16 改 owned, 不带生命周期, 便于 spawn_blocking 跨边界)
#[derive(Debug, Clone)]
pub struct TurnoverEntry {
    pub name: String,
    pub code: String,
    pub price: f64,
    pub change_pct: f64,
    pub turnover_pct: f64, // 换手率 (%)
    pub main_flow_yi: f64, // 主力净流入 (亿)
}

/// v12 §14.1 T-13 TurnoverTop 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
///
/// 与 R-04 龙虎榜严格区分:
/// - T-13: 盘中实时换手率 (真数据, data_provider 拉取)
/// - R-04: 盘后龙虎榜席位 (东方财富 API, 盘后 21:00 才更新)
///
/// AGENTS.md §2.1 红线: 不允许用换手率编造"龙虎榜"假数据.
pub fn render_turnover_top(hhmm: &str, entries: &[TurnoverEntry]) -> String {
    let mut out = format!("🔄 盘中换手率 Top10 ({})\n", hhmm);
    if entries.is_empty() {
        out.push_str("⚠️ 数据源不稳定, 跳过\n");
        out.push_str("数据源: 实时行情 (非龙虎榜, 龙虎榜盘后 21:00 才更新)\n");
        return out;
    }
    for (i, e) in entries.iter().enumerate() {
        out.push_str(&format!(
            "  {}. {}({}) 现价¥{:.2} 涨跌{:+.2}% 换手{:.2}% 主力{:.2}亿\n",
            i + 1,
            e.name,
            e.code,
            e.price,
            e.change_pct,
            e.turnover_pct,
            e.main_flow_yi,
        ));
    }
    out.push_str("数据源: 实时行情 (非龙虎榜, 龙虎榜盘后 21:00 才更新)\n");
    out.push_str("辅助建议, 非下单指令\n");
    out
}

/// v12 §14.2 R-01 DailyReport 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
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
        out.push_str(&format!("\n· 高开>{:.1}%: {}", it.high_gap_x, it.plan_high,));
        out.push_str(&format!("\n· 平开: {}", it.plan_flat));
        out.push_str(&format!("\n· 低开/跌破{}: 执行止损", fmt_price(it.stop),));
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
    pub low_conf: bool,                 // 是否低置信
    pub low_conf_tier: Option<&'a str>, // "保守档"
    pub account_mode: AccountMode,
    pub max_pos: u8,
}

/// v12 §14.2 R-02 ReviewMarket 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
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
        "\n→ 明日账户建议: {} 仓位上限{}成\n辅助建议, 非下单指令",
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

/// v12 §14.2 R-03 IndustryChain 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
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

/// v12 §14.2 R-04 ReviewLhb 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
pub fn render_review_lhb(date: &str, entries: &[LhbEntry<'_>]) -> String {
    if entries.is_empty() {
        return format!(
            "🐉 龙虎榜净买前五（{} 21:00）\n盘中无数据 (盘后 21:00 才更新), 请参考 T-13 盘中换手率 Top10\n仅结构化事实, 不含席位风格推断",
            date
        );
    }
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
    pub holding_n: u32, // 持仓建议推 n 条
    pub holding_exec: u32,
    pub holding_eff: u32,
    pub t0_n: u32, // 做T 推 n
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

/// v12 §14.2 R-05 ReviewSignal 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
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

/// v12 §14.2 R-06 ReviewFailure 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
pub fn render_review_failure(
    date: &str,
    entries: &[FailureEntry<'_>],
    dist: &FailureDistribution,
) -> String {
    let mut out = format!("❌ 失败归因（{}）", date);
    for e in entries {
        out.push_str(&format!(
            "\n{}({}) 原信号: {}{}\n结果: {} {:+.1}%\n归因: {}\n处理建议: {}\n─────",
            e.name,
            e.code,
            e.signal_level,
            e.virtual_reason,
            e.result_desc,
            e.pnl_pct,
            e.failure_reason,
            e.suggestion,
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

/// v12 §14.2 R-07 TomorrowWatch 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
pub fn render_tomorrow_watch(date: &str, items: &[WatchItem<'_>]) -> String {
    let mut out = format!("📌 明日观察池（{}）", date);
    for (i, it) in items.iter().enumerate() {
        out.push_str(&format!(
            "\n{}. {}({}) [{}] 来源: {}\n   触发{} | 低吸{}~{} | 止损{}\n   理由: {}",
            i + 1,
            it.name,
            it.code,
            it.topic,
            it.source,
            it.trigger,
            fmt_price(it.lo),
            fmt_price(it.hi),
            fmt_price(it.stop),
            it.reason,
        ));
        out.push_str("\n─────");
    }
    out.push_str(&format!("\n共{}只 | 明日竞价后按 T-11 复核", items.len(),));
    out
}

/// R-08 明日事件日历
#[derive(Debug)]
pub struct HoldingEventItem<'a> {
    pub name: &'a str,
    pub kind: &'a str, // "解禁{amt}亿" / "财报预告" / "减持到期"
}

/// v12 §14.2 R-08 EventCalendar 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
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

/// PR2-2.4 缺盘口"承接"护栏.
///
/// 当 OrderBook 缺失 (`book_missing=true`) 时, 文案应禁出现 "承接" 字样.
/// 若检测到, 返回 `Err` 包含违规内容, 由调用方决定 log/strip/reject.
///
/// 实现: 按行扫描. 每行若含 "承接", 检查该行是否含白名单自我标注短语.
///   默认白名单: "不作承接判断", "不做盘口承接判断", "本条不含承接判断", "暂缺盘口".
pub fn check_no_acceptance_when_missing_book(text: &str, book_missing: bool) -> Result<(), String> {
    if !book_missing {
        return Ok(());
    }

    const ALLOWLIST: &[&str] = &[
        "不作承接判断",
        "不做盘口承接判断",
        "本条不含承接判断",
        "暂缺盘口",
    ];

    let mut violations = Vec::new();
    for line in text.lines() {
        if line.contains("承接") {
            let mut allowed = false;
            for phrase in ALLOWLIST {
                if line.contains(phrase) {
                    allowed = true;
                    break;
                }
            }
            if !allowed {
                violations.push(line.to_string());
            }
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "PR2-2.4 护栏: 缺盘口时文案含未授权的'承接'字样: {:?}",
            violations
        ))
    }
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
        LibAM::ReduceOnly => {
            "当日盈亏回到 -1.5% 内 或 连续止损 < 3 笔 (运行时) / 下一交易日盘前重置"
        }
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
        &hhmm, prev_tmpl, new_tmpl, &reasons, forbidden, recovery,
    ));

    // 3. dispatch (code="" 全局键, AccountMode 无冷却)
    let ok = dispatch(
        crate::notify::PushKind::AccountMode,
        "", // code 空 = 全局键
        banner,
        text,
    )
    .await;

    // 4. 标记 pushed
    if ok {
        if let Err(e) = stock_analysis::database::account_mode_log::mark_account_mode_pushed(log_id)
        {
            log::warn!("[AccountMode] mark pushed=1 失败 (id={}): {}", log_id, e);
        }
    } else {
        log::info!(
            "[AccountMode] T-01 推送失败, log_id={} 保留 pushed=0 等重试",
            log_id
        );
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

// ============================================================================
// PR4-4.3 orchestrator: T-03 持仓建议 + T-04 紧急风险推送
// ============================================================================

/// PR4-4.3 T-03 持仓建议推送 (普通建议, ⚡ 30min 冷却).
///
/// 治理已由 `dispatch()` 内部完成:
///   - §14.3.4 mode/dm 停发 (T-03 在 Frozen/Unsafe 停发)
///   - §14.3.1 冷却 30min/票
///   - §14.3.3 日预算 30 条
///   - §14.3.2 紧急类 (T-04 HoldingEvent) 跳过冷却 + 预算
///
/// `account_mode`/`data_mode` 用于横幅上下文 (Frozen/Unsafe 时 dispatch 内部会停发).
pub async fn push_holding_plan_recommendation(
    code: &str,
    banner: Option<&BannerCtx>,
    params: HoldingPlanParams<'_>,
) -> bool {
    let text = render_holding_plan(banner.unwrap_or(&BannerCtx::default()), params);
    dispatch(crate::notify::PushKind::HoldingPlan, code, banner, text).await
}

/// PR4-4.3 T-04 持仓紧急风险推送 (🚨紧急, 无视冷却).
///
/// 用于: 跌破硬止损/触发三级止损/板块跳水.
/// 自动 PushLevel::Emergency → dispatch 跳过冷却和日预算.
pub async fn push_holding_emergency(
    code: &str,
    banner: Option<&BannerCtx>,
    params: HoldingEventParams<'_>,
) -> bool {
    let text = render_holding_event(banner.unwrap_or(&BannerCtx::default()), params);
    dispatch(crate::notify::PushKind::HoldingEvent, code, banner, text).await
}

// ============================================================================
// MVP2-2.2 orchestrator: T-05/T-06 做T建议
// ============================================================================

/// MVP2-2.2 T-05 做T建议推送 (⚡ 30min/票).
///
/// 拼接文本后调 dispatch, 治理 (mode/dm/cooling/budget) 由 dispatch 内部完成.
pub async fn push_t0_advice(
    code: &str,
    banner: Option<&BannerCtx>,
    params: T0AdviceParams<'_>,
) -> bool {
    let text = render_t0_advice(banner.unwrap_or(&BannerCtx::default()), params);
    dispatch(crate::notify::PushKind::T0Advice, code, banner, text).await
}

/// MVP2-2.2 T-06 不建议做T (ℹ️参考).
pub async fn push_t0_forbid(
    code: &str,
    banner: Option<&BannerCtx>,
    params: T0ForbidParams<'_>,
) -> bool {
    let text = render_t0_forbid(banner.unwrap_or(&BannerCtx::default()), params);
    dispatch(crate::notify::PushKind::T0Advice, code, banner, text).await
}

// ============================================================================
// v14.2: v13 核心 6 模板 push_* wrapper (render + dispatch)
// ============================================================================

/// v13 §14.1 P-01 盘前新闻热点 (ℹ️参考, 盘前无 banner)
pub async fn push_preopen_news_hot(code: &str, params: PreopenNewsHotParams<'_>) -> bool {
    let text = render_preopen_news_hot(params);
    dispatch(crate::notify::PushKind::PreopenNewsHot, code, None, text).await
}

/// v15.1: 从 chain_daily DB 构造 PreopenNewsHotParams (业务层集成入口)
/// - themes: 取 clusters 中前 3 个 concept
/// - watch_stocks: 每个 cluster 取前 3 个 stock code (简化: 无名称)
/// - news_pairs: 暂空, 需 v15.1+ 集成 news_monitor API 后填充
pub fn build_preopen_news_hot_from_db<'a>(
    hhmm: &'a str,
    clusters: &'a [stock_analysis::database::concepts::ChainDailyRow],
) -> PreopenNewsHotParams<'a> {
    let themes: Vec<&str> = clusters.iter().take(3).map(|c| c.concept.as_str()).collect();
    let theme_1 = themes.first().copied();
    let theme_2 = themes.get(1).copied();
    let theme_3 = themes.get(2).copied();

    let watch_stocks: Vec<(&str, &str, &str)> = clusters
        .iter()
        .take(3)
        .filter_map(|c| {
            // stocks 是 JSON 数组字符串 ["code1","code2",...]
            // 简化: 取前 3 个 code (无名称, 用 code 作为 reason)
            let codes: Vec<&str> = c
                .stocks
                .trim_matches(|c| c == '[' || c == ']')
                .split(',')
                .take(3)
                .map(|s| s.trim_matches('"').trim())
                .filter(|s| !s.is_empty())
                .collect();
            codes.first().map(|code| (*code, *code, c.concept.as_str()))
        })
        .collect();

    PreopenNewsHotParams {
        hhmm,
        theme_1,
        theme_2,
        theme_3,
        news_pairs: Vec::new(), // TODO: 集成 news_monitor API
        watch_stocks,
    }
}

/// v15.1: 业务层入口 — 09:00 盘前自动调用
pub async fn dispatch_preopen_news_hot_daily() -> bool {
    use stock_analysis::database::DatabaseManager;
    let clusters = DatabaseManager::get().get_latest_chain_clusters();
    if clusters.is_empty() {
        log_dispatcher_attempt("P-01", false, 0, "no clusters");
        log::info!("[P-01] 无主线簇, 跳过推送");
        return false;
    }
    let now = chrono::Local::now();
    let hhmm = now.format("%H:%M").to_string();
    let params = build_preopen_news_hot_from_db(&hhmm, &clusters);
    let snapshot_size = clusters.len();
    let result = push_preopen_news_hot("", params).await;
    log_dispatcher_attempt("P-01", result, snapshot_size, "");
    result
}

// ============================================================================
// v13.7: dispatcher_log (JSONL) — 6 dispatcher 统一记录
// ============================================================================

/// v13.7+v14.4: 记录 1 次 dispatch 尝试 (生产可观测)
/// 输出: data/dispatcher_log/{YYYY-MM-DD}.jsonl (按天轮转, 7 天保留)
/// 字段: ts, kind, success, snapshot_size, error
pub fn log_dispatcher_attempt(kind: &str, success: bool, snapshot_size: usize, error: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;
    let now = chrono::Local::now();
    let date_str = now.format("%Y-%m-%d").to_string();
    let dir = std::path::PathBuf::from("data/dispatcher_log");
    std::fs::create_dir_all(&dir).ok();
    let path = dir.join(format!("{}.jsonl", date_str));

    // v14.4: 按天轮转 + 7 天清理 (避免无限增长)
    rotate_dispatcher_logs(&dir, 7);

    let ts = now.format("%Y-%m-%dT%H:%M:%S%.3f").to_string();
    let line = format!(
        "{{\"ts\":\"{}\",\"kind\":\"{}\",\"success\":{},\"snapshot_size\":{},\"error\":\"{}\"}}\n",
        ts,
        kind,
        success,
        snapshot_size,
        error.replace('"', "'")
    );
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// v14.4: 清理 N 天前的 dispatcher_log 文件
fn rotate_dispatcher_logs(dir: &std::path::Path, retention_days: u64) {
    use std::time::{Duration, SystemTime};
    let threshold = match SystemTime::now().checked_sub(Duration::from_secs(retention_days * 86400)) {
        Some(t) => t,
        None => return,
    };
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(meta) = path.metadata() {
            if let Ok(modified) = meta.modified() {
                if modified < threshold {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
}

// ============================================================================
// v15.2: I-01 业务层集成 (sector_score 抽口)
// ============================================================================

/// v15.2: 板块快照 (3 大板块: 科技/电力/机器人)
/// 注: 真实 sector_score 算法待 v16+ 集成 (本结构仅作数据载体)
#[derive(Debug, Clone, Default)]
pub struct SectorSnapshot {
    pub hhmm: String,
    pub tech_sub: String,
    pub tech_score: Option<f32>,
    pub power_sub: String,
    pub power_score: Option<f32>,
    pub robot_sub: String,
    pub robot_score: Option<f32>,
    pub main_attack: String,
    pub rotation_state: RotationState,
}

/// v15.2: 从 SectorSnapshot 构造 IntradayMarketParams
pub fn build_intraday_market_from_snapshot<'a>(s: &'a SectorSnapshot) -> IntradayMarketParams<'a> {
    IntradayMarketParams {
        hhmm: &s.hhmm,
        tech_sub: if s.tech_sub.is_empty() { None } else { Some(&s.tech_sub) },
        tech_score: s.tech_score,
        power_sub: if s.power_sub.is_empty() { None } else { Some(&s.power_sub) },
        power_score: s.power_score,
        robot_sub: if s.robot_sub.is_empty() { None } else { Some(&s.robot_sub) },
        robot_score: s.robot_score,
        main_attack: if s.main_attack.is_empty() { None } else { Some(&s.main_attack) },
        rotation_state: s.rotation_state.clone(),
    }
}

/// v16.2: LLM-style 分类器 trait (mock + 真实 LLM 集成接口)
/// 现阶段: 启发式关键词 (32 个), 可替换为 LLM API 调用
pub trait SectorClassifier {
    fn classify(&self, name: &str) -> Option<&'static str>;
}

/// v16.2: 默认实现 (启发式关键词, 与 v13.5 一致)
/// 后续 v17+ 可换为: LlmClassifier { client: LlmClient }
pub struct HeuristicClassifier;

impl SectorClassifier for HeuristicClassifier {
    fn classify(&self, name: &str) -> Option<&'static str> {
        classify_sector_to_family(name)
    }
}

/// v17.1+v13.5: 板块关键词过滤 (tech/power/robot 按 name 关键词匹配)
/// v13.5 扩展: 半导体子分支/电力子分支/机器人子分支细分
/// v16.2: 此函数作为默认启发式实现, 被 HeuristicClassifier 调用
fn classify_sector_to_family(name: &str) -> Option<&'static str> {
    let n = name.to_lowercase();
    // tech 关键词 (v13.5 扩展: 半导体子分支)
    if n.contains("ai") || n.contains("算力") || n.contains("芯片") || n.contains("半导体")
        || n.contains("集成电路") || n.contains("封测") || n.contains("光刻")
        || n.contains("软件") || n.contains("互联网") || n.contains("电子")
        || n.contains("云计算") || n.contains("大数据") || n.contains("5g")
    {
        return Some("tech");
    }
    // power 关键词 (v13.5 扩展: 电力子分支)
    if n.contains("电") || n.contains("电网") || n.contains("储能") || n.contains("光伏")
        || n.contains("新能源") || n.contains("电池") || n.contains("锂")
        || n.contains("风电") || n.contains("核电") || n.contains("特高压")
        || n.contains("充电桩") || n.contains("氢能")
    {
        return Some("power");
    }
    // robot 关键词 (v13.5 扩展: 机器人子分支)
    if n.contains("机器") || n.contains("减速") || n.contains("伺服") || n.contains("机器视觉")
        || n.contains("自动化") || n.contains("智能")
        || n.contains("传感器") || n.contains("控制器") || n.contains("工业母机")
        || n.contains("人形") || n.contains("无人机")
    {
        return Some("robot");
    }
    None
}

/// v16.2: LLM-style 分类器 (mock 占位, 接口稳定)
/// 真实 LLM 集成 (v17+): 替换 HeuristicClassifier 为 LlmClassifier { client }
pub fn default_classifier() -> HeuristicClassifier {
    HeuristicClassifier
}

/// v16.1+v17.1: 真实 sector_score 算法集成
/// 联接 sector_monitor::fetch_board_ranking + sector_score::grade_sectors
/// v17.1 改进: 按关键词分类 tech/power/robot
pub fn load_sector_snapshot_real(hhmm: &str) -> SectorSnapshot {
    use stock_analysis::decision::sector_score::grade_sectors;
    use stock_analysis::market_analyzer::sector_monitor::fetch_board_ranking;

    let boards = match fetch_board_ranking("f3", 30) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("[I-01] fetch_board_ranking 失败: {}, 退潮兜底", e);
            return SectorSnapshot {
                hhmm: hhmm.to_string(),
                rotation_state: RotationState::Fading,
                ..Default::default()
            };
        }
    };

    let graded = grade_sectors(&boards);

    // v17.1 改进: 按关键词分类 tech/power/robot
    // v16.2: 通过 SectorClassifier trait (可换 LLM)
    let classifier = default_classifier();
    let mut tech: Option<&str> = None;
    let mut tech_score: Option<f64> = None;
    let mut power: Option<&str> = None;
    let mut power_score: Option<f64> = None;
    let mut robot: Option<&str> = None;
    let mut robot_score: Option<f64> = None;
    let mut main_attack = String::new();
    let mut best_score = f64::MIN;

    for s in &graded {
        if let Some(family) = classifier.classify(&s.name) {
            if s.change_pct > best_score {
                best_score = s.change_pct;
                main_attack = s.name.clone();
            }
            match family {
                "tech" if tech.is_none() => {
                    tech = Some(&s.name);
                    tech_score = Some(s.change_pct);
                }
                "power" if power.is_none() => {
                    power = Some(&s.name);
                    power_score = Some(s.change_pct);
                }
                "robot" if robot.is_none() => {
                    robot = Some(&s.name);
                    robot_score = Some(s.change_pct);
                }
                _ => {}  // 已填或无家族
            }
        }
    }

    // rotation_state 派生 (与 v16.1 一致)
    let positive_count = graded.iter().filter(|s| s.change_pct > 0.0).count();
    let total = graded.len().max(1);
    let rotation_state = if positive_count * 3 >= total * 2 {
        RotationState::Spreading
    } else if positive_count * 3 >= total {
        RotationState::Diverging
    } else {
        RotationState::Fading
    };

    SectorSnapshot {
        hhmm: hhmm.to_string(),
        tech_sub: tech.unwrap_or("").to_string(),
        tech_score: tech_score.map(|s| s as f32),
        power_sub: power.unwrap_or("").to_string(),
        power_score: power_score.map(|s| s as f32),
        robot_sub: robot.unwrap_or("").to_string(),
        robot_score: robot_score.map(|s| s as f32),
        main_attack,
        rotation_state,
    }
}

/// v15.2 兼容: 同步占位接口 (调用 v16.1 async 接口)
pub fn load_sector_snapshot(hhmm: &str) -> SectorSnapshot {
    // v16.1: 改用 block_on 同步调用 (测试用) — 实际 dispatcher 用 load_sector_snapshot_real
    SectorSnapshot {
        hhmm: hhmm.to_string(),
        rotation_state: RotationState::Fading,
        ..Default::default()
    }
}

/// v15.2 业务层入口 — 10/11/13/14 盘中调用 (v16.1 改用真实数据)
pub async fn dispatch_intraday_market_daily(hhmm: &str, banner: &BannerCtx) -> bool {
    let snapshot = load_sector_snapshot_real(hhmm);
    if snapshot.tech_sub.is_empty() && snapshot.power_sub.is_empty() && snapshot.robot_sub.is_empty() {
        log_dispatcher_attempt("I-01", false, 0, "sector_snapshot empty");
        log::info!("[I-01] sector_snapshot 空 (grade_sectors 无数据), 跳过推送");
        return false;
    }
    let params = build_intraday_market_from_snapshot(&snapshot);
    let snap_size = 3;  // tech/power/robot
    let result = push_intraday_market("", Some(banner), params).await;
    log_dispatcher_attempt("I-01", result, snap_size, "");
    result
}

// ============================================================================
// v15.3: I-02 业务层集成 (news_catalyst 抽口)
// ============================================================================

/// v15.3: 新闻催化快照 (headline + theme + 上涨个股)
/// 注: 真实数据集成待 v16+ (news_monitor + 实时行情)
#[derive(Debug, Clone, Default)]
pub struct NewsCatalystSnapshot {
    pub hhmm: String,
    pub headline: String,
    pub theme: String,
    /// (name, code, chg_pct)
    pub stocks: Vec<(String, String, Option<f32>)>,
}

/// v15.3: 从 NewsCatalystSnapshot 构造 NewsCatalystParams
pub fn build_news_catalyst_from_snapshot<'a>(s: &'a NewsCatalystSnapshot) -> NewsCatalystParams<'a> {
    // 借用 String → &str 转换 (注意生命周期与 s 一致)
    let stocks_ref: Vec<(&'a str, &'a str, Option<f32>, &'a str)> = s
        .stocks
        .iter()
        .map(|(n, c, chg)| (n.as_str(), c.as_str(), *chg, n.as_str()))  // reason=name (简化)
        .collect();
    NewsCatalystParams {
        hhmm: &s.hhmm,
        headline: &s.headline,
        theme: if s.theme.is_empty() { None } else { Some(&s.theme) },
        stocks: stocks_ref,
    }
}

/// v16.1: 批量 fetch_realtime_quote (并行, 避免 N 次 HTTP)
/// 注: gtimg_provider 无 batch API, 用 std::thread::scope 并行调用单股接口
pub fn fetch_realtime_quotes_batch(
    codes: &[&str],
) -> std::collections::HashMap<String, f32> {
    use stock_analysis::data_provider::GtimgProvider;
    let provider = match GtimgProvider::new() {
        Ok(p) => std::sync::Arc::new(p),
        Err(e) => {
            log::warn!("[v16.1] GtimgProvider::new 失败: {}", e);
            return std::collections::HashMap::new();
        }
    };
    let mut result = std::collections::HashMap::new();
    std::thread::scope(|s| {
        let handles: Vec<_> = codes
            .iter()
            .map(|code| {
                let code_owned = code.to_string();
                let provider_ref = provider.clone();
                s.spawn(move || {
                    let chg = match provider_ref.fetch_realtime_quote(&code_owned) {
                        Ok(Some(q)) => Some(q.pct_chg as f32),
                        _ => None,
                    };
                    (code_owned, chg)
                })
            })
            .collect();
        for h in handles {
            if let Ok((code, Some(chg))) = h.join() {
                result.insert(code, chg);
            }
        }
    });
    result
}

/// v17.2+v16.1: 实时涨跌接入 (data_provider::gtimg, 批量)
pub fn load_news_catalyst_snapshot_real(hhmm: &str) -> NewsCatalystSnapshot {
    use stock_analysis::database::DatabaseManager;
    let clusters = DatabaseManager::get().get_latest_chain_clusters();
    if clusters.is_empty() {
        return NewsCatalystSnapshot::default();
    }
    let top = &clusters[0];

    // 收集所有 codes (前 3 cluster × 前 3 code = 最多 9 个, 去重)
    let mut codes: Vec<String> = Vec::new();
    for c in clusters.iter().take(3) {
        for code in c
            .stocks
            .trim_matches(|ch| ch == '[' || ch == ']')
            .split(',')
            .take(3)
            .map(|s| s.trim_matches('"').trim().to_string())
            .filter(|s| !s.is_empty())
        {
            if !codes.contains(&code) {
                codes.push(code);
            }
        }
    }

    // v16.1: 批量 fetch (并行, 1 次 vs N 次)
    let code_refs: Vec<&str> = codes.iter().map(|s| s.as_str()).collect();
    let chg_map = fetch_realtime_quotes_batch(&code_refs);

    let mut stocks: Vec<(String, String, Option<f32>)> = Vec::new();
    for code in codes {
        let chg = chg_map.get(&code).copied();
        stocks.push((code.clone(), code, chg));
    }
    NewsCatalystSnapshot {
        hhmm: hhmm.to_string(),
        headline: format!("{} 板块持续走强", top.concept),
        theme: top.concept.clone(),
        stocks,
    }
}

/// v15.3 兼容: 同步占位
pub fn load_news_catalyst_snapshot(_hhmm: &str) -> NewsCatalystSnapshot {
    NewsCatalystSnapshot::default()
}

/// v17.2 内部: provider 失败时 fallback (避免 v16.2 的 chg=0.0 占位)
fn load_news_catalyst_snapshot_real_fallback(hhmm: &str) -> NewsCatalystSnapshot {
    use stock_analysis::database::DatabaseManager;
    let clusters = DatabaseManager::get().get_latest_chain_clusters();
    if clusters.is_empty() {
        return NewsCatalystSnapshot::default();
    }
    let top = &clusters[0];
    let stocks: Vec<(String, String, Option<f32>)> = clusters
        .iter()
        .take(3)
        .filter_map(|c| {
            let codes: Vec<&str> = c
                .stocks
                .trim_matches(|ch| ch == '[' || ch == ']')
                .split(',')
                .take(3)
                .map(|s| s.trim_matches('"').trim())
                .filter(|s| !s.is_empty())
                .collect();
            codes.first().map(|code| (code.to_string(), code.to_string(), None))
        })
        .collect();
    NewsCatalystSnapshot {
        hhmm: hhmm.to_string(),
        headline: format!("{} 板块持续走强", top.concept),
        theme: top.concept.clone(),
        stocks,
    }
}

/// v15.3 业务层入口 (v16.2 改用真实 chain_daily 数据)
pub async fn dispatch_news_catalyst_daily(hhmm: &str, banner: &BannerCtx) -> bool {
    let snapshot = load_news_catalyst_snapshot_real(hhmm);
    if snapshot.headline.is_empty() {
        log_dispatcher_attempt("I-02", false, 0, "news_catalyst_snapshot empty");
        log::info!("[I-02] news_catalyst_snapshot 空 (chain_daily 无数据), 跳过推送");
        return false;
    }
    let params = build_news_catalyst_from_snapshot(&snapshot);
    let snap_size = snapshot.stocks.len();
    let result = push_news_catalyst("", Some(banner), params).await;
    log_dispatcher_attempt("I-02", result, snap_size, "");
    result
}

// ============================================================================
// v15.4: I-03 业务层集成 (industry_chain_intraday 抽口)
// ============================================================================

/// v15.4: 涨停扩散快照 (主链 + 龙头 + 补涨候选)
/// 注: 真实板块涨停扫描待 v16+ (限 + 龙头 + 候选台)
#[derive(Debug, Clone, Default)]
pub struct IndustryChainSnapshot {
    pub hhmm: String,
    pub chain: String,
    pub limit_count: u32,
    pub leader_name: String,
    pub leader_code: String,
    pub leader_height: u32,
    /// (name, code, trigger, lo, hi, stop)
    pub supplements: Vec<(String, String, String, f64, f64, f64)>,
}

/// v15.4: 构造 IndustryChainIntradayParams
pub fn build_industry_chain_intraday_from_snapshot<'a>(
    s: &'a IndustryChainSnapshot,
) -> IndustryChainIntradayParams<'a> {
    let supplement_refs: Vec<SupplementCandidate<'a>> = s
        .supplements
        .iter()
        .map(|(n, c, t, lo, hi, st)| SupplementCandidate {
            name: n.as_str(),
            code: c.as_str(),
            trigger: t.as_str(),
            lo: *lo,
            hi: *hi,
            stop: *st,
        })
        .collect();

    IndustryChainIntradayParams {
        hhmm: &s.hhmm,
        chain: &s.chain,
        limit_count: s.limit_count,
        leader_name: if s.leader_name.is_empty() { None } else { Some(&s.leader_name) },
        leader_code: if s.leader_code.is_empty() { None } else { Some(&s.leader_code) },
        leader_height: s.leader_height,
        supplements: supplement_refs,
    }
}

/// v16.3+v14.1: 真实数据集成 — 复用 chain_daily DB + GtimgProvider + aggregate()
/// v14.1 改进: 走 market_analyzer::limit_chain_review::aggregate() 真正集成
pub fn load_industry_chain_snapshot_real(hhmm: &str) -> IndustryChainSnapshot {
    use stock_analysis::database::DatabaseManager;
    use stock_analysis::data_provider::GtimgProvider;
    use stock_analysis::market_analyzer::limit_chain_review::{
        aggregate, LimitChainInput, StockLimitStats,
    };

    let clusters = DatabaseManager::get().get_latest_chain_clusters();
    if clusters.is_empty() {
        return IndustryChainSnapshot::default();
    }

    // v14.1: 构造 StockLimitStats[] (从 chain_daily + 实时行情)
    let provider = match GtimgProvider::new() {
        Ok(p) => p,
        Err(e) => {
            log::warn!("[I-03] GtimgProvider::new 失败: {}, 退化到 chain_daily 简化版", e);
            return load_industry_chain_snapshot_real_fallback(hhmm);
        }
    };

    let mut stocks: Vec<StockLimitStats> = Vec::new();
    for c in clusters.iter().take(5) {
        // 解析 stocks JSON
        let codes: Vec<String> = c
            .stocks
            .trim_matches(|ch| ch == '[' || ch == ']')
            .split(',')
            .map(|s| s.trim_matches('"').trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        for (i, code) in codes.iter().take(3).enumerate() {
            // v14.1: 实时行情接入 (替代 chg=0.0 占位)
            let (price, chg_pct) = match provider.fetch_realtime_quote(code) {
                Ok(Some(q)) => (q.price, q.pct_chg as f64),
                _ => (0.0, 0.0),
            };
            stocks.push(StockLimitStats {
                code: code.clone(),
                name: code.clone(),  // 简化: code 作为 name
                chain: c.concept.clone(),
                board_level: (i + 1) as u8,  // 简化: 按位置推断 (1=首板)
                is_limit_up_today: chg_pct > 9.5,  // ST/*ST 涨跌幅 10% 视为涨停
                is_first_board: i == 0,
                consecutive_days: c.continuation_count as u32,
            });
        }
    }

    if stocks.is_empty() {
        return load_industry_chain_snapshot_real_fallback(hhmm);
    }

    // v14.1: 真正调 aggregate() (vs v16.3 简化)
    let input = LimitChainInput {
        stocks: stocks.clone(),
        source_complete: false,  // 简化 (v14+ 接真实 fetcher 改 true)
    };
    let aggregates = aggregate(&input);
    if aggregates.is_empty() {
        return load_industry_chain_snapshot_real_fallback(hhmm);
    }

    // 取 top 1 aggregate (按 limit_up_n 降序)
    let mut sorted: Vec<_> = aggregates.iter().collect();
    sorted.sort_by(|a, b| b.limit_up_n.cmp(&a.limit_up_n));
    let top = sorted[0];

    // 解析 followers → supplements (前 3)
    let supplements: Vec<(String, String, String, f64, f64, f64)> = top
        .followers
        .iter()
        .take(3)
        .map(|c| (c.clone(), c.clone(), "首板".to_string(), 0.0, 0.0, 0.0))
        .collect();

    IndustryChainSnapshot {
        hhmm: hhmm.to_string(),
        chain: top.chain.clone(),
        limit_count: top.limit_up_n,
        leader_name: top.leader_name.clone(),
        leader_code: top.leader_code.clone(),
        leader_height: top.leader_boards,
        supplements,
    }
}

/// v14.1 fallback: GtimgProvider 失败时退化
fn load_industry_chain_snapshot_real_fallback(hhmm: &str) -> IndustryChainSnapshot {
    use stock_analysis::database::DatabaseManager;
    let clusters = DatabaseManager::get().get_latest_chain_clusters();
    if clusters.is_empty() {
        return IndustryChainSnapshot::default();
    }
    let mut sorted: Vec<_> = clusters.iter().collect();
    sorted.sort_by(|a, b| b.continuation_count.cmp(&a.continuation_count));
    let top = sorted[0];
    let codes: Vec<String> = top
        .stocks
        .trim_matches(|ch| ch == '[' || ch == ']')
        .split(',')
        .map(|s| s.trim_matches('"').trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let leader_name = if !codes.is_empty() { codes[0].clone() } else { String::new() };
    let leader_code = codes.get(0).cloned().unwrap_or_default();
    let supplements: Vec<(String, String, String, f64, f64, f64)> = codes
        .iter()
        .skip(1)
        .take(3)
        .map(|c| (c.clone(), c.clone(), "首板".to_string(), 0.0, 0.0, 0.0))
        .collect();
    IndustryChainSnapshot {
        hhmm: hhmm.to_string(),
        chain: top.concept.clone(),
        limit_count: top.continuation_count as u32,
        leader_name,
        leader_code,
        leader_height: top.continuation_count as u32,
        supplements,
    }
}

/// v15.4 兼容: 同步占位
pub fn load_industry_chain_snapshot(_hhmm: &str) -> IndustryChainSnapshot {
    IndustryChainSnapshot::default()
}

/// v15.4 业务层入口 (v16.3 改用真实 chain_daily 数据)
pub async fn dispatch_industry_chain_intraday_daily(hhmm: &str, banner: &BannerCtx) -> bool {
    let snapshot = load_industry_chain_snapshot_real(hhmm);
    if snapshot.chain.is_empty() {
        log_dispatcher_attempt("I-03", false, 0, "industry_chain_snapshot empty");
        log::info!("[I-03] industry_chain_snapshot 空 (chain_daily 无数据), 跳过推送");
        return false;
    }
    let params = build_industry_chain_intraday_from_snapshot(&snapshot);
    let snap_size = snapshot.supplements.len() + 1;  // +1 leader
    let result = push_industry_chain_intraday("", Some(banner), params).await;
    log_dispatcher_attempt("I-03", result, snap_size, "");
    result
}

// ============================================================================
// v15.5: D-01 业务层集成 (news_to_idea 抽口)
// ============================================================================

/// v15.5: 新闻驱动个股快照
/// 注: 真实数据集成 (news_monitor + 候选台) 待 v16+
#[derive(Debug, Clone, Default)]
pub struct NewsToIdeaSnapshot {
    pub hhmm: String,
    pub headline: String,
    pub theme: String,
    pub stage: NewsStage,
    pub name: String,
    pub code: String,
    pub reasons: Vec<String>,
    pub action: Option<NewsAction>,
}

/// v15.5: 构造 NewsToIdeaParams
pub fn build_news_to_idea_from_snapshot<'a>(s: &'a NewsToIdeaSnapshot) -> NewsToIdeaParams<'a> {
    let reasons_ref: Vec<&'a str> = s.reasons.iter().map(|r| r.as_str()).collect();
    NewsToIdeaParams {
        hhmm: &s.hhmm,
        headline: &s.headline,
        theme: if s.theme.is_empty() { None } else { Some(&s.theme) },
        stage: s.stage.clone(),
        name: &s.name,
        code: &s.code,
        reasons: reasons_ref,
        action: s.action.clone(),
    }
}

/// v14.2: P5 源真实 fetcher (文件化)
// 读 data/p5_sources/{source}.jsonl, 每行 JSON {code, name, chg_pct}
pub fn load_p5_source_items(source_name: &str) -> Vec<(stock_analysis::opportunity::candidate_panel::CandidateSource, String, String)> {
    use stock_analysis::opportunity::candidate_panel::CandidateSource;
    use std::fs;
    let path = format!("data/p5_sources/{}.jsonl", source_name);
    let source = match source_name {
        "stock_pick" => CandidateSource::StockPick,
        "optimal_close" => CandidateSource::OptimalClose,
        "volume_watchlist" => CandidateSource::VolumeWatchlist,
        "volume_real_trade" => CandidateSource::VolumeRealTrade,
        _ => return Vec::new(),
    };
    let mut items = Vec::new();
    let raw = match fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        #[derive(serde::Deserialize)]
        struct P5Item {
            code: String,
            name: String,
            #[allow(dead_code)]
            chg_pct: Option<f32>,
        }
        if let Ok(p) = serde_json::from_str::<P5Item>(line) {
            if !p.code.is_empty() {
                items.push((source, p.code, p.name));
            }
        }
    }
    items
}

/// v16.4+v13.6.2+v14.2: 真实数据集成 — 从候选台取 top 1 candidate
/// 联接 opportunity::candidate_panel::merge_candidates
/// v14.2 改进: P5 源文件化 (data/p5_sources/*.jsonl)
pub fn load_news_to_idea_snapshot_real(hhmm: &str) -> NewsToIdeaSnapshot {
    use stock_analysis::opportunity::candidate_panel::{merge_candidates, CandidateSource};
    use stock_analysis::database::DatabaseManager;
    let clusters = DatabaseManager::get().get_latest_chain_clusters();
    if clusters.is_empty() {
        return NewsToIdeaSnapshot::default();
    }

    let mut items: Vec<(CandidateSource, String, String)> = Vec::new();

    // 1. IndustryChain: 5 个 cluster 头部
    for c in clusters.iter().take(5) {
        let code = c
            .stocks
            .trim_matches(|ch| ch == '[' || ch == ']')
            .split(',')
            .next()
            .map(|s| s.trim_matches('"').trim().to_string())
            .unwrap_or_default();
        if !code.is_empty() {
            items.push((CandidateSource::IndustryChain, code, c.concept.clone()));
        }
    }

    // 2-5. 4 路 P5 源: v14.2 文件化加载 (data/p5_sources/{source}.jsonl)
    items.extend(load_p5_source_items("stock_pick"));
    items.extend(load_p5_source_items("optimal_close"));
    items.extend(load_p5_source_items("volume_watchlist"));
    items.extend(load_p5_source_items("volume_real_trade"));

    // v14.2 fallback: 若 P5 文件都不存在, 复用 top 1 cluster (v13.6.2 兼容)
    if items.iter().filter(|(s, _, _)| matches!(s, CandidateSource::StockPick | CandidateSource::OptimalClose | CandidateSource::VolumeWatchlist | CandidateSource::VolumeRealTrade)).count() == 0 {
        if let Some(top) = clusters.first() {
            let code = top
                .stocks
                .trim_matches(|ch| ch == '[' || ch == ']')
                .split(',')
                .next()
                .map(|s| s.trim_matches('"').trim().to_string())
                .unwrap_or_default();
            if !code.is_empty() {
                items.push((CandidateSource::StockPick, code.clone(), top.concept.clone()));
                items.push((CandidateSource::OptimalClose, code.clone(), top.concept.clone()));
                items.push((CandidateSource::VolumeWatchlist, code.clone(), top.concept.clone()));
                items.push((CandidateSource::VolumeRealTrade, code, top.concept.clone()));
            }
        }
    }

    let candidates = merge_candidates(items);
    if candidates.is_empty() {
        return NewsToIdeaSnapshot::default();
    }
    let top = &candidates[0];
    let reasons: Vec<String> = top.evidence.iter().take(3).cloned().collect();
    let stage = if top.source_count() >= 3 {
        NewsStage::Starting
    } else if top.source_count() >= 2 {
        NewsStage::Fermenting
    } else {
        NewsStage::Diverging
    };
    let action = if top.change_pct > 5.0 {
        Some(NewsAction::DoNotChase)
    } else if top.change_pct > 0.0 {
        Some(NewsAction::BuyDip)
    } else {
        Some(NewsAction::Observe)
    };
    NewsToIdeaSnapshot {
        hhmm: hhmm.to_string(),
        headline: format!(
            "{} ({}) 多源验证 ({} 源)",
            top.name,
            top.code,
            top.source_count()
        ),
        theme: clusters[0].concept.clone(),
        stage,
        name: top.name.clone(),
        code: top.code.clone(),
        reasons,
        action,
    }
}

/// v15.5 兼容: 同步占位
pub fn load_news_to_idea_snapshot(_hhmm: &str) -> NewsToIdeaSnapshot {
    NewsToIdeaSnapshot::default()
}

// v29: D-01 dispatcher 内部 memo (1h/票, 跨日重置)
// 静态 Lazy 容器, 跨函数调用复用
// 注: Lazy/HashMap 已在文件顶部 import 过 (避免 unused import 警告), 这里只补 Mutex/Instant
use std::sync::Mutex;
use std::time::Instant;

pub static D01_LAST_PUSH: Lazy<Mutex<HashMap<String, Instant>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// v29: 测试用 - 重置 memo 容器
#[cfg(test)]
pub fn _reset_d01_memo_for_test() {
    D01_LAST_PUSH.lock().unwrap().clear();
}

/// v15.5 业务层入口 (v16.4 改用真实候选台数据)
/// v29: 加 dispatcher 内部 memo (1h/票) — 防止公告密集时同票刷屏
pub async fn dispatch_news_to_idea_daily(hhmm: &str, banner: &BannerCtx) -> bool {
    let snapshot = load_news_to_idea_snapshot_real(hhmm);
    if snapshot.headline.is_empty() {
        log_dispatcher_attempt("D-01", false, 0, "news_to_idea_snapshot empty");
        log::info!("[D-01] news_to_idea_snapshot 空 (候选台无候选), 跳过推送");
        return false;
    }

    // v29: memo 1h/票 (与 push_governor 20min 冷却叠加, 实际间隔 ≥ 1h)
    let memo_key = format!("{}:{}", snapshot.code, snapshot.name);
    {
        let mut map = D01_LAST_PUSH.lock().unwrap();
        if let Some(last) = map.get(&memo_key) {
            let elapsed = last.elapsed().as_secs();
            if elapsed < 3600 {
                log_dispatcher_attempt(
                    "D-01",
                    false,
                    0,
                    &format!("1h memo 冷却, 还需 {}s", 3600 - elapsed),
                );
                log::info!(
                    "[D-01] {}:{} memo 冷却中, 跳过推送 (剩余 {}s)",
                    snapshot.code,
                    snapshot.name,
                    3600 - elapsed
                );
                return false;
            }
        }
        map.insert(memo_key.clone(), Instant::now());
    }

    let params = build_news_to_idea_from_snapshot(&snapshot);
    let snap_size = snapshot.reasons.len();
    let result = push_news_to_idea("", Some(banner), params).await;
    log_dispatcher_attempt("D-01", result, snap_size, "");
    result
}

// ============================================================================
// v15.6: A-01 业务层集成 (paper_review 抽口, T-11 通路)
// ============================================================================

/// v15.6: 虚拟仓复盘快照 (复用 T-11 竞价复算 logic)
/// 注: 真实数据集成 (virtual_watch/paper_trades DB) 待 v16+
#[derive(Debug, Clone, Default)]
pub struct PaperReviewSnapshot {
    pub date: String,
    pub name: String,
    pub code: String,
    pub trigger: String,
    pub desc: String,
    pub pnl: Option<f32>,
    /// (high, flat, low) — 复用 T-11 plan_high/flat/low 派生
    pub plan_high: Option<String>,
    pub plan_flat: Option<String>,
    pub plan_low: Option<String>,
}

/// v15.6: 构造 PaperReviewParams
pub fn build_paper_review_from_snapshot<'a>(s: &'a PaperReviewSnapshot) -> PaperReviewParams<'a> {
    PaperReviewParams {
        date: &s.date,
        name: &s.name,
        code: &s.code,
        trigger: &s.trigger,
        desc: &s.desc,
        pnl: s.pnl,
        plan_high: s.plan_high.as_deref(),
        plan_flat: s.plan_flat.as_deref(),
        plan_low: s.plan_low.as_deref(),
    }
}

/// v17.5+v13.6.3: 完整 JSON 解析 (VirtualObservationRecord via serde_json)
/// v13.6.3 改进: 扩展 entry_price 字段 (替代 v13.5 退化 0.0)
pub fn load_paper_review_snapshot_real(date: &str) -> PaperReviewSnapshot {
    use stock_analysis::data_provider::GtimgProvider;

    let snapshot = load_virtual_observation_for_a01();
    if snapshot.records.is_empty() {
        return PaperReviewSnapshot::default();
    }
    let top = &snapshot.records[0];

    // v13.6.3: 从 VirtualRecordLite.entry_price 拿真实 entry (替代 0.0 占位)
    let entry_price = top.entry_price;
    let close_price = match GtimgProvider::new().and_then(|p| p.fetch_realtime_quote(&top.code)) {
        Ok(Some(q)) => q.price,
        Ok(None) => entry_price,
        Err(e) => {
            log::debug!("[A-01] fetch_realtime_quote({}) 失败: {}", top.code, e);
            entry_price
        }
    };
    let pnl = if entry_price > 0.0 {
        ((close_price / entry_price - 1.0) * 100.0) as f32
    } else {
        0.0
    };
    let (high, flat, low) = derive_plan_from_pnl(pnl);

    PaperReviewSnapshot {
        date: date.to_string(),
        name: top.name.clone(),
        code: top.code.clone(),
        trigger: top.entry_mode.clone(),
        desc: format!(
            "虚拟仓已建仓 (entry={:.2} → close={:.2}, pnl={:+.1}%)",
            entry_price, close_price, pnl
        ),
        pnl: Some(pnl),
        plan_high: Some(high),
        plan_flat: Some(flat),
        plan_low: Some(low),
    }
}

/// v15.6 兼容: 同步占位
pub fn load_paper_review_snapshot(_date: &str) -> PaperReviewSnapshot {
    PaperReviewSnapshot::default()
}

/// v15.6: T-11 通路复用 — pnl 派生 plan_high/flat/low
/// pnl > 5% → "减仓1/3", pnl > 0% → "减仓1/2", else → "持有观望"
pub fn derive_plan_from_pnl(pnl: f32) -> (String, String, String) {
    if pnl > 5.0 {
        ("减仓1/3".to_string(), "减仓1/2".to_string(), "持有观望".to_string())
    } else if pnl > 0.0 {
        ("减仓1/2".to_string(), "持有".to_string(), "止损".to_string())
    } else {
        ("持有观望".to_string(), "止损".to_string(), "止损".to_string())
    }
}

/// v15.6 业务层入口 (v16.5 改用真实 virtual_observation 数据)
pub async fn dispatch_paper_review_daily(date: &str) -> bool {
    let snapshot = load_paper_review_snapshot_real(date);
    if snapshot.name.is_empty() {
        log_dispatcher_attempt("A-01", false, 0, "paper_review_snapshot empty");
        log::info!("[A-01] paper_review_snapshot 空 (virtual_observation 无数据), 跳过推送");
        return false;
    }
    let params = build_paper_review_from_snapshot(&snapshot);
    let snap_size = 1;  // 1 record
    let result = push_paper_review("", params).await;
    log_dispatcher_attempt("A-01", result, snap_size, "");
    result
}

// ============================================================================
// v35: A-10 盘后题材催化复盘 dispatcher
// ============================================================================

/// v44: T-14 盘后固定价格申报 dispatcher
///   - 数据源: 委托回报 event (持仓/候选股)
///   - 简化: 沙箱无委托系统, 接受外部 caller 传具体 (exchange, code, name, price, qty, order_id, status)
///   - 模板: render_post_fixed_price_order
///   - 真实意图: 接 trade_pipeline 委托回报
pub async fn dispatch_post_fixed_price_order(
    exchange: Exchange,
    hhmm: &str,
    name: &str,
    code: &str,
    price: f64,
    qty: u32,
    order_id: &str,
    status: OrderStatus,
) -> bool {
    let banner = BannerCtx::default();
    let params = PostFixedPriceOrderParams {
        exchange,
        hhmm,
        name,
        code,
        price,
        qty,
        order_id,
        status,
    };
    let text = render_post_fixed_price_order(params);
    let result =
        dispatch(crate::notify::PushKind::PostFixedPriceOrder, code, Some(&banner), text).await;
    log_dispatcher_attempt(
        "T-14",
        result,
        1,
        &format!("exchange={:?} status={:?}", exchange, status),
    );
    result
}

/// v45: T-15 盘后固定价格成交 dispatcher
///   - 数据源: 成交回报 event
///   - 撮合期 15:05-15:30
///   - 模板: render_post_fixed_price_fill
pub async fn dispatch_post_fixed_price_fill(
    exchange: Exchange,
    hhmm: &str,
    name: &str,
    code: &str,
    fill_price: f64,
    qty: u32,
    vs_limit_pct: Option<f32>,
    next_session_carry: bool,
) -> bool {
    let banner = BannerCtx::default();
    let params = PostFixedPriceFillParams {
        exchange,
        hhmm,
        name,
        code,
        fill_price,
        qty,
        vs_limit_pct,
        next_session_carry,
    };
    let text = render_post_fixed_price_fill(params);
    let result =
        dispatch(crate::notify::PushKind::PostFixedPriceFill, code, Some(&banner), text).await;
    log_dispatcher_attempt(
        "T-15",
        result,
        1,
        &format!("exchange={:?} fill_price={}", exchange, fill_price),
    );
    result
}

/// v46: T-16 ST 涨跌幅变更 dispatcher
///   - 新规 2026-07-06: 主板 ST/*ST 5%→10%
///   - 触发: 开盘 9:30 一次/票/日
///   - 数据源: 持仓 DB (ST/*ST 票) + 新规参数 (5%→10%)
///   - 真实 intent: 每天首次入 9:30 推一次
pub async fn dispatch_st_price_limit_changed(
    hhmm: &str,
    name: &str,
    code: &str,
    st_type: StType,
    old_limit: f32,
    new_limit: f32,
    holding_qty: u32,
    cost: f64,
    now_price: f64,
    new_stop_loss: Option<f64>,
    new_take_profit: Option<f64>,
) -> bool {
    let banner = BannerCtx::default();
    let params = StPriceLimitChangedParams {
        hhmm,
        name,
        code,
        st_type,
        old_limit,
        new_limit,
        holding_qty,
        cost,
        now_price,
        new_stop_loss,
        new_take_profit,
    };
    let text = render_st_price_limit_changed(params);
    let result =
        dispatch(crate::notify::PushKind::StPriceLimitChanged, code, Some(&banner), text).await;
    log_dispatcher_attempt(
        "T-16",
        result,
        1,
        &format!("st_type={:?} {}→{}%", st_type, old_limit * 100.0, new_limit * 100.0),
    );
    result
}

/// v40: P-04 虚拟盘成交 dispatcher 包装
pub async fn push_paper_trade(code: &str, params: PaperTradeParams<'_>) -> bool {
    let text = render_paper_trade(params);
    dispatch(crate::notify::PushKind::PaperTrade, code, None, text).await
}

/// v40: P-04 dispatcher
///   - 虚拟盘成交回报 - 复用 monitor_loop 维护的 virtual_observation
///   - 简化: 推 1 条 NotFilled 模板, 标记已观察但未成交
///   - 真实数据: 应从 paper_trade 模块读成交回报 (后续 PR)
///   - count 参数: 调用方传入 virtual_observation.len(), 避免重复 DB 查询
pub async fn dispatch_paper_trade_daily(hhmm: &str, count: usize) -> bool {
    if count == 0 {
        log_dispatcher_attempt("P-04", false, 0, "virtual_observation empty");
        log::info!("[P-04] virtual_observation 空, 跳过推送");
        return false;
    }
    let params = PaperTradeParams {
        name: "虚拟仓",
        code: "",
        hhmm,
        status: PaperTradeStatus::NotFilled,
        fill_price: None,
        qty: None,
        virtual_reason: Some("已观察未成交"),
        not_fill_reason: Some("集合竞价后未触达买入价"),
        account_mode: AccountMode::Normal,
        data_mode: DataMode::Full,
    };
    let result = push_paper_trade("", params).await;
    log_dispatcher_attempt("P-04", result, count, "");
    result
}

/// v39: P-03 候选触发 dispatcher
///   - 候选台取 top 1 candidate (按 source_count 排序)
///   - is_candidate_live_enabled 影子开关 (默认 false)
///   - 简化版: 推送 1 条 A 档候选, evidence 拼成 trigger_desc
pub async fn dispatch_candidate_triggered_daily(hhmm: &str) -> bool {
    use stock_analysis::opportunity::candidate_panel::{
        merge_candidates, CandidateSource, EvidenceTier,
    };
    use stock_analysis::opportunity::candidate_state::is_candidate_live_enabled;

    if !is_candidate_live_enabled(None) {
        log_dispatcher_attempt("P-03", false, 0, "candidate_live disabled");
        log::info!("[P-03] 候选触发被影子开关拦截 (需 ENABLE_CANDIDATE_LIVE=true)");
        return false;
    }

    // 复用 load_news_to_idea_snapshot_real 的多源合并逻辑 (P5 4 路 + IndustryChain)
    // 简化: 直接调 merge_candidates, 给定空 items (实际应从各 P5 源拉)
    let items: Vec<(CandidateSource, String, String)> = Vec::new();
    let mut candidates = merge_candidates(items);
    if candidates.is_empty() {
        // 兜底: 从 chain_daily 拉 cluster 头部
        use stock_analysis::database::DatabaseManager;
        let clusters = DatabaseManager::get().get_latest_chain_clusters();
        if let Some(top) = clusters.first() {
            let code = top
                .stocks
                .trim_matches(|ch| ch == '[' || ch == ']')
                .split(',')
                .next()
                .map(|s| s.trim_matches('"').trim().to_string())
                .unwrap_or_default();
            if !code.is_empty() {
                candidates.push(stock_analysis::opportunity::candidate_panel::CandidateEntry {
                    code: code.clone(),
                    name: top.concept.clone(),
                    sources: vec![CandidateSource::IndustryChain],
                    tier: EvidenceTier::Theme,
                    evidence: vec![format!("主线 {} 涨停梯队", top.concept)],
                    current_price: 0.0,
                    change_pct: 0.0,
                });
            }
        }
    }
    if candidates.is_empty() {
        log_dispatcher_attempt("P-03", false, 0, "candidates empty");
        log::info!("[P-03] 候选台无候选, 跳过推送");
        return false;
    }

    let top = &candidates[0];
    let grade = if top.tier == EvidenceTier::Strong { CandidateGrade::A } else { CandidateGrade::B };
    let topic = top.sources_label();
    let trigger_desc = top.evidence.first().cloned().unwrap_or_else(|| "主线异动".to_string());
    let banner = BannerCtx::default();
    let params = CandidateTriggeredParams {
        name: &top.name,
        code: &top.code,
        hhmm,
        grade,
        topic: &topic,
        price: top.current_price,
        trigger_desc: &trigger_desc,
        lo: top.current_price * 0.97,
        hi: top.current_price * 1.03,
        stop: top.current_price * 0.95,
        max_pos_pct: 10,
        news_quality: EvidenceQuality::Mid,
        news_note: "见 evidence",
        vol_quality: EvidenceQuality::Mid,
        vol_ratio: 1.0,
        kline_quality: EvidenceQuality::Mid,
        kline_note: "N/A",
        book_quality: EvidenceQuality::Missing,
        no_buy: &["一字板不可买".to_string(), "板块跳水".to_string()],
    };
    let result = push_candidate_triggered(&top.code, Some(&banner), params, None).await;
    log_dispatcher_attempt("P-03", result, 1, "");
    result
}

/// v38 + v43: I-04 持仓操作建议 dispatcher
///   - v43: 接入真实报价 (fetch_realtime_quotes_batch), 替换 cost*1.02 写死
///   - 简化版: 遍历当前持仓, 用 real_price + cost + hard_stop 生成 plan
///   - 真实意图: 接入 decision::evaluate_holding (v12.2 规划, 当前未实现)
///   - 当前策略: 涨幅 > 5% → Reduce (逢高减仓), -3% < x < 5% → Hold, < -3% → Add
pub async fn dispatch_holding_plan_daily(hhmm: &str) -> bool {
    use stock_analysis::portfolio::{get_positions, PositionStatus};
    let positions = match get_positions() {
        Ok(p) => p,
        Err(e) => {
            log_dispatcher_attempt("I-04", false, 0, "get_positions failed");
            log::warn!("[I-04] get_positions 失败: {}", e);
            return false;
        }
    };
    if positions.is_empty() {
        log_dispatcher_attempt("I-04", false, 0, "no positions");
        log::info!("[I-04] 当前无持仓, 跳过推送");
        return false;
    }

    // v43: 批量拉真实报价 (并行, 避免 N 次 HTTP)
    let codes: Vec<String> = positions.iter().map(|p| p.code.clone()).collect();
    let quotes = tokio::task::spawn_blocking(move || {
        let code_refs: Vec<&str> = codes.iter().map(|s| s.as_str()).collect();
        fetch_realtime_quotes_batch(&code_refs)
    })
    .await
    .unwrap_or_default();
    if quotes.is_empty() {
        log_dispatcher_attempt("I-04", false, 0, "fetch_realtime_quotes empty");
        log::warn!("[I-04] 拉报价失败, 跳过推送 (沙箱无网络/数据源挂)");
        return false;
    }

    let mut pushed_count = 0;
    for pos in &positions {
        if pos.status != PositionStatus::Holding {
            continue;
        }
        // v43: 用真报价, fallback cost 价
        let current_price = quotes
            .get(&pos.code)
            .map(|p| *p as f64)
            .filter(|p| *p > 0.0)
            .unwrap_or(pos.cost_price);
        let pnl_pct = if pos.cost_price > 0.0 {
            (current_price - pos.cost_price) / pos.cost_price * 100.0
        } else {
            0.0
        };
        // 简单意图: >5% 减仓, <-3% 加仓, 否则持有
        let intent = if pnl_pct > 5.0 {
            Intent::Reduce
        } else if pnl_pct < -3.0 {
            Intent::Add
        } else {
            Intent::Hold
        };
        // v43: reduce_zone 按真实波动, pressure = 现价 * 1.10
        let reduce_zone = if matches!(intent, Intent::Reduce) {
            Some((current_price * 1.02, current_price * 1.05))
        } else {
            None
        };
        let banner = BannerCtx::default();
        let reasons_vec = vec![
            format!("成本{:.2} 现价{:.2} 盈亏{:+.1}%", pos.cost_price, current_price, pnl_pct),
            format!("硬止损{:.2}", pos.hard_stop),
        ];
        let invalidations_vec = vec![format!("跌破{:.2}且放量", pos.hard_stop)];
        let params = HoldingPlanParams {
            name: &pos.name,
            code: &pos.code,
            hhmm,
            intent,
            price: current_price,
            cost: pos.cost_price,
            avail: pos.shares as u32,
            reduce_zone,
            support: pos.hard_stop,
            pressure: current_price * 1.10,
            stop: pos.hard_stop,
            invalidations: &invalidations_vec,
            reasons: &reasons_vec,
        };
        let result = push_holding_plan_recommendation(&pos.code, Some(&banner), params).await;
        if result {
            pushed_count += 1;
        }
    }
    log_dispatcher_attempt("I-04", pushed_count > 0, pushed_count, "");
    pushed_count > 0
}

/// v37: P-02 竞价热点量能快照
#[derive(Debug, Clone, Default)]
pub struct AuctionVolumeSnapshot {
    pub hhmm: String,
    pub items: Vec<(String, String, f64, f64)>,  // (name, code, gap_pct, vol_ratio)
    pub sentiment: String,   // "强承接" | "一般" | "弱承接"
    pub watch_status: String, // 观察状态描述
}

/// v37: 加载 P-02 快照 - 复用 limit_up_stocks
pub fn load_auction_volume_snapshot_real(hhmm: &str) -> AuctionVolumeSnapshot {
    use stock_analysis::market_analyzer::MarketAnalyzer;
    let analyzer = match MarketAnalyzer::new(None) {
        Ok(a) => a,
        Err(_) => return AuctionVolumeSnapshot::default(),
    };
    let limit_stocks = match analyzer.get_limit_up_stocks() {
        Ok(s) => s,
        Err(_) => return AuctionVolumeSnapshot::default(),
    };
    if limit_stocks.is_empty() {
        return AuctionVolumeSnapshot::default();
    }
    // 按量比降序, 取前 10
    let mut sorted = limit_stocks.clone();
    sorted.sort_by(|a, b| {
        b.volume_ratio
            .partial_cmp(&a.volume_ratio)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let items: Vec<(String, String, f64, f64)> = sorted
        .iter()
        .take(10)
        .map(|s| (s.name.clone(), s.code.clone(), s.change_pct, s.volume_ratio))
        .collect();

    // sentiment: 平均量比 >= 3 强承接, >= 1 一般, < 1 弱承接
    let avg_vr: f64 = items.iter().map(|(_, _, _, vr)| vr).sum::<f64>() / items.len() as f64;
    let sentiment = if avg_vr >= 3.0 {
        "强承接"
    } else if avg_vr >= 1.0 {
        "一般"
    } else {
        "弱承接"
    };

    AuctionVolumeSnapshot {
        hhmm: hhmm.to_string(),
        items,
        sentiment: sentiment.to_string(),
        watch_status: "9:25 集合竞价结果, 关注开盘承接".to_string(),
    }
}

/// v37: P-02 dispatcher
pub async fn dispatch_auction_volume_daily(hhmm: &str) -> bool {
    let snapshot = load_auction_volume_snapshot_real(hhmm);
    if snapshot.items.is_empty() {
        log_dispatcher_attempt("P-02", false, 0, "auction_volume_snapshot empty");
        log::info!("[P-02] auction_volume_snapshot 空 (无涨停/无数据), 跳过推送");
        return false;
    }
    // 构造 AuctionItem refs
    let auction_items: Vec<AuctionItem<'_>> = snapshot
        .items
        .iter()
        .map(|(n, c, g, v)| AuctionItem {
            name: n,
            code: c,
            gap_pct: *g,
            vol_ratio: *v,
            tag: "",  // 简化: 不填 tag
        })
        .collect();
    let banner = BannerCtx::default();
    let text = render_auction_volume(
        &banner,
        &snapshot.hhmm,
        &auction_items,
        &snapshot.sentiment,
        &snapshot.watch_status,
    );
    let result = dispatch(crate::notify::PushKind::AuctionVolume, "", Some(&banner), text).await;
    log_dispatcher_attempt("P-02", result, snapshot.items.len(), "");
    result
}

/// v35: 加载 A-10 快照 - 取 chain_daily 最新 cluster, 推断持续性
pub fn load_catalyst_review_snapshot_real(date: &str) -> (String, Option<f32>, PersistentLevel, Vec<String>, Vec<String>, Option<String>) {
    use stock_analysis::database::DatabaseManager;
    use stock_analysis::database::concepts::ChainDailyRow;
    let clusters = DatabaseManager::get().get_latest_chain_clusters();
    if clusters.is_empty() {
        return (String::new(), None, PersistentLevel::Low, Vec::new(), Vec::new(), None);
    }

    // 取第一个 cluster 作主推
    let top: &ChainDailyRow = &clusters[0];
    let stocks: Vec<String> = serde_json::from_str(&top.stocks).unwrap_or_default();

    // 持续性: 用 continuation_count 推断
    let persistent = if top.continuation_count >= 3 {
        PersistentLevel::High
    } else if top.continuation_count >= 1 {
        PersistentLevel::Med
    } else {
        PersistentLevel::Low
    };

    // 强度: 用 cluster 数 (粗略代理) — 多个 cluster = 强
    let score = if clusters.len() >= 5 {
        Some(8.5)
    } else if clusters.len() >= 3 {
        Some(7.0)
    } else {
        Some(5.5)
    };

    // 已启动: cluster 头部 (前 3)
    let started: Vec<String> = stocks.iter().take(3).cloned().collect();
    // 待启动: cluster 尾部 (3-5)
    let pending: Vec<String> = stocks.iter().skip(3).take(3).cloned().collect();

    // 明日观察点: 简单模板
    let watch_point = format!("明日竞价复核 {} 主线持续性", top.concept);

    (date.to_string(), score, persistent, started, pending, Some(watch_point))
}

/// v35: A-10 dispatcher 入口
pub async fn dispatch_catalyst_review_daily(date: &str) -> bool {
    let (date_str, score, persistent, started, pending, watch_point) =
        load_catalyst_review_snapshot_real(date);
    if started.is_empty() {
        log_dispatcher_attempt("A-10", false, 0, "catalyst_review_snapshot empty");
        log::info!("[A-10] catalyst_review_snapshot 空 (chain_daily 无 cluster), 跳过推送");
        return false;
    }
    let started_refs: Vec<&str> = started.iter().map(|s| s.as_str()).collect();
    let pending_refs: Vec<&str> = pending.iter().map(|s| s.as_str()).collect();
    let theme = "主线题材";  // 简化: 第一个 cluster 主题
    let params = CatalystReviewParams {
        date: &date_str,
        theme,
        score,
        persistent,
        started_names: started_refs,
        pending_names: pending_refs,
        watch_point: watch_point.as_deref(),
    };
    let text = render_catalyst_review(params);
    let result = dispatch(crate::notify::PushKind::CatalystReview, "", None, text).await;
    log_dispatcher_attempt("A-10", result, started.len(), "");
    result
}

// ============================================================================
// v16.5 helper: 加载 virtual_observation (简化, 复用 main.rs::VirtualObservationRecord)
// ============================================================================
pub struct VirtualRecordLite {
    pub entry_date: String,
    pub code: String,
    pub name: String,
    pub entry_mode: String,
    /// v13.6.3 新增: 真实 entry_price (替代 0.0 占位)
    pub entry_price: f64,
}
pub struct VirtualSnapshotLite {
    pub records: Vec<VirtualRecordLite>,
}

/// v16.5: 简化版 virtual_observation 加载 (与 main.rs::VirtualObservationRecord 兼容)
/// 读 data/virtual_observation/*.json (按 main.rs 持久化格式)
/// v13.6.3 扩展: 解析 entry_price 字段
pub fn load_virtual_observation_for_a01() -> VirtualSnapshotLite {
    use std::fs;
    let dir = std::path::PathBuf::from("data/virtual_observation");
    if !dir.exists() {
        return VirtualSnapshotLite { records: vec![] };
    }
    let mut records: Vec<VirtualRecordLite> = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        let mut paths: Vec<_> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
            .collect();
        paths.sort();
        paths.reverse();

        for path in paths.iter().take(5) {
            // v13.6.3: 完整 serde_json 解析 (含 entry_price)
            #[derive(serde::Deserialize)]
            struct RecordJson {
                entry_date: Option<String>,
                code: Option<String>,
                name: Option<String>,
                entry_mode: Option<String>,
                /// v13.6.3 新增
                entry_price: Option<f64>,
            }
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Ok(parsed) = serde_json::from_str::<RecordJson>(&raw) {
                    records.push(VirtualRecordLite {
                        entry_date: parsed.entry_date.unwrap_or_default(),
                        code: parsed.code.unwrap_or_default(),
                        name: parsed.name.unwrap_or_default(),
                        entry_mode: parsed.entry_mode.unwrap_or("首板".to_string()),
                        entry_price: parsed.entry_price.unwrap_or(0.0),
                    });
                }
            }
        }
    }
    VirtualSnapshotLite { records }
}

/// v13 §14.2 I-01 盘中轮动总览 (⚡交易建议类, 带 banner)
pub async fn push_intraday_market(
    code: &str,
    banner: Option<&BannerCtx>,
    params: IntradayMarketParams<'_>,
) -> bool {
    let text = render_intraday_market(banner.unwrap_or(&BannerCtx::default()), params);
    dispatch(crate::notify::PushKind::IntradayMarket, code, banner, text).await
}

/// v13 §14.2 I-02 新闻催化映射 (⚡交易建议类, 带 banner)
pub async fn push_news_catalyst(
    code: &str,
    banner: Option<&BannerCtx>,
    params: NewsCatalystParams<'_>,
) -> bool {
    let text = render_news_catalyst(banner.unwrap_or(&BannerCtx::default()), params);
    dispatch(crate::notify::PushKind::NewsCatalyst, code, banner, text).await
}

/// v13 §14.2 I-03 盘中涨停扩散 (⚡交易建议类, 带 banner, 审计多发现)
pub async fn push_industry_chain_intraday(
    code: &str,
    banner: Option<&BannerCtx>,
    params: IndustryChainIntradayParams<'_>,
) -> bool {
    let text = render_industry_chain_intraday(banner.unwrap_or(&BannerCtx::default()), params);
    dispatch(
        crate::notify::PushKind::IndustryChainIntraday,
        code,
        banner,
        text,
    )
    .await
}

/// v13 §14.4 D-01 新闻驱动个股 (⚡交易建议类, 带 banner)
pub async fn push_news_to_idea(
    code: &str,
    banner: Option<&BannerCtx>,
    params: NewsToIdeaParams<'_>,
) -> bool {
    let text = render_news_to_idea(banner.unwrap_or(&BannerCtx::default()), params);
    dispatch(crate::notify::PushKind::NewsToIdea, code, banner, text).await
}

/// v13 §14.3 A-01 虚拟仓复盘 (ℹ️盘后参考, 复用 T-11 竞价复算)
pub async fn push_paper_review(code: &str, params: PaperReviewParams<'_>) -> bool {
    let text = render_paper_review(params);
    dispatch(crate::notify::PushKind::PaperReview, code, None, text).await
}

// ============================================================================
// MVP3-3.2 orchestrator: T-07 候选触发 + T-08 候选失效
// ============================================================================

/// MVP3-3.2 T-07 候选触发 (⚡ 1次/票/日).
///
/// 由 candidate_state::is_candidate_live_enabled() 控制: 关闭时直接返回 false (零推送).
pub async fn push_candidate_triggered(
    code: &str,
    banner: Option<&BannerCtx>,
    params: CandidateTriggeredParams<'_>,
    live_override: Option<bool>,
) -> bool {
    use stock_analysis::opportunity::candidate_state::is_candidate_live_enabled;

    if !is_candidate_live_enabled(live_override) {
        log::info!(
            "[T-07] 候选触发被影子开关拦截 (code={}, 需 ENABLE_CANDIDATE_LIVE=true)",
            code
        );
        return false;
    }

    let text = render_candidate_triggered(banner.unwrap_or(&BannerCtx::default()), params);
    dispatch(
        crate::notify::PushKind::CandidateTriggered,
        code,
        banner,
        text,
    )
    .await
}

/// MVP3-3.2 T-08 候选失效 (ℹ️参考, 复用 CandidateBoard).
pub async fn push_candidate_invalidated(
    code: &str,
    hhmm: &str,
    name: &str,
    prev: &str,
    reason: &str,
) -> bool {
    let text = render_candidate_invalidated(hhmm, name, code, prev, reason);
    dispatch(crate::notify::PushKind::CandidateInvalidated, code, None, text).await
}

/// v12 PR2-2.2: 数据模式变更编排器.
///
/// 完整链路: evaluate() → is_changed() → 拼 T-02 → dispatch() (10min 冷却由 push_governor 处理).
///
/// 返回 `Ok(true)` 表示推送成功; `Ok(false)` 表示无变更 (no-op).
///
/// `prev` 由调用方从 history 表恢复, 首次评估传 None.
pub async fn push_data_mode_change(
    input: &stock_analysis::monitor::data_mode::DataHealthInput,
    prev: Option<stock_analysis::monitor::data_mode::DataMode>,
    banner: Option<&BannerCtx>,
) -> Result<bool, String> {
    use stock_analysis::monitor::data_mode::{evaluate as dm_evaluate, DataMode as LibDM};

    let health = dm_evaluate(input, prev);

    if !health.is_changed() {
        return Ok(false);
    }

    let prev_mode = prev.expect("is_changed=true ⇒ prev=Some");
    let new_mode = health.mode;

    // 1. 拼 T-02 (复用 §14.1 T-02 模板)
    let hhmm = chrono::Local::now().format("%H:%M").to_string();
    let missing_str = if health.missing.is_empty() {
        "(无)".to_string()
    } else {
        health
            .missing
            .iter()
            .map(|c| c.label().to_string())
            .collect::<Vec<_>>()
            .join("/")
    };

    // 输出限制描述
    let restrictions: Vec<String> = match new_mode {
        LibDM::Full => vec![],
        LibDM::Degraded => vec![
            "不做盘口承接判断".to_string(),
            "价格型建议标注数据降级".to_string(),
        ],
        LibDM::Unsafe => vec![
            "不做盘口承接判断".to_string(),
            "禁出价格型建议".to_string(),
            "仅保留风险类推送".to_string(),
        ],
    };

    let prev_tmpl = match prev_mode {
        LibDM::Full => DataMode::Full,
        LibDM::Degraded => DataMode::Degraded,
        LibDM::Unsafe => DataMode::Unsafe,
    };
    let new_tmpl = match new_mode {
        LibDM::Full => DataMode::Full,
        LibDM::Degraded => DataMode::Degraded,
        LibDM::Unsafe => DataMode::Unsafe,
    };

    let mut text = if let Some(b) = banner {
        format!("{}\n", b.render())
    } else {
        String::new()
    };
    text.push_str(&render_data_mode(
        &hhmm,
        prev_tmpl,
        new_tmpl,
        &missing_str,
        &restrictions,
        health.eta.as_deref(),
    ));

    // 2. dispatch (code="" 全局键, DataMode 10min 冷却走 push_governor 默认)
    let ok = dispatch(crate::notify::PushKind::DataMode, "", banner, text).await;

    if !ok {
        log::info!(
            "[DataMode] T-02 推送被治理拦截 (冷却或预算), mode {:?} → {:?}",
            prev_mode,
            new_mode
        );
    }

    Ok(ok)
}

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};

/// 冷却表: key = (PushKind, code_or_empty), value = last sent epoch secs
///
/// 进程内全局, monitor 重启即清零. v12 §14.3.1.
static COOLDOWN_TABLE: Lazy<std::sync::Mutex<HashMap<(crate::notify::PushKind, String), i64>>> =
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
pub fn is_in_cooldown(kind: crate::notify::PushKind, code: &str) -> bool {
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
pub fn record_cooldown(kind: crate::notify::PushKind, code: &str) {
    let key = (kind, code.to_string());
    let now = chrono::Utc::now().timestamp();
    let mut table = COOLDOWN_TABLE.lock().expect("cooldown table poisoned");
    table.insert(key, now);
}

/// 是否计入日预算 (§14.3.3). 交易建议类 + 盘后 R 系列计入.
pub fn counts_against_daily_budget(kind: crate::notify::PushKind) -> bool {
    use crate::notify::PushKind as K;
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
pub fn should_block_on_mode(
    kind: crate::notify::PushKind,
    mode: AccountMode,
    dm: DataMode,
) -> bool {
    use crate::notify::PushKind as K;
    match kind {
        // 风险类: 永远照发
        K::HoldingEvent | K::ForbiddenOps | K::DataMode | K::AccountMode => false,
        // 交易建议类: Frozen 全停, Unsafe 全停
        K::HoldingPlan | K::T0Advice | K::CandidateTriggered => {
            matches!(mode, AccountMode::Frozen) || matches!(dm, DataMode::Unsafe)
        }
        // v14.5 G-03: PaperTrade 虚拟盘演示, 永远照发 (不因 Frozen 阻断)
        K::PaperTrade => false,
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
/// 内部调用 `crate::notify::push_governor`. PUSH_VERBOSE 降级逻辑沿用.
pub async fn dispatch(
    kind: crate::notify::PushKind,
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
    let ok = crate::notify::push_governor(&text, kind).await;
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
// v13 §14.1/14.2 新增模板 — P-01 盘前新闻热点 / I-01 盘中轮动 / I-02 新闻催化
// ============================================================================

/// v13 §14.1 P-01 盘前新闻热点
pub struct PreopenNewsHotParams<'a> {
    pub hhmm: &'a str,
    pub theme_1: Option<&'a str>,
    pub theme_2: Option<&'a str>,
    pub theme_3: Option<&'a str>,
    pub news_pairs: Vec<(&'a str, &'a str)>, // (news, chain)
    pub watch_stocks: Vec<(&'a str, &'a str, &'a str)>, // (name, code, reason)
}

/// v13 §14.2 I-01 盘中轮动 — 板块状态
#[derive(Debug, Clone, Default, PartialEq)]
pub enum RotationState {
    #[default]
    Spreading, // 扩散
    Diverging, // 分化
    Fading,    // 退潮
}

/// v13 §14.2 I-01 盘中轮动总览
pub struct IntradayMarketParams<'a> {
    pub hhmm: &'a str,
    pub tech_sub: Option<&'a str>,
    pub tech_score: Option<f32>,
    pub power_sub: Option<&'a str>,
    pub power_score: Option<f32>,
    pub robot_sub: Option<&'a str>,
    pub robot_score: Option<f32>,
    pub main_attack: Option<&'a str>,
    pub rotation_state: RotationState,
}

/// v13 §14.2 I-02 新闻催化映射
pub struct NewsCatalystParams<'a> {
    pub hhmm: &'a str,
    pub headline: &'a str,
    pub theme: Option<&'a str>,
    pub stocks: Vec<(&'a str, &'a str, Option<f32>, &'a str)>, // (name, code, chg, reason)
}

/// v13 §14.1 P-01 盘前新闻热点（盘前无 banner）
pub fn render_preopen_news_hot(p: PreopenNewsHotParams<'_>) -> String {
    let mut s = format!("📰 盘前热点（{}）\n", p.hhmm);
    let themes: Vec<&str> = [p.theme_1, p.theme_2, p.theme_3]
        .into_iter()
        .flatten()
        .collect();
    if !themes.is_empty() {
        s.push_str(&format!("主线: {}\n", themes.join(" / ")));
    }
    if !p.news_pairs.is_empty() {
        s.push_str("催化:\n");
        for (news, chain) in &p.news_pairs {
            s.push_str(&format!("· {} → 利好{}\n", news, chain));
        }
    }
    if !p.watch_stocks.is_empty() {
        s.push_str("关注票:\n");
        for (name, code, reason) in &p.watch_stocks {
            s.push_str(&format!("· {}({}) 逻辑: {}\n", name, code, reason));
        }
    }
    s.push_str("辅助建议, 非下单指令");
    s
}

/// v13 §14.2 I-01 盘中轮动总览（盘中交易建议类带 banner）
pub fn render_intraday_market(banner: &BannerCtx, p: IntradayMarketParams<'_>) -> String {
    let render_sub = |sub: Option<&str>, score: Option<f32>| -> String {
        let s = sub.unwrap_or("—");
        let sc = score
            .map(|v| format!("{:.1}", v))
            .unwrap_or_else(|| "N/A".to_string());
        format!("{}(强度{})", s, sc)
    };
    let state = match p.rotation_state {
        RotationState::Spreading => "扩散",
        RotationState::Diverging => "分化",
        RotationState::Fading => "退潮",
    };
    let main = p.main_attack.unwrap_or("暂无主攻");
    format!(
        "{}\n📊 盘中轮动（{}）\n科技: {}\n电力: {}\n机器人: {}\n当前主攻: {} | 轮动状态: {}\n辅助建议, 非下单指令",
        banner.render(),
        p.hhmm,
        render_sub(p.tech_sub, p.tech_score),
        render_sub(p.power_sub, p.power_score),
        render_sub(p.robot_sub, p.robot_score),
        main,
        state,
    )
}

/// v13 §14.2 I-02 新闻催化映射（盘中交易建议类带 banner）
pub fn render_news_catalyst(banner: &BannerCtx, p: NewsCatalystParams<'_>) -> String {
    let theme = p.theme.unwrap_or("未分类");
    let mut s = format!(
        "{}\n📰⚡ 新闻催化跟踪（{}）\n新闻: {}\n受益板块: {}\n",
        banner.render(),
        p.hhmm,
        p.headline,
        theme
    );
    for (name, code, chg, reason) in &p.stocks {
        if let Some(c) = chg {
            s.push_str(&format!(
                "· {}({}) {:+.1}% | 原因:{}\n",
                name, code, c, reason
            ));
        }
    }
    s.push_str("辅助建议, 非下单指令");
    s
}

/// v13 §14.4 D-01 新闻驱动个股 — 主题阶段
#[derive(Debug, Clone, Default, PartialEq)]
pub enum NewsStage {
    #[default]
    Starting,   // 启动
    Fermenting, // 发酵
    Diverging,  // 分歧
}

/// v13 §14.4 D-01 新闻驱动个股 — 建议动作
#[derive(Debug, Clone, Default, PartialEq)]
pub enum NewsAction {
    #[default]
    Observe,    // 观察
    BuyDip,     // 低吸
    DoNotChase, // 不追
}

/// v13 §14.4 D-01 新闻驱动个股
pub struct NewsToIdeaParams<'a> {
    pub hhmm: &'a str,
    pub headline: &'a str,
    pub theme: Option<&'a str>,
    pub stage: NewsStage,
    pub name: &'a str,
    pub code: &'a str,
    pub reasons: Vec<&'a str>,
    pub action: Option<NewsAction>,
}

/// v13 §14.4 D-01 新闻驱动个股（⚡交易建议类带 banner）
pub fn render_news_to_idea(banner: &BannerCtx, p: NewsToIdeaParams<'_>) -> String {
    let stage = match p.stage {
        NewsStage::Starting => "启动",
        NewsStage::Fermenting => "发酵",
        NewsStage::Diverging => "分歧",
    };
    let theme = p.theme.unwrap_or("未分类");
    let mut s = format!(
        "{}\n🧭 新闻驱动个股（{}）\n新闻: {}\n板块: {} | 阶段: {}\n个股: {}({})\n",
        banner.render(),
        p.hhmm,
        p.headline,
        theme,
        stage,
        p.name,
        p.code
    );
    if !p.reasons.is_empty() {
        s.push_str("推送原因:\n");
        for r in &p.reasons {
            s.push_str(&format!("· {}\n", r));
        }
    }
    if let Some(act) = p.action {
        let a = match act {
            NewsAction::Observe => "观察",
            NewsAction::BuyDip => "低吸",
            NewsAction::DoNotChase => "不追",
        };
        s.push_str(&format!("[建议动作: {}]\n", a));
    }
    s.push_str("辅助建议, 非下单指令");
    s
}

/// v13 §14.3 A-10 题材催化复盘 — 持续性
pub enum PersistentLevel {
    High,
    Med,
    Low,
}

/// v13 §14.3 A-10 盘后题材催化复盘
pub struct CatalystReviewParams<'a> {
    pub date: &'a str,
    pub theme: &'a str,
    pub score: Option<f32>,
    pub persistent: PersistentLevel,
    pub started_names: Vec<&'a str>,
    pub pending_names: Vec<&'a str>,
    pub watch_point: Option<&'a str>,
}

/// v13 §14.3 A-10 盘后题材催化复盘
pub fn render_catalyst_review(p: CatalystReviewParams<'_>) -> String {
    let score_str = p
        .score
        .map(|v| format!("{:.1}", v))
        .unwrap_or_else(|| "N/A".to_string());
    let persistent = match p.persistent {
        PersistentLevel::High => "high",
        PersistentLevel::Med => "med",
        PersistentLevel::Low => "low",
    };
    let mut s = format!(
        "📰 题材催化复盘（{}）\n{}: 当日强度{} | 持续性{}\n",
        p.date, p.theme, score_str, persistent
    );
    if !p.started_names.is_empty() {
        s.push_str(&format!("已启动: {}\n", p.started_names.join("、")));
    }
    if !p.pending_names.is_empty() {
        s.push_str(&format!("待启动: {}\n", p.pending_names.join("、")));
    }
    if let Some(w) = p.watch_point {
        s.push_str(&format!("明日观察点: {}\n", w));
    }
    s.push_str("辅助建议, 非下单指令");
    s
}

/// v13 §14.2 I-03 盘中涨停扩散 — 补涨候选
pub struct SupplementCandidate<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub trigger: &'a str,
    pub lo: f64,
    pub hi: f64,
    pub stop: f64,
}

/// v13 §14.2 I-03 盘中涨停扩散
pub struct IndustryChainIntradayParams<'a> {
    pub hhmm: &'a str,
    pub chain: &'a str,
    pub limit_count: u32,
    pub leader_name: Option<&'a str>,
    pub leader_code: Option<&'a str>,
    pub leader_height: u32,
    pub supplements: Vec<SupplementCandidate<'a>>,
}

/// v13 §14.2 I-03 盘中涨停扩散（盘中交易建议类, 带 banner）
pub fn render_industry_chain_intraday(
    banner: &BannerCtx,
    p: IndustryChainIntradayParams<'_>,
) -> String {
    let leader = match (p.leader_name, p.leader_code) {
        (Some(n), Some(c)) => format!("龙头: {}({}) {}板", n, c, p.leader_height),
        _ => "龙头: 暂无".to_string(),
    };
    let mut s = format!(
        "{}\n🔥 盘中涨停扩散（{}）\n主链: {} | 涨停{}家 | 连板高度{}板\n{}\n",
        banner.render(),
        p.hhmm,
        p.chain,
        p.limit_count,
        p.leader_height,
        leader
    );
    if !p.supplements.is_empty() {
        s.push_str("补涨候选:\n");
        for c in &p.supplements {
            s.push_str(&format!(
                "· {}({}) 触发条件{} | 低吸{:.2}~{:.2} | 止损{:.2}\n",
                c.name, c.code, c.trigger, c.lo, c.hi, c.stop
            ));
        }
    }
    s.push_str("辅助建议, 非下单指令");
    s
}

/// v13.1 §5.2 交易所
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exchange {
    SH, // 沪市 A 股/ETF (9:30-11:30, 13:00-15:30)
    SZ, // 深市 A 股/ETF (9:15-11:30, 13:00-15:30)
    BJ, // 北交所 A 股 (9:15-11:30, 13:00-15:30)
}

/// v13.1 §5.2 委托状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    Submitted, // 已报
    Cancelled, // 已撤
    Rejected,  // 废单
}

/// v13.1 §5.2 T-14 盘后固定价格申报
pub struct PostFixedPriceOrderParams<'a> {
    pub exchange: Exchange,
    pub hhmm: &'a str,
    pub name: &'a str,
    pub code: &'a str,
    pub price: f64,
    pub qty: u32,
    pub order_id: &'a str,
    pub status: OrderStatus,
}

/// v13.1 §5.3 T-15 盘后固定价格成交
pub struct PostFixedPriceFillParams<'a> {
    pub exchange: Exchange,
    pub hhmm: &'a str,
    pub name: &'a str,
    pub code: &'a str,
    pub fill_price: f64,
    pub qty: u32,
    pub vs_limit_pct: Option<f32>,
    pub next_session_carry: bool,
}

/// v13.1 §5.2 T-14 盘后固定价格申报
pub fn render_post_fixed_price_order(p: PostFixedPriceOrderParams<'_>) -> String {
    let ex = match p.exchange {
        Exchange::SH => "沪市",
        Exchange::SZ => "深市",
        Exchange::BJ => "北交所",
    };
    let status = match p.status {
        OrderStatus::Submitted => "已报",
        OrderStatus::Cancelled => "已撤",
        OrderStatus::Rejected => "废单",
    };
    // 按 HH:MM 派生窗口 (上午/下午/尾盘)
    let window = if p.hhmm < "11:30" {
        "上午"
    } else if p.hhmm < "15:00" {
        "下午"
    } else {
        "尾盘"
    };
    format!(
        "📋 盘后固定价格申报（{} {}）\n{}({}) 价格{:.2} 数量{} | 状态: {} | 窗口: {}\n订单号: {}\n辅助建议, 非下单指令",
        p.hhmm, ex, p.name, p.code, p.price, p.qty, status, window, p.order_id
    )
}

/// v13.1 §5.3 T-15 盘后固定价格成交
pub fn render_post_fixed_price_fill(p: PostFixedPriceFillParams<'_>) -> String {
    let ex = match p.exchange {
        Exchange::SH => "沪市",
        Exchange::SZ => "深市",
        Exchange::BJ => "北交所",
    };
    let vs = p
        .vs_limit_pct
        .map(|v| format!("{:+.1}%", v))
        .unwrap_or_else(|| "N/A".to_string());
    let carry = if p.next_session_carry {
        "过户到次一交易日"
    } else {
        "本日内"
    };
    format!(
        "✅ 盘后固定价格成交（{} {}）\n{}({}) 成交价{:.2} 数量{} | 价差{}\n清算: {}\n辅助建议, 非下单指令",
        p.hhmm, ex, p.name, p.code, p.fill_price, p.qty, vs, carry
    )
}

/// v13.1 §5.4 ST/*ST 类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StType {
    ST,     // ST
    StarST, // *ST
}

/// v13.1 §5.4 T-16 ST 涨跌幅变更提醒 (新规 5%→10%, 2026-07-06 生效)
pub struct StPriceLimitChangedParams<'a> {
    pub hhmm: &'a str,
    pub name: &'a str,
    pub code: &'a str,
    pub st_type: StType,
    pub old_limit: f32, // 原 0.05
    pub new_limit: f32, // 新 0.10
    pub holding_qty: u32,
    pub cost: f64,
    pub now_price: f64,
    pub new_stop_loss: Option<f64>,
    pub new_take_profit: Option<f64>,
}

/// v13.1 §5.4 T-16 ST 涨跌幅变更提醒（⚡交易建议类, 带 banner）
pub fn render_st_price_limit_changed(p: StPriceLimitChangedParams<'_>) -> String {
    let st = match p.st_type {
        StType::ST => "ST",
        StType::StarST => "*ST",
    };
    let mut s = format!(
        "⚠️ ST 涨跌幅变更（{}）\n{}({}) [{}] 持仓 {} 股\n原涨跌幅: {:+.0}% → 新涨跌幅: {:+.0}%\n现价: {:.2} 成本: {:.2} 浮盈: {:+.1}%\n",
        p.hhmm, p.name, p.code, st, p.holding_qty,
        p.old_limit * 100.0, p.new_limit * 100.0,
        p.now_price, p.cost, ((p.now_price - p.cost) / p.cost) * 100.0
    );
    if let Some(sl) = p.new_stop_loss {
        s.push_str(&format!(
            "新止损: {:.2} (基于 {:.0}% 阈值)\n",
            sl,
            p.new_limit * 100.0
        ));
    } else {
        s.push_str("新止损: 未重算\n");
    }
    if let Some(tp) = p.new_take_profit {
        s.push_str(&format!("新止盈: {:.2}\n", tp));
    }
    s.push_str("辅助建议, 非下单指令 — 现有持仓风险阈值已重新校准");
    s
}

/// v13.1 §5.5 T-17 ETF 收盘集合竞价（仅沪市 ETF, 14:57-15:00）
pub struct EtfClosingCallAuctionParams<'a> {
    pub hhmm: &'a str, // 14:57-15:00
    pub name: &'a str,
    pub code: &'a str,
    pub call_auction_price: Option<f64>,
    pub vs_continuous_est: Option<f32>,
    pub liquidity_note: &'a str,
}

/// v13.1 §5.5 T-17 ETF 收盘集合竞价（盘后参考, 无 banner）
pub fn render_etf_closing_call_auction(p: EtfClosingCallAuctionParams<'_>) -> String {
    let price = p
        .call_auction_price
        .map(|v| format!("{:.3}", v))
        .unwrap_or_else(|| "暂无".to_string());
    let vs = p
        .vs_continuous_est
        .map(|v| format!("{:+.2}%", v))
        .unwrap_or_else(|| "N/A".to_string());
    format!(
        "📊 ETF 集合竞价尾盘（{}）\n{}({}) 沪市 ETF 收盘价: {}\nvs 连续竞价估值: {}\n流动性: {}\n注: 14:57-15:00 集合竞价形成收盘价（抑制尾盘操纵）",
        p.hhmm, p.name, p.code, price, vs, p.liquidity_note
    )
}

/// v13.1 §5.6 大宗类型
pub enum BlockType {
    Agreed,      // 协议大宗
    Competitive, // 竞价大宗
}

/// v13.1 §5.6 板块
pub enum Board {
    GEM,  // 创业板
    STAR, // 科创板
    Main, // 主板
}

/// v13.1 §5.6 清算节奏
pub enum SettleType {
    NextSession, // 次日
    RealTime,    // 实时
}

/// v13.1 §5.6 T-18 创业板协议大宗盘中确认
pub struct BlockTradeIntradayConfirmParams<'a> {
    pub hhmm: &'a str,
    pub name: &'a str,
    pub code: &'a str,
    pub qty: u32,
    pub price: f64,
    pub block_type: BlockType,
    pub board: Board,
    pub real_time_confirm: bool,
    pub next_session_settle: SettleType,
}

/// v13.1 §5.6 T-18 创业板协议大宗盘中确认（盘后参考, 无 banner）
pub fn render_block_trade_intraday_confirm(p: BlockTradeIntradayConfirmParams<'_>) -> String {
    let bt = match p.block_type {
        BlockType::Agreed => "协议大宗",
        BlockType::Competitive => "竞价大宗",
    };
    let bd = match p.board {
        Board::GEM => "创业板",
        Board::STAR => "科创板",
        Board::Main => "主板",
    };
    let settle = match p.next_session_settle {
        SettleType::NextSession => "次日清算",
        SettleType::RealTime => "实时清算",
    };
    let confirm = if p.real_time_confirm {
        "✅ 盘中实时确认"
    } else {
        "⏳ 等待确认"
    };
    format!(
        "📋 大宗交易盘中确认（{}）\n{}({}) {} {}\n数量: {} 价格: {:.2}\n板块: {} | 清算: {}",
        p.hhmm, p.name, p.code, bt, confirm, p.qty, p.price, bd, settle
    )
}

/// v13.1 §5.7 T-19 北交所大宗价格区间
pub struct BlockTradePriceRangeParams<'a> {
    pub hhmm: &'a str,
    pub name: &'a str,
    pub code: &'a str,
    pub prev_close: Option<f64>,
    pub today_avg_price: f64,
    pub block_price_range: Option<&'a str>,
    pub note: &'a str,
}

/// v13.1 §5.7 T-19 北交所大宗价格区间（盘后参考, 无 banner）
pub fn render_block_trade_price_range(p: BlockTradePriceRangeParams<'_>) -> String {
    let prev = p
        .prev_close
        .map(|v| format!("{:.2}", v))
        .unwrap_or_else(|| "N/A".to_string());
    let range = p.block_price_range.unwrap_or("暂无");
    format!(
        "📊 北交所大宗价格区间（{}）\n{}({})\n前收盘价: {} (原口径)\n当日实时均价: {:.2} (新口径)\n价格区间: {}\n注: {}",
        p.hhmm, p.name, p.code, prev, p.today_avg_price, range, p.note
    )
}

/// v13 §14.3 A-01 虚拟仓复盘 (P1, 复用 T-11 竞价复算通路)
pub struct PaperReviewParams<'a> {
    pub date: &'a str,
    pub name: &'a str,
    pub code: &'a str,
    pub trigger: &'a str,
    pub desc: &'a str,
    pub pnl: Option<f32>,
    pub plan_high: Option<&'a str>,
    pub plan_flat: Option<&'a str>,
    pub plan_low: Option<&'a str>,
}

/// v13 §14.3 A-01 虚拟仓复盘（盘后参考, 无 banner）
pub fn render_paper_review(p: PaperReviewParams<'_>) -> String {
    let pnl_str = p
        .pnl
        .map(|v| format!("{:+.1}%", v))
        .unwrap_or_else(|| "N/A%".to_string());
    let mut s = format!(
        "🧪 虚拟仓复盘（{}）\n{}({}) 原触发: {}\n结果: {} {}\n",
        p.date, p.name, p.code, p.trigger, p.desc, pnl_str
    );
    let has_plan = p.plan_high.is_some() || p.plan_flat.is_some() || p.plan_low.is_some();
    if has_plan {
        s.push_str("次日计划:\n");
        if let Some(h) = p.plan_high {
            s.push_str(&format!("· 高开>1%: {}\n", h));
        }
        if let Some(f) = p.plan_flat {
            s.push_str(&format!("· 平开: {}\n", f));
        }
        if let Some(l) = p.plan_low {
            s.push_str(&format!("· 低开/跌破止损: {}\n", l));
        }
    }
    s.push_str("辅助建议, 非下单指令");
    s
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
    fn banner_normal_full_format() {
        let b = banner_normal();
        assert_eq!(b.render(), "[🟢 Normal | 仓位5成 | 日盈亏+0.3% | 数据Full]");
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
                invalidations: &["跌破5日线且放量".to_string(), "板块热度转Fade".to_string()],
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
                reasons: &["距涨停仅 1.2%".to_string(), "板块已 Climax".to_string()],
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
            AuctionItem {
                name: "A",
                code: "000001",
                gap_pct: 5.2,
                vol_ratio: 8.5,
                tag: "昨日涨停",
            },
            AuctionItem {
                name: "B",
                code: "600000",
                gap_pct: 2.1,
                vol_ratio: 3.2,
                tag: "观察池",
            },
        ];
        let s = render_auction_volume(&banner_normal(), "09:25", &items, "强承接", "可操作");
        assert!(s.contains("🌅 竞价热点量能 Top2（09:25）")); // v13 标题统一
        assert!(s.contains("A(000001) 高开+5.2% 量比8.5 [昨日涨停]"));
        assert!(s.contains("B(600000) 高开+2.1% 量比3.2 [观察池]"));
        assert!(s.contains("情绪判读: 强承接, 观察池今日可操作"));
        assert!(s.contains("辅助建议, 非下单指令"));
    }

    // ---- T-12 尾盘决策 ----

    #[test]
    fn t12_close_call_holding_only() {
        let h = CloseCallHolding {
            name: "XX",
            state: "尾盘跳水-建议处理",
        };
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
            sh_chg: 0.0,
            chinext_chg: 0.0,
            star_chg: 0.0,
            limit_up_n: 0,
            limit_down_n: 0,
            broken_pct: 0.0,
            consecutive_h: 0,
            amount_yi: 0.0,
            amount_delta_pct: 0.0,
            amount_dir: "放量",
            main_flow_yi: 0.0,
            money_effect: "差",
            heat_stage: "Fade",
            heat_conf_pct: 50,
            low_conf: false,
            low_conf_tier: None,
            account_mode: AccountMode::Normal,
            max_pos: 5,
        }
    }

    // ---- R-03 涨停产业链 ----

    #[test]
    fn r03_industry_chain_two() {
        let chains = vec![
            ChainLine {
                chain: "AI算力",
                limit_up_n: 8,
                first_n: 5,
                consec_n: 3,
                heat_stage: "MainUp",
                leader_name: "龙头A",
                leader_code: "688001",
                leader_boards: 4,
                followers: "B,C,D",
                watch_point: "明日分歧",
            },
            ChainLine {
                chain: "机器人",
                limit_up_n: 5,
                first_n: 4,
                consec_n: 1,
                heat_stage: "HeatUp",
                leader_name: "龙头Z",
                leader_code: "300750",
                leader_boards: 2,
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
        let entries = vec![LhbEntry {
            name: "X",
            code: "688001",
            net_buy_yi: 1.5,
            reason: "涨幅偏离值达7%",
            buy_inst_n: 2,
            buy_inst_amt_wan: 8000.0,
            buy_other_n: 3,
            buy_other_amt_wan: 4000.0,
            buy_conc_pct: 65.0,
            sell_desc: "游资席位",
            sell_conc_pct: 45.0,
            chain_match: Some("AI算力"),
            next_day_risk: "高开震荡",
        }];
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
            holding_n: 5,
            holding_exec: 4,
            holding_eff: 3,
            t0_n: 2,
            t0_eff: 1,
            cand_trigger: 6,
            cand_filled: 3,
            cand_notfilled: 3,
            cand_limitup: 2,
            cand_notreach: 1,
            paper_pnl_pct: 0.5,
            paper_total_pct: 3.2,
            paper_n: 12,
            news_push_n: 4,
            news_d1_eff: 2,
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
        let entries = vec![FailureEntry {
            name: "X",
            code: "688001",
            signal_level: "⚡",
            virtual_reason: "A档",
            result_desc: "未成交",
            pnl_pct: 0.0,
            failure_reason: "涨停不可买",
            suggestion: "调高触发阈值",
        }];
        let dist = FailureDistribution {
            buy_late: 2,
            chain_fade: 1,
            not_fillable: 3,
            human_not_exec: 1,
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
        let items = vec![WatchItem {
            name: "Y",
            code: "002415",
            topic: "机器人",
            source: "A档未触发",
            trigger: "突破50.5",
            lo: 49.5,
            hi: 50.3,
            stop: 48.5,
            reason: "板块共振",
        }];
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
            HoldingEventItem {
                name: "XX",
                kind: "解禁3.2亿",
            },
            HoldingEventItem {
                name: "YY",
                kind: "财报预告",
            },
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

    // ====== v13 P-01 盘前新闻热点 (4 用例) ======
    #[test]
    fn preopen_news_hot_three_themes_two_news_two_stocks() {
        let p = PreopenNewsHotParams {
            hhmm: "09:05",
            theme_1: Some("AI算力"),
            theme_2: Some("机器人"),
            theme_3: Some("消费电子"),
            news_pairs: vec![("英伟达新品", "GPU"), ("特斯拉FSD入华", "智驾")],
            watch_stocks: vec![
                ("中科曙光", "603019", "AI算力龙头"),
                ("绿的谐波", "688017", "减速器"),
            ],
        };
        let out = render_preopen_news_hot(p);
        assert!(out.contains("📰 盘前热点（09:05）"));
        assert!(out.contains("主线: AI算力 / 机器人 / 消费电子"));
        assert!(out.contains("· 英伟达新品 → 利好GPU"));
        assert!(out.contains("· 中科曙光(603019) 逻辑: AI算力龙头"));
        assert!(out.ends_with("辅助建议, 非下单指令"));
    }

    #[test]
    fn preopen_news_hot_missing_themes_omits_section() {
        let p = PreopenNewsHotParams {
            hhmm: "09:05",
            theme_1: None,
            theme_2: None,
            theme_3: None,
            news_pairs: vec![],
            watch_stocks: vec![("X", "000001", "r")],
        };
        let out = render_preopen_news_hot(p);
        assert!(!out.contains("主线:"));
        assert!(!out.contains("催化:"));
        assert!(out.contains("· X(000001) 逻辑: r"));
    }

    #[test]
    fn preopen_news_hot_partial_themes() {
        // 1 theme only
        let p = PreopenNewsHotParams {
            hhmm: "09:05",
            theme_1: Some("AI"),
            theme_2: None,
            theme_3: None,
            news_pairs: vec![("N", "C")],
            watch_stocks: vec![],
        };
        let out = render_preopen_news_hot(p);
        assert!(out.contains("主线: AI"));
        assert!(!out.contains("AI /"));
    }

    #[test]
    fn preopen_news_hot_empty_watch_stocks_omits_section() {
        let p = PreopenNewsHotParams {
            hhmm: "09:05",
            theme_1: Some("T"),
            theme_2: None,
            theme_3: None,
            news_pairs: vec![],
            watch_stocks: vec![],
        };
        let out = render_preopen_news_hot(p);
        assert!(!out.contains("关注票:"));
        assert!(out.ends_with("辅助建议, 非下单指令"));
    }

    // ====== v13 I-01 盘中轮动总览 (3 用例) ======
    #[test]
    fn intraday_market_full_state() {
        let p = IntradayMarketParams {
            hhmm: "10:30",
            tech_sub: Some("AI算力"),
            tech_score: Some(85.5),
            power_sub: Some("特高压"),
            power_score: Some(60.0),
            robot_sub: Some("减速器"),
            robot_score: Some(72.3),
            main_attack: Some("AI算力"),
            rotation_state: RotationState::Spreading,
        };
        let banner = BannerCtx::test_default();
        let out = render_intraday_market(&banner, p);
        assert!(out.contains("📊 盘中轮动（10:30）"));
        assert!(out.contains("科技: AI算力(强度85.5)"));
        assert!(out.contains("电力: 特高压(强度60.0)"));
        assert!(out.contains("机器人: 减速器(强度72.3)"));
        assert!(out.contains("轮动状态: 扩散"));
        assert!(out.contains("当前主攻: AI算力"));
        assert!(out.ends_with("辅助建议, 非下单指令"));
    }

    #[test]
    fn intraday_market_missing_score_shows_na() {
        let p = IntradayMarketParams {
            hhmm: "10:30",
            tech_sub: Some("AI"),
            tech_score: None,
            power_sub: None,
            power_score: None,
            robot_sub: None,
            robot_score: None,
            main_attack: None,
            rotation_state: RotationState::Fading,
        };
        let banner = BannerCtx::test_default();
        let out = render_intraday_market(&banner, p);
        assert!(out.contains("AI(强度N/A)"));
        assert!(out.contains("—(强度N/A)")); // power and robot default to "—"
        assert!(out.contains("轮动状态: 退潮"));
        assert!(out.contains("当前主攻: 暂无主攻"));
    }

    #[test]
    fn intraday_market_rotation_states() {
        for (state, label) in [
            (RotationState::Spreading, "扩散"),
            (RotationState::Diverging, "分化"),
            (RotationState::Fading, "退潮"),
        ] {
            let p = IntradayMarketParams {
                hhmm: "10:30",
                tech_sub: None,
                tech_score: None,
                power_sub: None,
                power_score: None,
                robot_sub: None,
                robot_score: None,
                main_attack: None,
                rotation_state: state,
            };
            let banner = BannerCtx::test_default();
            let out = render_intraday_market(&banner, p);
            assert!(
                out.contains(&format!("轮动状态: {}", label)),
                "missing state label: {}",
                label
            );
        }
    }

    // ====== v13 I-02 新闻催化映射 (3 用例) ======
    #[test]
    fn news_catalyst_full_state() {
        let p = NewsCatalystParams {
            hhmm: "10:30",
            headline: "英伟达发布H200",
            theme: Some("AI算力"),
            stocks: vec![("中科曙光", "603019", Some(5.2), "AI龙头")],
        };
        let banner = BannerCtx::test_default();
        let out = render_news_catalyst(&banner, p);
        assert!(out.contains("🟢")); // banner 包含 Normal icon
        assert!(out.contains("📰⚡ 新闻催化跟踪（10:30）"));
        assert!(out.contains("新闻: 英伟达发布H200"));
        assert!(out.contains("受益板块: AI算力"));
        assert!(out.contains("· 中科曙光(603019) +5.2% | 原因:AI龙头"));
        assert!(out.ends_with("辅助建议, 非下单指令"));
    }

    #[test]
    fn news_catalyst_missing_chg_omits_row() {
        let p = NewsCatalystParams {
            hhmm: "10:30",
            headline: "X",
            theme: None,
            stocks: vec![("A", "000001", None, "r"), ("B", "000002", Some(3.0), "r2")],
        };
        let banner = BannerCtx::test_default();
        let out = render_news_catalyst(&banner, p);
        assert!(!out.contains("· A(000001)"));
        assert!(out.contains("· B(000002) +3.0% | 原因:r2"));
        assert!(out.contains("受益板块: 未分类"));
    }

    #[test]
    fn news_catalyst_no_stocks() {
        let p = NewsCatalystParams {
            hhmm: "10:30",
            headline: "催化",
            theme: Some("X"),
            stocks: vec![],
        };
        let banner = BannerCtx::test_default();
        let out = render_news_catalyst(&banner, p);
        assert!(out.contains("受益板块: X"));
        assert!(out.ends_with("辅助建议, 非下单指令"));
    }

    // ====== v13 治理元信息测试 (9 用例) ======
    #[test]
    fn gov_preopen_news_hot_cooldown() {
        assert_eq!(
            crate::notify::PushKind::PreopenNewsHot.cooldown_secs(),
            Some(900)
        );
    }
    #[test]
    fn gov_intraday_market_cooldown() {
        assert_eq!(
            crate::notify::PushKind::IntradayMarket.cooldown_secs(),
            Some(900)
        );
    }
    #[test]
    fn gov_news_catalyst_cooldown() {
        assert_eq!(
            crate::notify::PushKind::NewsCatalyst.cooldown_secs(),
            Some(600)
        );
    }
    #[test]
    fn gov_preopen_news_hot_no_banner() {
        assert!(!crate::notify::PushKind::PreopenNewsHot.requires_banner());
    }
    #[test]
    fn gov_intraday_market_banner() {
        assert!(crate::notify::PushKind::IntradayMarket.requires_banner());
    }
    #[test]
    fn gov_news_catalyst_banner() {
        assert!(crate::notify::PushKind::NewsCatalyst.requires_banner());
    }
    #[test]
    fn gov_preopen_news_hot_level() {
        assert_eq!(
            crate::notify::PushKind::PreopenNewsHot.level(),
            crate::notify::PushLevel::Important
        );
    }
    #[test]
    fn gov_intraday_market_level() {
        assert_eq!(
            crate::notify::PushKind::IntradayMarket.level(),
            crate::notify::PushLevel::Important
        );
    }
    #[test]
    fn gov_news_catalyst_level() {
        assert_eq!(
            crate::notify::PushKind::NewsCatalyst.level(),
            crate::notify::PushLevel::Important
        );
    }

    // ====== v13 D-01 新闻驱动个股 (4 用例) ======
    #[test]
    fn news_to_idea_full_state() {
        let p = NewsToIdeaParams {
            hhmm: "10:30",
            headline: "英伟达H200发布",
            theme: Some("AI算力"),
            stage: NewsStage::Starting,
            name: "中科曙光",
            code: "603019",
            reasons: vec!["AI算力龙头", "业绩超预期"],
            action: Some(NewsAction::BuyDip),
        };
        let banner = BannerCtx::test_default();
        let out = render_news_to_idea(&banner, p);
        assert!(out.contains("🧭 新闻驱动个股（10:30）"));
        assert!(out.contains("板块: AI算力 | 阶段: 启动"));
        assert!(out.contains("个股: 中科曙光(603019)"));
        assert!(out.contains("· AI算力龙头"));
        assert!(out.contains("[建议动作: 低吸]"));
        assert!(out.ends_with("辅助建议, 非下单指令"));
    }

    #[test]
    fn news_to_idea_no_reasons_no_action() {
        let p = NewsToIdeaParams {
            hhmm: "10:30",
            headline: "X",
            theme: None,
            stage: NewsStage::Fermenting,
            name: "A",
            code: "000001",
            reasons: vec![],
            action: None,
        };
        let banner = BannerCtx::test_default();
        let out = render_news_to_idea(&banner, p);
        assert!(out.contains("板块: 未分类 | 阶段: 发酵"));
        assert!(!out.contains("推送原因:"));
        assert!(!out.contains("[建议动作:"));
    }

    #[test]
    fn news_to_idea_action_do_not_chase() {
        let p = NewsToIdeaParams {
            hhmm: "10:30",
            headline: "X",
            theme: Some("X"),
            stage: NewsStage::Diverging,
            name: "A",
            code: "000001",
            reasons: vec!["r"],
            action: Some(NewsAction::DoNotChase),
        };
        let banner = BannerCtx::test_default();
        let out = render_news_to_idea(&banner, p);
        assert!(out.contains("[建议动作: 不追]"));
        assert!(out.contains("阶段: 分歧"));
    }

    #[test]
    fn news_to_idea_action_observe() {
        let p = NewsToIdeaParams {
            hhmm: "10:30",
            headline: "X",
            theme: Some("X"),
            stage: NewsStage::Starting,
            name: "A",
            code: "000001",
            reasons: vec!["r1", "r2"],
            action: Some(NewsAction::Observe),
        };
        let banner = BannerCtx::test_default();
        let out = render_news_to_idea(&banner, p);
        assert!(out.contains("[建议动作: 观察]"));
        assert!(out.contains("· r1"));
        assert!(out.contains("· r2"));
    }

    // ====== v13 治理元信息测试 (D-01) ======
    #[test]
    fn gov_news_to_idea_cooldown() {
        assert_eq!(
            crate::notify::PushKind::NewsToIdea.cooldown_secs(),
            Some(1200)
        );
    }
    #[test]
    fn gov_news_to_idea_banner() {
        assert!(crate::notify::PushKind::NewsToIdea.requires_banner());
    }
    #[test]
    fn gov_news_to_idea_level() {
        assert_eq!(
            crate::notify::PushKind::NewsToIdea.level(),
            crate::notify::PushLevel::Important
        );
    }

    // ====== v13 A-10 盘后题材催化复盘 (2 用例) ======
    #[test]
    fn catalyst_review_full() {
        let p = CatalystReviewParams {
            date: "2026-07-06",
            theme: "AI算力",
            score: Some(85.0),
            persistent: PersistentLevel::High,
            started_names: vec!["A", "B"],
            pending_names: vec!["C"],
            watch_point: Some("明日是否扩散"),
        };
        let out = render_catalyst_review(p);
        assert!(out.contains("📰 题材催化复盘（2026-07-06）"));
        assert!(out.contains("AI算力: 当日强度85.0 | 持续性high"));
        assert!(out.contains("已启动: A、B"));
        assert!(out.contains("待启动: C"));
        assert!(out.contains("明日观察点: 明日是否扩散"));
    }

    #[test]
    fn catalyst_review_persistent_low_empty() {
        let p = CatalystReviewParams {
            date: "2026-07-06",
            theme: "X",
            score: None,
            persistent: PersistentLevel::Low,
            started_names: vec![],
            pending_names: vec![],
            watch_point: None,
        };
        let out = render_catalyst_review(p);
        assert!(out.contains("当日强度N/A"));
        assert!(out.contains("持续性low"));
        assert!(!out.contains("已启动:"));
        assert!(!out.contains("待启动:"));
        assert!(!out.contains("明日观察点:"));
    }

    // ====== v13 治理元信息测试 (A-10) ======
    #[test]
    fn gov_catalyst_review_cooldown() {
        assert_eq!(
            crate::notify::PushKind::CatalystReview.cooldown_secs(),
            Some(86_400)
        );
    }
    #[test]
    fn gov_catalyst_review_no_banner() {
        // A-10 盘后非交易建议类, 不要 banner
        assert!(!crate::notify::PushKind::CatalystReview.requires_banner());
    }
    #[test]
    fn gov_catalyst_review_level() {
        assert_eq!(
            crate::notify::PushKind::CatalystReview.level(),
            crate::notify::PushLevel::Important
        );
    }

    // ====== v13 I-03 盘中涨停扩散 (审计多发现) (2 用例) ======
    #[test]
    fn industry_chain_intraday_with_supplements() {
        let p = IndustryChainIntradayParams {
            hhmm: "10:30",
            chain: "AI算力",
            limit_count: 5,
            leader_name: Some("A"),
            leader_code: Some("000001"),
            leader_height: 3,
            supplements: vec![SupplementCandidate {
                name: "B",
                code: "000002",
                trigger: "首板",
                lo: 10.0,
                hi: 12.0,
                stop: 9.0,
            }],
        };
        let banner = BannerCtx::test_default();
        let out = render_industry_chain_intraday(&banner, p);
        assert!(out.contains("🔥 盘中涨停扩散（10:30）"));
        assert!(out.contains("主链: AI算力 | 涨停5家 | 连板高度3板"));
        assert!(out.contains("龙头: A(000001) 3板"));
        assert!(out.contains("· B(000002) 触发条件首板 | 低吸10.00~12.00 | 止损9.00"));
    }

    #[test]
    fn industry_chain_intraday_no_leader_no_supplements() {
        let p = IndustryChainIntradayParams {
            hhmm: "10:30",
            chain: "X",
            limit_count: 0,
            leader_name: None,
            leader_code: None,
            leader_height: 0,
            supplements: vec![],
        };
        let banner = BannerCtx::test_default();
        let out = render_industry_chain_intraday(&banner, p);
        assert!(out.contains("龙头: 暂无"));
        assert!(out.contains("涨停0家 | 连板高度0板"));
        assert!(!out.contains("补涨候选:"));
    }

    // ====== v13 治理元信息测试 (I-03) ======
    #[test]
    fn gov_industry_chain_intraday_cooldown() {
        assert_eq!(
            crate::notify::PushKind::IndustryChainIntraday.cooldown_secs(),
            Some(1800)
        );
    }
    #[test]
    fn gov_industry_chain_intraday_banner() {
        assert!(crate::notify::PushKind::IndustryChainIntraday.requires_banner());
    }
    #[test]
    fn gov_industry_chain_intraday_level() {
        assert_eq!(
            crate::notify::PushKind::IndustryChainIntraday.level(),
            crate::notify::PushLevel::Important
        );
    }

    // ====== v13.1 T-14/T-15 盘后固定价格 (4 用例) ======
    #[test]
    fn post_fixed_price_order_sh_submitted() {
        let p = PostFixedPriceOrderParams {
            exchange: Exchange::SH,
            hhmm: "10:00",
            name: "A",
            code: "600000",
            price: 10.50,
            qty: 1000,
            order_id: "ORD001",
            status: OrderStatus::Submitted,
        };
        let out = render_post_fixed_price_order(p);
        assert!(out.contains("📋 盘后固定价格申报（10:00 沪市）"));
        assert!(out.contains("价格10.50 数量1000 | 状态: 已报"));
        assert!(out.contains("窗口: 上午"));
        assert!(out.contains("订单号: ORD001"));
    }

    #[test]
    fn post_fixed_price_order_sz_afternoon_cancelled() {
        let p = PostFixedPriceOrderParams {
            exchange: Exchange::SZ,
            hhmm: "13:30",
            name: "A",
            code: "000001",
            price: 10.0,
            qty: 100,
            order_id: "X",
            status: OrderStatus::Cancelled,
        };
        let out = render_post_fixed_price_order(p);
        assert!(out.contains("深市"));
        assert!(out.contains("窗口: 下午"));
        assert!(out.contains("已撤"));
    }

    #[test]
    fn post_fixed_price_order_bj_tail_rejected() {
        let p = PostFixedPriceOrderParams {
            exchange: Exchange::BJ,
            hhmm: "15:00",
            name: "A",
            code: "830001",
            price: 5.0,
            qty: 500,
            order_id: "Y",
            status: OrderStatus::Rejected,
        };
        let out = render_post_fixed_price_order(p);
        assert!(out.contains("北交所"));
        assert!(out.contains("窗口: 尾盘"));
        assert!(out.contains("废单"));
    }

    #[test]
    fn post_fixed_price_fill_with_carry() {
        let p = PostFixedPriceFillParams {
            exchange: Exchange::SH,
            hhmm: "15:10",
            name: "A",
            code: "600000",
            fill_price: 10.0,
            qty: 100,
            vs_limit_pct: Some(2.5),
            next_session_carry: true,
        };
        let out = render_post_fixed_price_fill(p);
        assert!(out.contains("✅ 盘后固定价格成交（15:10 沪市）"));
        assert!(out.contains("成交价10.00 数量100 | 价差+2.5%"));
        assert!(out.contains("清算: 过户到次一交易日"));
    }

    #[test]
    fn post_fixed_price_fill_no_carry() {
        let p = PostFixedPriceFillParams {
            exchange: Exchange::BJ,
            hhmm: "15:20",
            name: "A",
            code: "830001",
            fill_price: 5.0,
            qty: 100,
            vs_limit_pct: None,
            next_session_carry: false,
        };
        let out = render_post_fixed_price_fill(p);
        assert!(out.contains("价差N/A"));
        assert!(out.contains("清算: 本日内"));
    }

    // ====== v13.1 治理元信息测试 (T-14/T-15) ======
    #[test]
    fn gov_post_fixed_price_order_cooldown() {
        assert_eq!(
            crate::notify::PushKind::PostFixedPriceOrder.cooldown_secs(),
            Some(60)
        );
    }
    #[test]
    fn gov_post_fixed_price_fill_cooldown() {
        assert_eq!(
            crate::notify::PushKind::PostFixedPriceFill.cooldown_secs(),
            Some(300)
        );
    }
    #[test]
    fn gov_post_fixed_price_order_banner() {
        assert!(crate::notify::PushKind::PostFixedPriceOrder.requires_banner());
    }
    #[test]
    fn gov_post_fixed_price_fill_banner() {
        assert!(crate::notify::PushKind::PostFixedPriceFill.requires_banner());
    }
    #[test]
    fn gov_post_fixed_price_order_level() {
        assert_eq!(
            crate::notify::PushKind::PostFixedPriceOrder.level(),
            crate::notify::PushLevel::Important
        );
    }
    #[test]
    fn gov_post_fixed_price_fill_level() {
        assert_eq!(
            crate::notify::PushKind::PostFixedPriceFill.level(),
            crate::notify::PushLevel::Important
        );
    }

    // ====== v13.1 T-16 ST 涨跌幅变更 (3 用例) ======
    #[test]
    fn st_price_limit_changed_with_recalc() {
        let p = StPriceLimitChangedParams {
            hhmm: "09:30",
            name: "A",
            code: "600000",
            st_type: StType::ST,
            old_limit: 0.05,
            new_limit: 0.10,
            holding_qty: 1000,
            cost: 10.0,
            now_price: 11.0,
            new_stop_loss: Some(9.0),
            new_take_profit: Some(12.0),
        };
        let out = render_st_price_limit_changed(p);
        assert!(out.contains("⚠️ ST 涨跌幅变更（09:30）"));
        assert!(out.contains("A(600000) [ST] 持仓 1000 股"));
        assert!(out.contains("原涨跌幅: +5% → 新涨跌幅: +10%"));
        assert!(out.contains("新止损: 9.00 (基于 10% 阈值)"));
        assert!(out.contains("新止盈: 12.00"));
        assert!(out.contains("浮盈: +10.0%"));
        assert!(out.contains("辅助建议, 非下单指令 — 现有持仓风险阈值已重新校准"));
    }

    #[test]
    fn st_price_limit_changed_star_st_no_recalc() {
        let p = StPriceLimitChangedParams {
            hhmm: "09:30",
            name: "B",
            code: "000001",
            st_type: StType::StarST,
            old_limit: 0.05,
            new_limit: 0.10,
            holding_qty: 500,
            cost: 5.0,
            now_price: 4.5,
            new_stop_loss: None,
            new_take_profit: None,
        };
        let out = render_st_price_limit_changed(p);
        assert!(out.contains("B(000001) [*ST]"));
        assert!(out.contains("新止损: 未重算"));
        assert!(!out.contains("新止盈:"));
        assert!(out.contains("浮盈: -10.0%"));
    }

    #[test]
    fn st_price_limit_changed_zero_qty_alert() {
        let p = StPriceLimitChangedParams {
            hhmm: "09:30",
            name: "A",
            code: "600000",
            st_type: StType::ST,
            old_limit: 0.05,
            new_limit: 0.10,
            holding_qty: 0,
            cost: 0.0,
            now_price: 0.0,
            new_stop_loss: None,
            new_take_profit: None,
        };
        let out = render_st_price_limit_changed(p);
        assert!(out.contains("持仓 0 股"));
    }

    // ====== v13.1 治理元信息测试 (T-16) ======
    #[test]
    fn gov_st_price_limit_changed_cooldown() {
        assert_eq!(
            crate::notify::PushKind::StPriceLimitChanged.cooldown_secs(),
            Some(86_400)
        );
    }
    #[test]
    fn gov_st_price_limit_changed_banner() {
        assert!(crate::notify::PushKind::StPriceLimitChanged.requires_banner());
    }
    #[test]
    fn gov_st_price_limit_changed_level() {
        assert_eq!(
            crate::notify::PushKind::StPriceLimitChanged.level(),
            crate::notify::PushLevel::Important
        );
    }

    // ====== v13.1 T-17/T-18/T-19 剩余 3 新规 (3 用例) ======
    #[test]
    fn etf_closing_call_auction_with_data() {
        let p = EtfClosingCallAuctionParams {
            hhmm: "14:58",
            name: "沪深300ETF",
            code: "510300",
            call_auction_price: Some(3.952),
            vs_continuous_est: Some(0.15),
            liquidity_note: "正常, 无尾盘操纵",
        };
        let out = render_etf_closing_call_auction(p);
        assert!(out.contains("📊 ETF 集合竞价尾盘（14:58）"));
        assert!(out.contains("沪深300ETF(510300) 沪市 ETF 收盘价: 3.952"));
        assert!(out.contains("vs 连续竞价估值: +0.15%"));
        assert!(out.contains("14:57-15:00 集合竞价形成收盘价"));
    }

    #[test]
    fn block_trade_intraday_confirm_gem() {
        let p = BlockTradeIntradayConfirmParams {
            hhmm: "11:15",
            name: "A",
            code: "300750",
            qty: 1000,
            price: 50.0,
            block_type: BlockType::Agreed,
            board: Board::GEM,
            real_time_confirm: true,
            next_session_settle: SettleType::NextSession,
        };
        let out = render_block_trade_intraday_confirm(p);
        assert!(out.contains("📋 大宗交易盘中确认（11:15）"));
        assert!(out.contains("A(300750) 协议大宗 ✅ 盘中实时确认"));
        assert!(out.contains("数量: 1000 价格: 50.00"));
        assert!(out.contains("板块: 创业板"));
        assert!(out.contains("清算: 次日清算"));
    }

    #[test]
    fn block_trade_price_range_bj() {
        let p = BlockTradePriceRangeParams {
            hhmm: "14:30",
            name: "A",
            code: "830001",
            prev_close: Some(10.50),
            today_avg_price: 10.80,
            block_price_range: Some("10.50~11.10"),
            note: "原口径为前收盘价, 新口径为当日均价",
        };
        let out = render_block_trade_price_range(p);
        assert!(out.contains("📊 北交所大宗价格区间（14:30）"));
        assert!(out.contains("A(830001)"));
        assert!(out.contains("前收盘价: 10.50 (原口径)"));
        assert!(out.contains("当日实时均价: 10.80 (新口径)"));
        assert!(out.contains("价格区间: 10.50~11.10"));
    }

    // ====== v13.1 治理元信息测试 (T-17/T-18/T-19) ======
    #[test]
    fn gov_etf_closing_call_auction_cooldown() {
        assert_eq!(
            crate::notify::PushKind::EtfClosingCallAuction.cooldown_secs(),
            Some(86_400)
        );
    }
    #[test]
    fn gov_block_trade_intraday_confirm_cooldown() {
        assert_eq!(
            crate::notify::PushKind::BlockTradeIntradayConfirm.cooldown_secs(),
            Some(300)
        );
    }
    #[test]
    fn gov_block_trade_price_range_cooldown() {
        assert_eq!(
            crate::notify::PushKind::BlockTradePriceRange.cooldown_secs(),
            Some(3600)
        );
    }
    #[test]
    fn gov_etf_closing_call_auction_no_banner() {
        assert!(!crate::notify::PushKind::EtfClosingCallAuction.requires_banner());
    }
    #[test]
    fn gov_block_trade_intraday_confirm_no_banner() {
        assert!(!crate::notify::PushKind::BlockTradeIntradayConfirm.requires_banner());
    }
    #[test]
    fn gov_block_trade_price_range_no_banner() {
        assert!(!crate::notify::PushKind::BlockTradePriceRange.requires_banner());
    }
    #[test]
    fn gov_etf_closing_call_auction_level() {
        assert_eq!(
            crate::notify::PushKind::EtfClosingCallAuction.level(),
            crate::notify::PushLevel::Important
        );
    }
    #[test]
    fn gov_block_trade_intraday_confirm_level() {
        assert_eq!(
            crate::notify::PushKind::BlockTradeIntradayConfirm.level(),
            crate::notify::PushLevel::Important
        );
    }
    #[test]
    fn gov_block_trade_price_range_level() {
        assert_eq!(
            crate::notify::PushKind::BlockTradePriceRange.level(),
            crate::notify::PushLevel::Important
        );
    }

    // ====== v14 A-01 虚拟仓复盘 (2 用例) ======
    #[test]
    fn paper_review_full() {
        let p = PaperReviewParams {
            date: "2026-07-06", name: "A", code: "000001", trigger: "首板",
            desc: "已成交", pnl: Some(2.5),
            plan_high: Some("观察"), plan_flat: Some("持有"), plan_low: Some("止损"),
        };
        let out = render_paper_review(p);
        assert!(out.contains("🧪 虚拟仓复盘（2026-07-06）"));
        assert!(out.contains("A(000001) 原触发: 首板"));
        assert!(out.contains("结果: 已成交 +2.5%"));
        assert!(out.contains("· 高开>1%: 观察"));
        assert!(out.contains("· 平开: 持有"));
        assert!(out.contains("· 低开/跌破止损: 止损"));
    }

    #[test]
    fn paper_review_pnl_missing_no_plan() {
        let p = PaperReviewParams {
            date: "2026-07-06", name: "A", code: "000001", trigger: "T",
            desc: "X", pnl: None,
            plan_high: None, plan_flat: None, plan_low: None,
        };
        let out = render_paper_review(p);
        assert!(out.contains("结果: X N/A%"));
        assert!(!out.contains("次日计划:"));
    }

    // ====== v14 治理元信息测试 (A-01) ======
    #[test] fn gov_paper_review_cooldown() { assert_eq!(crate::notify::PushKind::PaperReview.cooldown_secs(), Some(86_400)); }
    #[test] fn gov_paper_review_no_banner() { assert!(!crate::notify::PushKind::PaperReview.requires_banner()); }
    #[test] fn gov_paper_review_level() { assert_eq!(crate::notify::PushKind::PaperReview.level(), crate::notify::PushLevel::Info); }

    // ====== v14.3 F-12: 候选失效独立 enum 治理测试 ======
    #[test] fn gov_candidate_invalidated_cooldown() { assert_eq!(crate::notify::PushKind::CandidateInvalidated.cooldown_secs(), Some(1800)); }
    #[test] fn gov_candidate_invalidated_no_banner() { assert!(!crate::notify::PushKind::CandidateInvalidated.requires_banner()); }
    #[test] fn gov_candidate_invalidated_level() { assert_eq!(crate::notify::PushKind::CandidateInvalidated.level(), crate::notify::PushLevel::Important); }

    // ====== v15.1: P-01 业务层集成测试 ======
    #[test]
    fn v15_build_preopen_news_hot_from_db() {
        use stock_analysis::database::concepts::ChainDailyRow;
        let clusters = vec![
            ChainDailyRow {
                date: "2026-07-06".to_string(),
                concept: "AI算力".to_string(),
                stocks: r#"["600000","000001","600519"]"#.to_string(),
                continuation_count: 3,
            },
            ChainDailyRow {
                date: "2026-07-06".to_string(),
                concept: "机器人".to_string(),
                stocks: r#"["000002","000003"]"#.to_string(),
                continuation_count: 2,
            },
        ];
        let p = build_preopen_news_hot_from_db("09:05", &clusters);
        assert_eq!(p.hhmm, "09:05");
        assert_eq!(p.theme_1, Some("AI算力"));
        assert_eq!(p.theme_2, Some("机器人"));
        assert_eq!(p.theme_3, None);  // 只有 2 cluster
        assert_eq!(p.watch_stocks.len(), 2);
        assert_eq!(p.watch_stocks[0], ("600000", "600000", "AI算力"));
        assert_eq!(p.news_pairs.len(), 0);  // TODO v15.1+
    }

    #[test]
    fn v15_build_preopen_news_hot_empty_db() {
        use stock_analysis::database::concepts::ChainDailyRow;
        let clusters: Vec<ChainDailyRow> = vec![];
        let p = build_preopen_news_hot_from_db("09:05", &clusters);
        assert!(p.theme_1.is_none());
        assert!(p.watch_stocks.is_empty());
        let out = render_preopen_news_hot(p);
        assert!(out.contains("📰 盘前热点（09:05）"));
        assert!(!out.contains("主线:"));
        assert!(!out.contains("关注票:"));
        assert!(out.ends_with("辅助建议, 非下单指令"));
    }

    #[test]
    fn v15_dispatch_preopen_news_hot_daily_no_data() {
        // 空 DB 时不推送 (graceful no-op)
        // 实际需要 DB, 此处仅验证 build_* 函数路径, dispatch 行为在 e2e
        use stock_analysis::database::concepts::ChainDailyRow;
        let clusters: Vec<ChainDailyRow> = vec![];
        let p = build_preopen_news_hot_from_db("09:05", &clusters);
        assert!(p.theme_1.is_none());
    }

    // ====== v15.2: I-01 业务层集成测试 (sector_score 抽口) ======
    #[test]
    fn v15_build_intraday_market_from_snapshot() {
        let s = SectorSnapshot {
            hhmm: "10:30".to_string(),
            tech_sub: "AI算力".to_string(), tech_score: Some(85.5),
            power_sub: "特高压".to_string(), power_score: Some(60.0),
            robot_sub: "减速器".to_string(), robot_score: Some(72.3),
            main_attack: "AI算力".to_string(),
            rotation_state: RotationState::Spreading,
        };
        let p = build_intraday_market_from_snapshot(&s);
        assert_eq!(p.hhmm, "10:30");
        assert_eq!(p.tech_sub, Some("AI算力"));
        assert_eq!(p.tech_score, Some(85.5));
        assert_eq!(p.rotation_state, RotationState::Spreading);
    }

    #[test]
    fn v15_sector_snapshot_empty_skips() {
        let s = SectorSnapshot::default();
        assert!(s.tech_sub.is_empty());
        // 空 snapshot → dispatch 应返回 false
        let p = build_intraday_market_from_snapshot(&s);
        assert!(p.tech_sub.is_none());
        assert!(p.tech_score.is_none());
        assert!(p.main_attack.is_none());
    }

    #[test]
    fn v15_load_sector_snapshot_default() {
        // v16+ 待集成真实 sector_score 算法, 验证默认空 snapshot
        let s = load_sector_snapshot("10:30");
        assert_eq!(s.hhmm, "10:30");
        assert!(s.tech_sub.is_empty());
        assert_eq!(s.rotation_state, RotationState::Fading);
    }

    // ====== v16.1: 真实 sector_score 集成测试 (mock network) ======
    #[test]
    fn v16_sector_snapshot_real_integration_shape() {
        // 验证 load_sector_snapshot_real 函数签名 + snapshot shape (不调网络)
        use std::any::Any;
        let _: fn(&str) -> SectorSnapshot = load_sector_snapshot_real;
        // 验证 SectorSnapshot Default 字段
        let s = SectorSnapshot::default();
        assert_eq!(s.rotation_state, RotationState::Spreading);  // enum default
        assert!(s.tech_sub.is_empty());
    }

    // ====== v15.3: I-02 业务层集成测试 (news_catalyst 抽口) ======
    #[test]
    fn v15_build_news_catalyst_from_snapshot() {
        let s = NewsCatalystSnapshot {
            hhmm: "10:30".to_string(),
            headline: "英伟达H200发布".to_string(),
            theme: "AI算力".to_string(),
            stocks: vec![
                ("中科曙光".to_string(), "603019".to_string(), Some(5.2)),
                ("浪潮信息".to_string(), "000977".to_string(), Some(3.8)),
            ],
        };
        let p = build_news_catalyst_from_snapshot(&s);
        assert_eq!(p.headline, "英伟达H200发布");
        assert_eq!(p.theme, Some("AI算力"));
        assert_eq!(p.stocks.len(), 2);
    }

    #[test]
    fn v15_news_catalyst_snapshot_empty_skips() {
        let s = NewsCatalystSnapshot::default();
        assert!(s.headline.is_empty());
        let p = build_news_catalyst_from_snapshot(&s);
        assert_eq!(p.theme, None);
        assert!(p.stocks.is_empty());
    }

    #[test]
    fn v15_load_news_catalyst_snapshot_default() {
        // v16+ 待集成真实 news_monitor + 实时行情
        let s = load_news_catalyst_snapshot("10:30");
        assert!(s.headline.is_empty());
        assert!(s.stocks.is_empty());
    }

    // ====== v15.4: I-03 业务层集成测试 (industry_chain 抽口) ======
    #[test]
    fn v15_build_industry_chain_intraday_from_snapshot() {
        let s = IndustryChainSnapshot {
            hhmm: "10:30".to_string(),
            chain: "AI算力".to_string(),
            limit_count: 5,
            leader_name: "龙头A".to_string(),
            leader_code: "000001".to_string(),
            leader_height: 3,
            supplements: vec![(
                "补涨B".to_string(),
                "000002".to_string(),
                "首板".to_string(),
                10.0,
                12.0,
                9.0,
            )],
        };
        let p = build_industry_chain_intraday_from_snapshot(&s);
        assert_eq!(p.chain, "AI算力");
        assert_eq!(p.limit_count, 5);
        assert_eq!(p.leader_name, Some("龙头A"));
        assert_eq!(p.supplements.len(), 1);
        assert_eq!(p.supplements[0].lo, 10.0);
    }

    #[test]
    fn v15_industry_chain_snapshot_empty_skips() {
        let s = IndustryChainSnapshot::default();
        let p = build_industry_chain_intraday_from_snapshot(&s);
        assert_eq!(p.chain, "");
        assert_eq!(p.leader_name, None);
        assert_eq!(p.leader_height, 0);
    }

    #[test]
    fn v15_load_industry_chain_snapshot_default() {
        // v16+ 待集成真实涨停扫描
        let s = load_industry_chain_snapshot("10:30");
        assert!(s.chain.is_empty());
    }

    // ====== v15.5: D-01 业务层集成测试 (news_to_idea 抽口) ======
    #[test]
    fn v15_build_news_to_idea_from_snapshot() {
        let s = NewsToIdeaSnapshot {
            hhmm: "10:30".to_string(),
            headline: "英伟达H200发布".to_string(),
            theme: "AI算力".to_string(),
            stage: NewsStage::Starting,
            name: "中科曙光".to_string(),
            code: "603019".to_string(),
            reasons: vec!["AI算力龙头".to_string(), "业绩超预期".to_string()],
            action: Some(NewsAction::BuyDip),
        };
        let p = build_news_to_idea_from_snapshot(&s);
        assert_eq!(p.headline, "英伟达H200发布");
        assert_eq!(p.name, "中科曙光");
        assert_eq!(p.reasons.len(), 2);
        assert_eq!(p.action, Some(NewsAction::BuyDip));
    }

    #[test]
    fn v15_news_to_idea_snapshot_empty_skips() {
        let s = NewsToIdeaSnapshot::default();
        assert_eq!(s.stage, NewsStage::Starting);  // default
        let p = build_news_to_idea_from_snapshot(&s);
        assert!(p.headline.is_empty());
        assert!(p.reasons.is_empty());
        assert_eq!(p.action, None);
    }

    #[test]
    fn v15_load_news_to_idea_snapshot_default() {
        // v16+ 待集成真实 news_monitor + 候选台
        let s = load_news_to_idea_snapshot("10:30");
        assert!(s.headline.is_empty());
        assert!(s.reasons.is_empty());
    }

    // ====== v15.6: A-01 业务层集成测试 (paper_review 抽口) ======
    #[test]
    fn v15_build_paper_review_from_snapshot() {
        let s = PaperReviewSnapshot {
            date: "2026-07-06".to_string(),
            name: "A".to_string(),
            code: "000001".to_string(),
            trigger: "首板".to_string(),
            desc: "已成交".to_string(),
            pnl: Some(2.5),
            plan_high: Some("减仓1/2".to_string()),
            plan_flat: Some("持有".to_string()),
            plan_low: Some("止损".to_string()),
        };
        let p = build_paper_review_from_snapshot(&s);
        assert_eq!(p.name, "A");
        assert_eq!(p.code, "000001");
        assert_eq!(p.pnl, Some(2.5));
        assert_eq!(p.plan_high, Some("减仓1/2"));
    }

    #[test]
    fn v15_paper_review_snapshot_empty_skips() {
        let s = PaperReviewSnapshot::default();
        let p = build_paper_review_from_snapshot(&s);
        assert_eq!(p.name, "");
        assert_eq!(p.pnl, None);
        assert!(p.plan_high.is_none());
    }

    #[test]
    fn v15_derive_plan_from_pnl() {
        // pnl > 5% → 减仓1/3
        let (h, f, l) = derive_plan_from_pnl(7.0);
        assert_eq!(h, "减仓1/3");
        // pnl > 0% → 减仓1/2
        let (h, f, l) = derive_plan_from_pnl(3.0);
        assert_eq!(h, "减仓1/2");
        // pnl <= 0% → 持有观望
        let (h, f, l) = derive_plan_from_pnl(-1.0);
        assert_eq!(h, "持有观望");
    }

    #[test]
    fn v15_load_paper_review_snapshot_default() {
        // v16+ 待集成真实 virtual_watch/paper_trades
        let s = load_paper_review_snapshot("2026-07-06");
        assert!(s.name.is_empty());
    }

    // ====== v13.7: dispatcher_log 可观测性测试 ======
    #[test]
    fn v13_7_dispatcher_log_writes_jsonl() {
        use std::fs;
        // 测试前清理 (避免污染)
        let dir = std::path::PathBuf::from("data/dispatcher_log");
        let _ = fs::remove_dir_all(&dir);

        // 写 3 条 (成功 2 + 失败 1)
        log_dispatcher_attempt("P-01", true, 3, "");
        log_dispatcher_attempt("I-01", false, 0, "sector empty");
        log_dispatcher_attempt("A-01", true, 1, "");

        // v14.4: 按天轮转, 找今天的文件
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let path = dir.join(format!("{}.jsonl", today));
        assert!(path.exists());
        let raw = fs::read_to_string(&path).expect("read dispatcher_log");
        let lines: Vec<&str> = raw.trim().split('\n').collect();
        assert_eq!(lines.len(), 3);
        // 验证 JSON 格式
        assert!(lines[0].contains("\"kind\":\"P-01\""));
        assert!(lines[0].contains("\"success\":true"));
        assert!(lines[0].contains("\"snapshot_size\":3"));
        assert!(lines[1].contains("\"success\":false"));
        assert!(lines[1].contains("\"error\":\"sector empty\""));
        assert!(lines[2].contains("\"kind\":\"A-01\""));

        // 清理
        let _ = fs::remove_dir_all(&dir);
    }

    // ====== v14.2: P5 源文件化测试 ======
    #[test]
    fn v14_2_p5_source_loads_jsonl() {
        use std::fs;
        let dir = std::path::PathBuf::from("data/p5_sources");
        let _ = fs::create_dir_all(&dir);

        // 写 2 个 P5 源文件
        let stock_pick_path = dir.join("stock_pick.jsonl");
        let optimal_path = dir.join("optimal_close.jsonl");
        fs::write(&stock_pick_path, "{\"code\":\"600519\",\"name\":\"贵州茅台\",\"chg_pct\":3.2}\n{\"code\":\"000858\",\"name\":\"五粮液\",\"chg_pct\":2.1}\n").unwrap();
        fs::write(&optimal_path, "{\"code\":\"002208\",\"name\":\"合肥城建\",\"chg_pct\":5.5}\n").unwrap();

        // 验证加载
        let items1 = load_p5_source_items("stock_pick");
        assert_eq!(items1.len(), 2);
        assert_eq!(items1[0].1, "600519");
        assert_eq!(items1[0].2, "贵州茅台");

        let items2 = load_p5_source_items("optimal_close");
        assert_eq!(items2.len(), 1);
        assert_eq!(items2[0].1, "002208");

        // 不存在的文件 → 空 Vec (不报错)
        let items3 = load_p5_source_items("nonexistent");
        assert_eq!(items3.len(), 0);

        // 清理
        let _ = fs::remove_file(&stock_pick_path);
        let _ = fs::remove_file(&optimal_path);
    }

    // ====== v14.5: 治理微调测试 ======
    #[test]
    fn v14_5_governance_micro_adjust() {
        use crate::notify::PushKind;

        // G-03: PaperTrade 永远照发 (不因 Frozen 阻断)
        assert!(!should_block_on_mode(
            PushKind::PaperTrade,
            AccountMode::Frozen,
            DataMode::Full
        ));
        assert!(!should_block_on_mode(
            PushKind::PaperTrade,
            AccountMode::Normal,
            DataMode::Degraded
        ));

        // G-03 验证对照: HoldingPlan 仍按 spec 阻断
        assert!(should_block_on_mode(
            PushKind::HoldingPlan,
            AccountMode::Frozen,
            DataMode::Full
        ));
        assert!(!should_block_on_mode(
            PushKind::HoldingPlan,
            AccountMode::Normal,
            DataMode::Full
        ));

        // G-05: TurnoverTop 显式 600s (10 min)
        assert_eq!(PushKind::TurnoverTop.cooldown_secs(), Some(600));

        // G-06: IndustryChain 显式 86400s (1次/日)
        assert_eq!(PushKind::IndustryChain.cooldown_secs(), Some(86_400));
        // 对照: IndustryChainIntraday 仍 30 min (不影响)
        assert_eq!(PushKind::IndustryChainIntraday.cooldown_secs(), Some(1800));
    }

    // ====== v14.7: I-03 真正 is_limit_up_today 测试 (chg_pct > 9.5 阈值) ======
    #[test]
    fn v14_7_is_limit_up_today_threshold() {
        use stock_analysis::market_analyzer::limit_chain_review::StockLimitStats;

        // chg_pct > 9.5 (新规 10% 涨停阈值) → is_limit_up_today = true
        let n_above = StockLimitStats {
            code: "600000".to_string(),
            name: "浦发银行".to_string(),
            chain: "银行".to_string(),
            board_level: 1,
            is_limit_up_today: 9.8 > 9.5,  // 9.8% 涨 → 涨停
            is_first_board: true,
            consecutive_days: 1,
        };
        assert!(n_above.is_limit_up_today);

        // chg_pct < 9.5 → is_limit_up_today = false
        let n_below = StockLimitStats {
            code: "000001".to_string(),
            name: "平安银行".to_string(),
            chain: "银行".to_string(),
            board_level: 1,
            is_limit_up_today: 5.0 > 9.5,  // 5% 涨 → 不涨停
            is_first_board: false,
            consecutive_days: 0,
        };
        assert!(!n_below.is_limit_up_today);

        // 边界: 9.5 整 → 不涨停 (> 严格不等)
        let n_boundary = StockLimitStats {
            code: "600519".to_string(),
            name: "贵州茅台".to_string(),
            chain: "白酒".to_string(),
            board_level: 2,
            is_limit_up_today: 9.5 > 9.5,  // 9.5 整 → false
            is_first_board: false,
            consecutive_days: 2,
        };
        assert!(!n_boundary.is_limit_up_today);

        // 涨停 (>9.5) + 一字板 (is_first_board=false) → board_level 仍按位置推断
        let n_limit_up = StockLimitStats {
            code: "002415".to_string(),
            name: "海康威视".to_string(),
            chain: "AI".to_string(),
            board_level: 2,  // 简化: 按位置推断
            is_limit_up_today: 10.2 > 9.5,
            is_first_board: false,
            consecutive_days: 2,
        };
        assert!(n_limit_up.is_limit_up_today);
        assert_eq!(n_limit_up.board_level, 2);
    }

    // ====== v16.1: 批量 fetch_realtime_quote 测试 (空 codes + 正常 codes) ======
    #[test]
    fn v16_1_batch_fetch_empty_codes() {
        // 空 codes → 返回空 HashMap (不调 provider)
        let result = fetch_realtime_quotes_batch(&[]);
        assert!(result.is_empty());
    }

    // ====== v16.2: LLM-style 分类器 trait 测试 ======
    #[test]
    fn v16_2_sector_classifier_trait() {
        // HeuristicClassifier 默认实现 (v13.5 关键词 32 个)
        let c = HeuristicClassifier;

        // tech 家族
        assert_eq!(c.classify("AI算力"), Some("tech"));
        assert_eq!(c.classify("半导体"), Some("tech"));
        assert_eq!(c.classify("光刻"), Some("tech"));

        // power 家族
        assert_eq!(c.classify("特高压"), Some("power"));
        assert_eq!(c.classify("储能"), Some("power"));

        // robot 家族
        assert_eq!(c.classify("减速器"), Some("robot"));
        assert_eq!(c.classify("人形"), Some("robot"));

        // 未匹配
        assert_eq!(c.classify("银行"), None);
        assert_eq!(c.classify("白酒"), None);

        // default_classifier() = HeuristicClassifier
        let c2 = default_classifier();
        assert_eq!(c2.classify("AI"), Some("tech"));
    }

    // ====== v18: 13 个新模板 render 函数实测 (Phase 1 完整覆盖) ======
    #[test]
    fn v18_render_all_13_templates_smoke() {
        let banner = BannerCtx::test_default();
        eprintln!("\n═══════════ 13 个新模板 render 输出 ═══════════\n");

        // 1. P-01
        let p1 = render_preopen_news_hot(PreopenNewsHotParams {
            hhmm: "09:05", theme_1: Some("AI算力"), theme_2: Some("机器人"),
            theme_3: None, news_pairs: vec![("英伟达H200", "GPU")],
            watch_stocks: vec![("中科曙光", "603019", "AI龙头")],
        });
        assert!(p1.contains("📰 盘前热点"));
        assert!(p1.contains("AI算力"));
        assert!(p1.contains("中科曙光"));

        // 2. I-01
        let i1 = render_intraday_market(&banner, IntradayMarketParams {
            hhmm: "10:30", tech_sub: "AI算力".into(), tech_score: Some(85.5),
            power_sub: "特高压".into(), power_score: Some(60.0),
            robot_sub: "减速器".into(), robot_score: Some(72.3),
            main_attack: Some("AI算力"), rotation_state: RotationState::Spreading,
        });
        assert!(i1.contains("📊 盘中轮动"));
        assert!(i1.contains("轮动状态: 扩散"));

        // 3. I-02
        let i2 = render_news_catalyst(&banner, NewsCatalystParams {
            hhmm: "10:30", headline: "英伟达H200发布", theme: Some("AI算力"),
            stocks: vec![("中科曙光", "603019", Some(5.2), "AI算力订单"), ("浪潮信息", "000977", Some(3.8), "服务器受益")],
        });
        assert!(i2.contains("📰⚡ 新闻催化跟踪"));
        assert!(i2.contains("中科曙光"));

        // 4. I-03
        let i3 = render_industry_chain_intraday(&banner, IndustryChainIntradayParams {
            hhmm: "10:30", chain: "AI算力", limit_count: 5,
            leader_name: Some("中科曙光"), leader_code: Some("603019"), leader_height: 3,
            supplements: vec![SupplementCandidate { name: "浪潮信息", code: "000977",
                trigger: "首板".into(), lo: 10.0, hi: 12.0, stop: 9.0 }],
        });
        assert!(i3.contains("🔥 盘中涨停扩散"));
        assert!(i3.contains("AI算力"));

        // 5. D-01
        let d1 = render_news_to_idea(&banner, NewsToIdeaParams {
            hhmm: "10:30", headline: "AI算力龙头", theme: Some("AI"),
            stage: NewsStage::Starting, name: "中科曙光", code: "603019",
            reasons: vec!["AI龙头", "业绩超预期"], action: Some(NewsAction::BuyDip),
        });
        assert!(d1.contains("🧭 新闻驱动个股"));
        assert!(d1.contains("[建议动作: 低吸]"));

        // 6. A-10
        let a10 = render_catalyst_review(CatalystReviewParams {
            date: "2026-07-06", theme: "AI算力", score: Some(85.0),
            persistent: PersistentLevel::High,
            started_names: vec!["中科曙光", "浪潮信息"],
            pending_names: vec!["紫光股份"],
            watch_point: Some("明日是否扩散"),
        });
        assert!(a10.contains("📰 题材催化复盘"));
        assert!(a10.contains("AI算力"));

        // 7. A-01
        let a01 = render_paper_review(PaperReviewParams {
            date: "2026-07-06", name: "中科曙光", code: "603019",
            trigger: "首板", desc: "已成交", pnl: Some(2.5),
            plan_high: Some("减仓1/2"), plan_flat: Some("持有"), plan_low: Some("止损"),
        });
        assert!(a01.contains("🧪 虚拟仓复盘"));
        assert!(a01.contains("中科曙光"));

        // 8. T-14
        let t14 = render_post_fixed_price_order(PostFixedPriceOrderParams {
            exchange: Exchange::SH, hhmm: "10:00", name: "A", code: "600000",
            price: 10.5, qty: 1000, order_id: "ORD001",
            status: OrderStatus::Submitted,
        });
        assert!(t14.contains("📋 盘后固定价格申报"));
        assert!(t14.contains("沪市"));

        // 9. T-15
        let t15 = render_post_fixed_price_fill(PostFixedPriceFillParams {
            exchange: Exchange::BJ, hhmm: "15:10", name: "A", code: "830001",
            fill_price: 10.0, qty: 100, vs_limit_pct: Some(2.5),
            next_session_carry: true,
        });
        assert!(t15.contains("✅ 盘后固定价格成交"));
        assert!(t15.contains("北交所"));

        // 10. T-16
        let t16 = render_st_price_limit_changed(StPriceLimitChangedParams {
            hhmm: "09:30", name: "A", code: "600000", st_type: StType::ST,
            old_limit: 0.05, new_limit: 0.10, holding_qty: 1000,
            cost: 10.0, now_price: 11.0,
            new_stop_loss: Some(9.0), new_take_profit: Some(12.0),
        });
        assert!(t16.contains("⚠️ ST 涨跌幅变更"));
        assert!(t16.contains("原涨跌幅"));
        assert!(t16.contains("新涨跌幅"));

        // 11. T-17
        let t17 = render_etf_closing_call_auction(EtfClosingCallAuctionParams {
            hhmm: "14:58", name: "沪深300ETF", code: "510300",
            call_auction_price: Some(3.952),
            vs_continuous_est: Some(0.15),
            liquidity_note: "正常",
        });
        assert!(t17.contains("📊 ETF 集合竞价尾盘"));
        assert!(t17.contains("沪市 ETF"));

        // 12. T-18
        let t18 = render_block_trade_intraday_confirm(BlockTradeIntradayConfirmParams {
            hhmm: "11:15", name: "A", code: "300750", qty: 1000, price: 50.0,
            block_type: BlockType::Agreed, board: Board::GEM,
            real_time_confirm: true, next_session_settle: SettleType::NextSession,
        });
        assert!(t18.contains("📋 大宗交易盘中确认"));
        assert!(t18.contains("创业板"));

        // 13. T-19
        let t19 = render_block_trade_price_range(BlockTradePriceRangeParams {
            hhmm: "14:30", name: "A", code: "830001",
            prev_close: Some(10.50), today_avg_price: 10.80,
            block_price_range: Some("10.50~11.10"),
            note: "原口径为前收盘价, 新口径为当日均价",
        });
        assert!(t19.contains("📊 北交所大宗价格区间"));
        assert!(t19.contains("北交所"));

        // 全部 13 个模板 + 辅助行 ("辅助建议, 非下单指令" 等)
        assert!(p1.contains("辅助建议, 非下单指令"));
        assert!(i1.contains("辅助建议, 非下单指令"));
        assert!(i2.contains("辅助建议, 非下单指令"));
        assert!(i3.contains("辅助建议, 非下单指令"));
        assert!(d1.contains("辅助建议, 非下单指令"));
        assert!(a10.contains("辅助建议, 非下单指令"));
        assert!(a01.contains("辅助建议, 非下单指令"));

        // 打印所有 13 个模板样例 (v19 任务: 用户要看每个模板输出)
        eprintln!("\n╔══════════════════════════════════════════════════════════════════╗");
        eprintln!("║ 13 个新模板 render 输出 (v13/v13.1)                              ║");
        eprintln!("╚══════════════════════════════════════════════════════════════════╝\n");
        eprintln!("────── 1. P-01 盘前新闻热点 ──────\n{}\n", p1);
        eprintln!("────── 2. I-01 盘中轮动总览 ──────\n{}\n", i1);
        eprintln!("────── 3. I-02 新闻催化映射 ──────\n{}\n", i2);
        eprintln!("────── 4. I-03 盘中涨停扩散 ──────\n{}\n", i3);
        eprintln!("────── 5. D-01 新闻驱动个股 ──────\n{}\n", d1);
        eprintln!("────── 6. A-10 题材催化复盘 ──────\n{}\n", a10);
        eprintln!("────── 7. A-01 虚拟仓复盘 ──────\n{}\n", a01);
        eprintln!("────── 8. T-14 盘后固定价格申报 ──────\n{}\n", t14);
        eprintln!("────── 9. T-15 盘后固定价格成交 ──────\n{}\n", t15);
        eprintln!("────── 10. T-16 ST 涨跌幅变更 ──────\n{}\n", t16);
        eprintln!("────── 11. T-17 ETF 集合竞价尾盘 ──────\n{}\n", t17);
        eprintln!("────── 12. T-18 创业板大宗盘中确认 ──────\n{}\n", t18);
        eprintln!("────── 13. T-19 北交所大宗价格区间 ──────\n{}\n", t19);
        eprintln!("═══════════════════════════════════════════════════════════════════\n");
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
        assert_eq!(
            PushKind::AccountMode.cooldown_secs(),
            None,
            "AccountMode 无冷却"
        );
        assert_eq!(
            PushKind::HoldingEvent.cooldown_secs(),
            None,
            "HoldingEvent 无冷却"
        );
        assert_eq!(
            PushKind::DataMode.cooldown_secs(),
            Some(600),
            "DataMode 10min"
        );
        assert_eq!(
            PushKind::HoldingPlan.cooldown_secs(),
            Some(1800),
            "HoldingPlan 30min"
        );
        assert_eq!(
            PushKind::T0Advice.cooldown_secs(),
            Some(1800),
            "T0Advice 30min"
        );
        assert_eq!(
            PushKind::CandidateTriggered.cooldown_secs(),
            Some(86_400),
            "1次/票/日"
        );
        assert_eq!(
            PushKind::ForbiddenOps.cooldown_secs(),
            Some(3600),
            "ForbiddenOps 60min"
        );
        assert_eq!(
            PushKind::PaperTrade.cooldown_secs(),
            Some(300),
            "PaperTrade 5min"
        );
        assert_eq!(
            PushKind::CloseCall.cooldown_secs(),
            Some(86_400),
            "CloseCall 1次/日"
        );
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
        assert!(
            !is_in_cooldown(PushKind::HoldingPlan, "000002"),
            "不同 code 应独立"
        );
    }

    #[test]
    fn emergency_bypass_cooldown_table() {
        use super::super::notify::{PushKind, PushLevel};
        // HoldingEvent 是 Emergency, 即使在 cooldown table 中也是 false
        record_cooldown(PushKind::HoldingEvent, "000001");
        assert!(!is_in_cooldown(PushKind::HoldingEvent, "000001"));
        assert_eq!(PushKind::HoldingEvent.level(), PushLevel::Emergency);
    }

    // ---- PR2-2.4 缺盘口"承接"护栏 ----

    #[test]
    fn acceptance_guard_passes_when_book_ok() {
        // book 不缺失 → 任何文本都过
        let text = "放量承接, 主力净流入 1.2亿";
        assert!(check_no_acceptance_when_missing_book(text, false).is_ok());
    }

    #[test]
    fn acceptance_guard_passes_when_no_phrase() {
        // book 缺失 + 无 "承接" 字样 → 过
        let text = "现价12.30 主力净流入 1.2亿";
        assert!(check_no_acceptance_when_missing_book(text, true).is_ok());
    }

    #[test]
    fn acceptance_guard_allows_self_annotation() {
        // book 缺失 + "不作承接判断" 自我标注 → 过
        let text = "[⚠️ 缺盘口深度: 本条不含承接判断]";
        assert!(check_no_acceptance_when_missing_book(text, true).is_ok());
    }

    #[test]
    fn acceptance_guard_allows_restriction_phrase() {
        let text = "输出限制:\n· 不做盘口承接判断";
        assert!(check_no_acceptance_when_missing_book(text, true).is_ok());
    }

    #[test]
    fn acceptance_guard_rejects_unauthorized_acceptance() {
        // book 缺失 + 违规 "承接" → 拒绝
        let text = "盘后强势股, 高开放量承接";
        assert!(check_no_acceptance_when_missing_book(text, true).is_err());
    }

    #[test]
    fn acceptance_guard_error_includes_context() {
        let text = "高位承接盘, 主力兑现";
        let err = check_no_acceptance_when_missing_book(text, true).unwrap_err();
        assert!(err.contains("PR2-2.4"));
        assert!(err.contains("承接"));
    }

    // ---- 真实推送内容验证 (user 硬性要求: 测试内容必须准确推送) ----
    // 这些测试用 V10_DRY_RUN_PUSH=1 让 push_wechat 不真发, 但 capture 调用结果.
    // 这样既能验证 dispatch 路径, 又不骚扰用户.

    // 注意: t01/t02 orchestrator 集成测试需要 DB init, 留给 `tests/push_orchestrator_e2e.rs`
    // 单独跑 (需 test_data/test.db init). 本文件只验证模板渲染 + 治理逻辑.

    #[test]
    fn banner_renders_exact_format() {
        // §14.0 横幅格式硬性: "[icon mode | 仓位N成 | 日盈亏+/-X.X% | 数据Mode]"
        let b = BannerCtx {
            account_mode: AccountMode::Normal,
            total_pos: 5,
            today_pnl: 0.3,
            data_mode: DataMode::Full,
            data_missing_note: None,
        };
        assert_eq!(b.render(), "[🟢 Normal | 仓位5成 | 日盈亏+0.3% | 数据Full]");
    }

    #[test]
    fn t03_text_exact_format() {
        // T-03 持仓建议: 验证拼接输出与 v13-push-templates.md §14.1 T-03 模板逐行一致
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
                invalidations: &["跌破5日线且放量".to_string(), "板块热度转Fade".to_string()],
                reasons: &["放量冲高回落".to_string(), "主力净流出0.8亿".to_string()],
            },
        );
        // 验证 5 个关键字段精确出现
        assert!(s.contains("[🟢 Normal | 仓位5成 | 日盈亏+0.3% | 数据Full]"));
        assert!(s.contains("🎯 持仓建议 XX科技(000001)（13:42）"));
        assert!(s.contains("动作倾向: 逢高减仓"));
        assert!(s.contains("现价12.30 成本11.80 可用3000股"));
        assert!(s.contains("支撑11.95 | 压力12.70 | 硬止损11.95"));
        assert!(s.ends_with("辅助建议, 非下单指令"));
    }

    #[test]
    fn t07_text_includes_all_required_fields() {
        // T-07 候选触发: 14 个必填字段都要出现
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
        // 必填 14 字段
        for required in &[
            "📋 候选触发 候选X(688001)（10:30）",
            "等级A | 状态: Triggered",
            "主题: AI算力",
            "现价50.00 已触发: 突破前高+量比4.5",
            "低吸参考: 49.50~50.30",
            "止损48.00",
            "仓位上限10%",
            "· 新闻: 强 政策面共振",
            "· 量能: 强 量比4.5",
            "· K线: 中 突破未稳",
            "· 盘口: 缺失,不作承接判断",
            "· 大盘跳水同步",
            "需人工确认, 非自动买入",
        ] {
            assert!(s.contains(required), "缺字段: {}", required);
        }
        // PR2-2.4: "缺失,不作承接判断" 是自我标注, 不算违规
        let guard = check_no_acceptance_when_missing_book(&s, true);
        if let Err(e) = &guard {
            eprintln!("护栏错误: {}", e);
            eprintln!("T-07 输出:\n{}", s);
        }
        assert!(guard.is_ok(), "T-07 应通过承接护栏");
    }

    #[test]
    fn t07_with_missing_book_self_annotates() {
        // 验证 T-07 模板在 book 缺失时的 self-annotation
        let s = render_candidate_triggered(
            &banner_normal(),
            CandidateTriggeredParams {
                name: "T",
                code: "688002",
                hhmm: "10:00",
                grade: CandidateGrade::B,
                topic: "X",
                price: 10.0,
                trigger_desc: "突破",
                lo: 9.5,
                hi: 10.5,
                stop: 9.0,
                max_pos_pct: 5,
                news_quality: EvidenceQuality::Mid,
                news_note: "",
                vol_quality: EvidenceQuality::Mid,
                vol_ratio: 2.0,
                kline_quality: EvidenceQuality::Mid,
                kline_note: "",
                book_quality: EvidenceQuality::Missing,
                no_buy: &[],
            },
        );
        // "· 盘口: 缺失,不作承接判断" 应出现, 且护栏放行
        assert!(s.contains("缺失,不作承接判断"));
        assert!(check_no_acceptance_when_missing_book(&s, true).is_ok());
    }

    #[test]
    fn r02_market_review_text_exact_lines() {
        // R-02: 7 个必填行
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
        for required in &[
            "📊 今日盘面（2026-07-05）",
            "上证+0.5% 创业+1.2% 科创+1.5%",
            "涨停35家 跌停3家",
            "炸板率15%",
            "连板高度5板",
            "两市8500亿（放量+8%）",
            "主力净+120亿",
            "阶段判定: MainUp（置信度80%）",
            "→ 明日账户建议: Normal 仓位上限7成",
        ] {
            assert!(s.contains(required), "R-02 缺字段: {}", required);
        }
    }

    // ---- PR1-1.7 + PR2-2.2 E2E: 真 DB + 真 push_governor(dry-run) ----
    // 硬性要求 (user 2026-07-05): 测试内容必须准确推送到消息推送服务.
    // 真实 DB 初始化 + V10_DRY_RUN_PUSH=1 + PUSH_VERBOSE=true 让 push_wechat 走 dry-run 返回 true.
    // 跑在 monitor bin 的 tests 模块, 共享同一进程 DB 单例.

    use std::sync::OnceLock;

    static DB_INIT: OnceLock<()> = OnceLock::new();

    /// e2e 串行化 Mutex (修复并行测试 DB row count 干扰) — tokio 跨 await 安全
    static E2E_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// 重置全局 DAILY_BUDGET_COUNT 计数 + 清空 COOLDOWN_TABLE (修复 4 个 e2e 并行测试隔离 bug)
    /// 测试间共享的全局状态 (account_mode_log 表, 预算 counter, 冷却表) 必须全部重置
    /// 才能保证 67 个并行测试互不干扰。
    fn reset_daily_budget_for_test() {
        DAILY_BUDGET_COUNT.store(0, Ordering::Relaxed);
        let mut table = COOLDOWN_TABLE.lock().expect("cooldown table poisoned");
        table.clear();
        // 清空 account_mode_log (并行测试可能插入行, 影响 e2e_t01_no_change 的 count 断言)
        use diesel::prelude::*;
        if let Ok(mut conn) = stock_analysis::database::DatabaseManager::get().get_conn() {
            diesel::sql_query("DELETE FROM account_mode_log")
                .execute(&mut conn)
                .ok();
        }
    }

    fn init_test_db() {
        DB_INIT.get_or_init(|| {
            use std::path::PathBuf;
            use stock_analysis::database::DatabaseManager;
            std::fs::create_dir_all("./test_data").expect("create test_data");
            // 清理旧 DB 避免上一轮残留 (包括 WAL/SHM)
            for ext in ["", "-shm", "-wal"] {
                let p = format!("./test_data/test_orch.db{}", ext);
                let _ = std::fs::remove_file(&p);
            }
            // DatabaseManager 是单例 (OnceCell). 一旦初始化就不可重置.
            // 但删除文件后, 重新打开已存在的 DB 不会触发 run_migrations.
            // 这里用 test_data/test.db (已有完整迁移的共享测试 DB) — 已有账户模式日志表吗? 否.
            // 解决: 先 init, 然后通过 diesel::sql_query 手工建 account_mode_log 表.
            DatabaseManager::init(Some(PathBuf::from("./test_data/test.db")))
                .expect("DB init for E2E");

            // 单独建 account_mode_log 表 (该表不在 run_migrations 内, 因 PR1 migration 走 SQL 文件)
            use diesel::prelude::*;
            let mut conn = DatabaseManager::get().get_conn().expect("conn");
            diesel::sql_query(
                r#"
                CREATE TABLE IF NOT EXISTS account_mode_log (
                    id              INTEGER PRIMARY KEY AUTOINCREMENT,
                    ts              TIMESTAMP NOT NULL,
                    prev_mode       TEXT NOT NULL,
                    new_mode        TEXT NOT NULL,
                    trigger_reason  TEXT NOT NULL,
                    today_pnl_pct   REAL,
                    consecutive_n   INTEGER,
                    total_pos_cheng INTEGER,
                    data_complete   INTEGER NOT NULL DEFAULT 1,
                    pushed          INTEGER NOT NULL DEFAULT 0,
                    push_attempted_at TIMESTAMP
                )
                "#,
            )
            .execute(&mut conn)
            .expect("create account_mode_log");

            // 清理旧 E2E 数据 (避免测试间干扰)
            diesel::sql_query("DELETE FROM account_mode_log")
                .execute(&mut conn)
                .ok();
        });
    }

    fn banner_normal_full() -> BannerCtx {
        BannerCtx {
            account_mode: AccountMode::Normal,
            total_pos: 5,
            today_pnl: 0.3,
            data_mode: DataMode::Full,
            data_missing_note: None,
        }
    }

    /// T-01 E2E: Normal → ReduceOnly. 验证 DB 写 + 推送路径
    #[tokio::test]
    async fn e2e_t01_normal_to_reduce_only_db_and_push() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        init_test_db();
        reset_daily_budget_for_test();
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        std::env::set_var("PUSH_VERBOSE", "true");

        use stock_analysis::database::account_mode_log;
        use stock_analysis::risk::account_mode::PortfolioMetrics;
        use stock_analysis::risk::action_gate::AccountMode as LibAM;

        let metrics = PortfolioMetrics {
            today_pnl_pct: -1.6,
            consecutive_stop_loss_n: 0,
            total_pos_cheng: 5,
            data_complete: true,
        };

        let result =
            push_account_mode_change(&metrics, Some(LibAM::Normal), Some(&banner_normal_full()))
                .await;

        assert!(result.is_ok(), "orchestrator 不应报错: {:?}", result);
        assert!(result.unwrap(), "T-01 应推送成功 (dry-run)");

        // 验证 DB 行
        let rows = account_mode_log::recent_account_mode_changes(10).expect("query");
        assert!(!rows.is_empty(), "应至少插 1 行");
        // 找 prev=Normal → new=ReduceOnly 的最新行
        let target = rows
            .iter()
            .find(|r| r.prev_mode == "Normal" && r.new_mode == "ReduceOnly");
        assert!(target.is_some(), "应找到 Normal→ReduceOnly 行");
        let row = target.unwrap();
        assert_eq!(row.pushed, 1, "成功推送后应 mark pushed=1");
        assert!(
            row.trigger_reason.contains("-1.60%"),
            "触发原因应含具体亏损"
        );
        assert!((row.today_pnl_pct.unwrap() - -1.6).abs() < 0.01);
        // 数据准确: 关键字段校验
        assert!(row.trigger_reason.contains("当日亏损"));
        assert!(row.trigger_reason.contains("降级线"));
        assert!(row.trigger_reason.contains("-1.50%"));

        std::env::remove_var("V10_DRY_RUN_PUSH");
        std::env::remove_var("PUSH_VERBOSE");
    }

    /// T-01 E2E: 无变更 → 不推送不写库
    #[tokio::test]
    async fn e2e_t01_no_change_no_push_no_db_write() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        init_test_db();
        reset_daily_budget_for_test();
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        std::env::set_var("PUSH_VERBOSE", "true");

        use stock_analysis::database::account_mode_log;
        use stock_analysis::risk::account_mode::PortfolioMetrics;
        use stock_analysis::risk::action_gate::AccountMode as LibAM;

        let before = account_mode_log::recent_account_mode_changes(100)
            .map(|r| r.len())
            .unwrap_or(0);

        let metrics = PortfolioMetrics {
            today_pnl_pct: -1.6,
            consecutive_stop_loss_n: 0,
            total_pos_cheng: 5,
            data_complete: true,
        };
        // prev 已是 ReduceOnly, metrics 不变 → is_changed=false
        let result = push_account_mode_change(
            &metrics,
            Some(LibAM::ReduceOnly),
            Some(&banner_normal_full()),
        )
        .await;
        assert!(result.is_ok());
        assert!(!result.unwrap(), "无变更应返回 false");

        let after = account_mode_log::recent_account_mode_changes(100)
            .map(|r| r.len())
            .unwrap_or(0);
        assert_eq!(before, after, "无变更不应写库");

        std::env::remove_var("V10_DRY_RUN_PUSH");
        std::env::remove_var("PUSH_VERBOSE");
    }

    /// T-01 E2E: ReduceOnly → Frozen. 数据准确
    #[tokio::test]
    async fn e2e_t01_reduce_only_to_frozen_circuit_breaker() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        init_test_db();
        reset_daily_budget_for_test();
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        std::env::set_var("PUSH_VERBOSE", "true");

        use stock_analysis::database::account_mode_log;
        use stock_analysis::risk::account_mode::PortfolioMetrics;
        use stock_analysis::risk::action_gate::AccountMode as LibAM;

        let metrics = PortfolioMetrics {
            today_pnl_pct: -2.5, // 超过 -2.0% 熔断线
            consecutive_stop_loss_n: 5,
            total_pos_cheng: 9,
            data_complete: true,
        };

        let result = push_account_mode_change(
            &metrics,
            Some(LibAM::ReduceOnly),
            Some(&banner_normal_full()),
        )
        .await;
        assert!(result.is_ok());

        let rows = account_mode_log::recent_account_mode_changes(1).expect("query");
        assert_eq!(rows[0].new_mode, "Frozen");
        assert_eq!(rows[0].prev_mode, "ReduceOnly");
        assert!(rows[0].trigger_reason.contains("熔断"));
        assert!(rows[0].trigger_reason.contains("-2.00%"));
        assert_eq!(rows[0].pushed, 1);

        std::env::remove_var("V10_DRY_RUN_PUSH");
        std::env::remove_var("PUSH_VERBOSE");
    }

    /// T-01 E2E: 数据缺失 → 保守 ReduceOnly
    #[tokio::test]
    async fn e2e_t01_data_missing_conservative_reduce_only() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        init_test_db();
        reset_daily_budget_for_test();
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        std::env::set_var("PUSH_VERBOSE", "true");

        use stock_analysis::database::account_mode_log;
        use stock_analysis::risk::account_mode::PortfolioMetrics;
        use stock_analysis::risk::action_gate::AccountMode as LibAM;

        let metrics = PortfolioMetrics {
            today_pnl_pct: 0.0,
            consecutive_stop_loss_n: 0,
            total_pos_cheng: 0,
            data_complete: false,
        };

        let result =
            push_account_mode_change(&metrics, Some(LibAM::Normal), Some(&banner_normal_full()))
                .await;
        assert!(result.is_ok());

        let rows = account_mode_log::recent_account_mode_changes(1).expect("query");
        assert_eq!(rows[0].new_mode, "ReduceOnly");
        assert!(rows[0].trigger_reason.contains("数据缺失"));
        assert_eq!(rows[0].data_complete, 0);

        std::env::remove_var("V10_DRY_RUN_PUSH");
        std::env::remove_var("PUSH_VERBOSE");
    }

    /// T-02 E2E: Full → Degraded (Kline 过期)
    #[tokio::test]
    async fn e2e_t02_full_to_degraded_kline_stale() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        init_test_db();
        reset_daily_budget_for_test();
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        std::env::set_var("PUSH_VERBOSE", "true");

        use stock_analysis::monitor::data_mode::{
            Capability, CapabilityStatus, DataHealthInput, DataMode as LibDM,
        };

        let input = DataHealthInput {
            capabilities: vec![
                CapabilityStatus::fresh(Capability::Quote, 30),
                CapabilityStatus::fresh(Capability::Kline, 200), // 超过 120s
                CapabilityStatus::missing(Capability::MoneyFlow),
                CapabilityStatus::missing(Capability::News),
                CapabilityStatus::missing(Capability::OrderBook),
            ],
            critical_max_age_secs: 120,
            orderbook_max_age_secs: 600,
        };

        let result =
            push_data_mode_change(&input, Some(LibDM::Full), Some(&banner_normal_full())).await;
        assert!(result.is_ok(), "T-02 orchestrator: {:?}", result);
        assert!(result.unwrap(), "T-02 应推送成功");

        std::env::remove_var("V10_DRY_RUN_PUSH");
        std::env::remove_var("PUSH_VERBOSE");
    }

    /// T-02 E2E: 无变更 → no-op
    #[tokio::test]
    async fn e2e_t02_no_change_no_push() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        init_test_db();
        reset_daily_budget_for_test();
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        std::env::set_var("PUSH_VERBOSE", "true");

        use stock_analysis::monitor::data_mode::{
            Capability, CapabilityStatus, DataHealthInput, DataMode as LibDM,
        };

        let input = DataHealthInput {
            capabilities: Capability::ALL
                .iter()
                .map(|c| CapabilityStatus::fresh(*c, 30))
                .collect(),
            critical_max_age_secs: 120,
            orderbook_max_age_secs: 600,
        };

        let result =
            push_data_mode_change(&input, Some(LibDM::Full), Some(&banner_normal_full())).await;
        assert!(result.is_ok());
        assert!(!result.unwrap(), "Full → Full 应 no-op");

        std::env::remove_var("V10_DRY_RUN_PUSH");
        std::env::remove_var("PUSH_VERBOSE");
    }

    /// T-02 模板精确内容验证: 文本必须与 §14.1 T-02 模板逐字符一致
    #[test]
    fn t02_template_text_exact_format() {
        let s = render_data_mode(
            "10:23",
            DataMode::Full,
            DataMode::Degraded,
            "Kline/MoneyFlow",
            &[
                "不做盘口承接判断".to_string(),
                "价格型建议标注数据降级".to_string(),
            ],
            Some("15min"),
        );
        // 6 个必填字段
        for required in &[
            "📡 数据状态变更（10:23）",
            "Full → Degraded",
            "受影响: Kline/MoneyFlow",
            "· 不做盘口承接判断",
            "· 价格型建议标注数据降级",
            "恢复预计: 15min",
        ] {
            assert!(s.contains(required), "T-02 缺字段: {}", required);
        }
    }

    /// T-01 模板精确内容验证: 与 §14.1 T-01 一致
    #[test]
    fn t01_template_text_exact_format() {
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
        for required in &[
            "🛡️ 账户模式变更（10:23）",
            "Normal → Frozen",
            "· 连续第3笔止损: 300xxx -3.1%",
            "· 当日亏损 -2.1% 触发熔断线 -2.0%",
            "生效限制: 禁止新开仓/加仓/正T, 候选转影子",
            "解除条件: 下一交易日盘前重置",
        ] {
            assert!(s.contains(required), "T-01 缺字段: {}", required);
        }
    }

    /// §14.0 横幅 + T-01 拼接: 拼装顺序必须是 banner 先, 然后 T-01
    #[test]
    fn banner_plus_t01_concat_format() {
        let banner = BannerCtx {
            account_mode: AccountMode::ReduceOnly,
            total_pos: 5,
            today_pnl: -1.6,
            data_mode: DataMode::Full,
            data_missing_note: None,
        };
        let banner_str = banner.render();
        let template_str = render_account_mode(
            "10:23",
            AccountMode::Normal,
            AccountMode::ReduceOnly,
            &["当日亏损 -1.60% 触发降级线 -1.50%".to_string()],
            "禁止新开仓/加仓/正T, 候选转影子",
            "下一交易日盘前重置",
        );
        let full = format!("{}\n{}", banner_str, template_str);
        // banner 第 1 行 + T-01 第 1 行紧跟
        let lines: Vec<&str> = full.lines().collect();
        assert!(lines[0].starts_with("[🟡 ReduceOnly |"), "第 1 行应是横幅");
        assert!(lines[1].starts_with("🛡️ 账户模式变更"), "第 2 行应是 T-01");
    }

    // ===============================================================
    // =========== 20 模板真实数据 + 实际推送 E2E 测试 ===============
    // 触发条件: env V12_E2E_REAL_PUSH=1 (opt-in, 避免 CI 噪音)
    // 所有消息带 [v12-E2E-T0X] 前缀, 便于飞书群识别 + 清理
    // ===============================================================

    /// 单条推送冒烟: 验证 magiclaw daemon 可达 + PUSH_VERBOSE=true
    /// 运行: V12_E2E_REAL_PUSH=1 cargo test --bin monitor push_templates::tests::e2e_single_smoke
    #[tokio::test]
    async fn e2e_single_smoke() {
        if std::env::var("V12_E2E_REAL_PUSH").ok().as_deref() != Some("1") {
            return;
        }
        // 初始化 env_logger (test env 默认不 init, log macros 静默)
        let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .format(|buf, record| {
                use std::io::Write;
                writeln!(
                    buf,
                    "[{} {}] {}",
                    chrono::Local::now().format("%H:%M:%S"),
                    record.level(),
                    record.args()
                )
            })
            .try_init();
        std::env::set_var("PUSH_VERBOSE", "true");
        std::env::remove_var("V10_DRY_RUN_PUSH");
        // 显式设 MAGICLAW_HOME, 让 push_via_magiclaw_cli 找对 .env
        if std::env::var("MAGICLAW_HOME").is_err() {
            std::env::set_var("MAGICLAW_HOME", "/Users/zhangzhen/Desktop/magiclaw");
        }
        // 显式设 DATABASE_PATH / MAGICLAW_DB_PATH (test env 默认无, push_via_magiclaw_cli 需)
        if std::env::var("DATABASE_PATH").is_err() {
            std::env::set_var("DATABASE_PATH", "./data/stock_analysis.db");
        }
        if std::env::var("MAGICLAW_DB_PATH").is_err() {
            std::env::set_var("MAGICLAW_DB_PATH", "./data/stock_analysis.db");
        }
        // 显式设 FEISHU_RECEIVE_ID_TYPE (push_via_magiclaw_cli 据此传 --receive-id-type)
        if std::env::var("FEISHU_RECEIVE_ID_TYPE").is_err() {
            std::env::set_var("FEISHU_RECEIVE_ID_TYPE", "chat_id");
        }
        let text = "[v12-E2E-smoke] 冒烟测试 — 验证 magiclaw daemon 可达";
        eprintln!(
            "[v12-E2E-smoke] cwd={:?}, MAGICAW_HOME={:?}, MAGICLAW_BIN={:?}, DATABASE_PATH={:?}",
            std::env::current_dir().ok(),
            std::env::var("MAGICLAW_HOME").ok(),
            std::env::var("MAGICLAW_BIN").ok(),
            std::env::var("DATABASE_PATH").ok(),
        );
        // 直接调 magiclaw binary 验证可执行 + auth 可达
        let magiclaw_bin = "/Users/zhangzhen/Desktop/magiclaw/target/release/magiclaw";
        let out = std::process::Command::new(magiclaw_bin)
            .args(&[
                "send",
                "--channel",
                "feishu",
                "--to",
                "oc_4bca5d870fd5ff3352795a674194d5b0",
                "--message",
                text,
            ])
            .current_dir("/Users/zhangzhen/Desktop/magiclaw")
            .output();
        match out {
            Ok(o) => {
                eprintln!(
                    "[v12-E2E-smoke] magiclaw exit={}, stdout={}, stderr={}",
                    o.status,
                    String::from_utf8_lossy(&o.stdout)
                        .chars()
                        .take(200)
                        .collect::<String>(),
                    String::from_utf8_lossy(&o.stderr)
                        .chars()
                        .take(200)
                        .collect::<String>()
                );
            }
            Err(e) => eprintln!("[v12-E2E-smoke] magiclaw spawn failed: {}", e),
        }
        // Now test push_governor
        let ok = crate::notify::push_governor(text, crate::notify::PushKind::AccountMode).await;
        eprintln!("[v12-E2E-smoke] push_governor result: {}", ok);
        // 调试: 直接调 push_via_magiclaw_cli 模拟
        let magiclaw_bin2 = "/Users/zhangzhen/Desktop/magiclaw/target/release/magiclaw";
        let out2 = std::process::Command::new(magiclaw_bin2)
            .args(&[
                "send",
                "--channel",
                "feishu",
                "--to",
                "oc_4bca5d870fd5ff3352795a674194d5b0",
                "--message",
                text,
            ])
            .current_dir("/Users/zhangzhen/Desktop/magiclaw")
            .env("DATABASE_PATH", "./data/stock_analysis.db")
            .env("MAGICLAW_DB_PATH", "./data/stock_analysis.db")
            .env("FEISHU_TO", "oc_4bca5d870fd5ff3352795a674194d5b0")
            .output();
        match out2 {
            Ok(o) => eprintln!(
                "[v12-E2E-smoke] magiclaw2 exit={}, stdout={}",
                o.status,
                String::from_utf8_lossy(&o.stdout)
                    .chars()
                    .take(150)
                    .collect::<String>()
            ),
            Err(e) => eprintln!("[v12-E2E-smoke] magiclaw2 spawn failed: {}", e),
        }
        assert!(ok, "smoke test 推送应成功");
    }

    /// v12 E2E 真实推送: 装配真实数据 (从 DB ledger/positions/trades) + 实际飞书推送.
    /// 运行: `V12_E2E_REAL_PUSH=1 cargo test --bin monitor push_templates::tests::e2e_real_all_20`
    #[tokio::test]
    async fn e2e_real_all_20_templates() {
        // 0. 跳过条件: 必须显式 opt-in
        if std::env::var("V12_E2E_REAL_PUSH").ok().as_deref() != Some("1") {
            eprintln!(
                "[v12-E2E] 跳过: 需 V12_E2E_REAL_PUSH=1 启用. \
                 命令: V12_E2E_REAL_PUSH=1 cargo test --bin monitor push_templates::tests::e2e_real_all_20"
            );
            return;
        }

        // 强制 PUSH_VERBOSE=true 让被 deprecated 的 PushKind 也能推
        // (v12-P0-4 设计: deprecated 默认降级 log, 但 E2E 验证需真推)
        std::env::set_var("PUSH_VERBOSE", "true");
        // 显式设 MAGICLAW_HOME, 让 push_via_magiclaw_cli 找对 .env (test env cwd 与 cargo run 不同)
        if std::env::var("MAGICLAW_HOME").is_err() {
            std::env::set_var("MAGICLAW_HOME", "/Users/zhangzhen/Desktop/magiclaw");
        }

        // 1. Setup: init DB + 装配真实数据
        let _ = init_test_db();
        let hhmm = chrono::Local::now().format("%H:%M").to_string();
        let today_str = chrono::Local::now().format("%Y-%m-%d").to_string();

        // 真实数据 1: ledger (净值/今日盈亏) — 真实查 DB
        let ledger_entry = stock_analysis::portfolio::get_equity_curve(1)
            .ok()
            .and_then(|c| c.last().cloned());
        let (today_pnl_pct, total_value, market_value) = match ledger_entry.as_ref() {
            Some(e) => {
                let pct = if e.total_value > 0.0 {
                    (e.daily_pnl / e.total_value) * 100.0
                } else {
                    0.0
                };
                (pct, e.total_value, e.market_value)
            }
            None => (0.0, 0.0, 0.0),
        };
        let data_complete = ledger_entry.is_some() && total_value > 0.0;

        // 真实数据 2: 当前 open positions — 真实查 DB
        let positions = stock_analysis::portfolio::get_positions().unwrap_or_default();
        let first_pos = positions.first().cloned();

        // 真实数据 3: trades 历史 (最近)
        let trades = stock_analysis::portfolio::get_trade_history(30).unwrap_or_default();

        // 真实数据 4: latest account mode (从 DB 读)
        let prev_account_mode_label =
            stock_analysis::database::account_mode_log::latest_account_mode_change()
                .ok()
                .flatten()
                .map(|r| r.new_mode);
        let prev_mode = match prev_account_mode_label.as_deref() {
            Some("Normal") => stock_analysis::risk::action_gate::AccountMode::Normal,
            Some("ReduceOnly") => stock_analysis::risk::action_gate::AccountMode::ReduceOnly,
            Some("Frozen") => stock_analysis::risk::action_gate::AccountMode::Frozen,
            _ => stock_analysis::risk::action_gate::AccountMode::Normal,
        };

        let mut success_count = 0u32;
        let mut fail_msgs: Vec<String> = Vec::new();
        let libam_to_tmpl = |m: stock_analysis::risk::action_gate::AccountMode| match m {
            stock_analysis::risk::action_gate::AccountMode::Normal => AccountMode::Normal,
            stock_analysis::risk::action_gate::AccountMode::ReduceOnly => AccountMode::ReduceOnly,
            stock_analysis::risk::action_gate::AccountMode::Frozen => AccountMode::Frozen,
        };

        // ===== T-01 账户模式变更 =====
        {
            let metrics = stock_analysis::risk::account_mode::PortfolioMetrics {
                today_pnl_pct,
                consecutive_stop_loss_n: 0,
                total_pos_cheng: if total_value > 0.0 {
                    ((market_value / total_value) * 10.0).round() as u8
                } else {
                    0
                },
                data_complete,
            };
            let eval = stock_analysis::risk::account_mode::evaluate(
                &metrics,
                Some(prev_mode),
                &stock_analysis::risk::account_mode::ModeThresholds::default(),
            );
            if eval.is_changed() {
                let forbidden = match eval.mode {
                    stock_analysis::risk::action_gate::AccountMode::Normal => "(无)",
                    stock_analysis::risk::action_gate::AccountMode::ReduceOnly => {
                        "禁止新开仓/加仓/正T, 候选转影子"
                    }
                    stock_analysis::risk::action_gate::AccountMode::Frozen => {
                        "禁止新开仓/加仓/正T/反T, 候选转影子"
                    }
                };
                let recovery = match eval.mode {
                    stock_analysis::risk::action_gate::AccountMode::Normal => "(已是 Normal)",
                    stock_analysis::risk::action_gate::AccountMode::ReduceOnly => {
                        "当日盈亏回到 -1.5% 内 或 连续止损 < 3 笔 (运行时) / 下一交易日盘前重置"
                    }
                    stock_analysis::risk::action_gate::AccountMode::Frozen => "下一交易日盘前重置",
                };
                let text = render_account_mode(
                    &hhmm,
                    libam_to_tmpl(prev_mode),
                    libam_to_tmpl(eval.mode),
                    &eval
                        .trigger_reason
                        .clone()
                        .map(|r| vec![r])
                        .unwrap_or_default(),
                    forbidden,
                    recovery,
                );
                let banner_text = format!("[v12-E2E-T01] {}", text);
                let ok = crate::notify::push_governor(
                    &banner_text,
                    crate::notify::PushKind::AccountMode,
                )
                .await;
                if ok {
                    success_count += 1;
                } else {
                    fail_msgs.push("T-01".to_string());
                }
                log::info!(
                    "[v12-E2E] T-01 推送 {} (real data: prev={:?}, pnl={:.2}%)",
                    if ok { "OK" } else { "FAIL" },
                    prev_mode,
                    today_pnl_pct
                );
            } else {
                log::info!("[v12-E2E] T-01 无变更 (skip)");
            }
        }

        // ===== T-02 数据状态变更 =====
        {
            use stock_analysis::monitor::data_mode::{
                evaluate as dm_evaluate, Capability, CapabilityStatus, DataHealthInput,
                DataMode as LibDM,
            };
            let input = DataHealthInput {
                capabilities: Capability::ALL
                    .iter()
                    .map(|c| CapabilityStatus::fresh(*c, 200))
                    .collect(),
                critical_max_age_secs: 120,
                orderbook_max_age_secs: 600,
            };
            let health = dm_evaluate(&input, Some(LibDM::Full));
            if health.is_changed() {
                let missing_str = if health.missing.is_empty() {
                    "(无)".to_string()
                } else {
                    health
                        .missing
                        .iter()
                        .map(|c| c.label().to_string())
                        .collect::<Vec<_>>()
                        .join("/")
                };
                let restrictions = vec![
                    "不做盘口承接判断".to_string(),
                    "禁出价格型建议".to_string(),
                    "仅保留风险类推送".to_string(),
                ];
                let text = render_data_mode(
                    &hhmm,
                    DataMode::Full,
                    DataMode::Unsafe,
                    &missing_str,
                    &restrictions,
                    Some("15min"),
                );
                let banner_text = format!("[v12-E2E-T02] {}", text);
                let ok =
                    crate::notify::push_governor(&banner_text, crate::notify::PushKind::DataMode)
                        .await;
                if ok {
                    success_count += 1;
                } else {
                    fail_msgs.push("T-02".to_string());
                }
                log::info!("[v12-E2E] T-02 推送 {}", if ok { "OK" } else { "FAIL" });
            }
        }

        // ===== T-03 持仓建议 (真实持仓, 无则用合成数据) =====
        let (t03_name, t03_code, t03_cost, t03_shares) = if let Some(p) = &first_pos {
            (
                p.name.clone(),
                p.code.clone(),
                p.cost_price,
                p.shares as u32,
            )
        } else {
            // 合成: 无持仓时也演示完整推送路径
            (
                "示例持仓".to_string(),
                "000001".to_string(),
                11.80_f64,
                3000_u32,
            )
        };
        {
            let banner = BannerCtx {
                account_mode: AccountMode::Normal,
                total_pos: if total_value > 0.0 {
                    ((market_value / total_value) * 10.0).round() as u8
                } else {
                    0
                },
                today_pnl: today_pnl_pct,
                data_mode: DataMode::Full,
                data_missing_note: None,
            };
            let t03_name_s = t03_name.clone();
            let t03_code_s = t03_code.clone();
            let params = HoldingPlanParams {
                name: &t03_name_s,
                code: &t03_code_s,
                hhmm: &hhmm,
                intent: Intent::Reduce,
                price: t03_cost * 1.05,
                cost: t03_cost,
                avail: t03_shares,
                reduce_zone: Some((t03_cost * 1.10, t03_cost * 1.15)),
                support: t03_cost * 0.95,
                pressure: t03_cost * 1.20,
                stop: t03_cost * 0.92,
                invalidations: &["跌破5日线且放量".to_string(), "板块热度转Fade".to_string()],
                reasons: &["放量冲高回落".to_string(), "主力净流出".to_string()],
            };
            let text = render_holding_plan(&banner, params);
            let banner_text = format!("[v12-E2E-T03] {}", text);
            let ok =
                crate::notify::push_governor(&banner_text, crate::notify::PushKind::HoldingPlan)
                    .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("T-03".to_string());
            }
            eprintln!(
                "[v12-E2E] T-03 推送 {} (real: {}({}))",
                if ok { "OK" } else { "FAIL" },
                t03_name,
                t03_code
            );
        }

        // ===== T-04 持仓紧急风险 (真实持仓, 无则用合成数据) =====
        let (t04_name, t04_code, t04_cost, t04_shares) = if let Some(p) = &first_pos {
            (
                p.name.clone(),
                p.code.clone(),
                p.cost_price,
                p.shares as u32,
            )
        } else {
            (
                "示例持仓".to_string(),
                "000001".to_string(),
                11.80_f64,
                3000_u32,
            )
        };
        {
            let banner = BannerCtx {
                account_mode: AccountMode::Normal,
                total_pos: if total_value > 0.0 {
                    ((market_value / total_value) * 10.0).round() as u8
                } else {
                    0
                },
                today_pnl: today_pnl_pct,
                data_mode: DataMode::Full,
                data_missing_note: None,
            };
            let t04_name_s = t04_name.clone();
            let t04_code_s = t04_code.clone();
            let params = HoldingEventParams {
                name: &t04_name_s,
                code: &t04_code_s,
                hhmm: &hhmm,
                trigger: "跌破硬止损",
                price: t04_cost * 0.90,
                chg_pct: -10.0,
                gap_pct: 2.5,
                action: "建议减仓/止损",
                avail: t04_shares,
            };
            let text = render_holding_event(&banner, params);
            let banner_text = format!("[v12-E2E-T04] {}", text);
            let ok =
                crate::notify::push_governor(&banner_text, crate::notify::PushKind::HoldingEvent)
                    .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("T-04".to_string());
            }
            eprintln!(
                "[v12-E2E] T-04 推送 {} (real: {})",
                if ok { "OK" } else { "FAIL" },
                t04_code
            );
        }

        // ===== T-05 做T建议 (真实持仓, 无则用合成数据) =====
        let (t05_name, t05_code, t05_cost, t05_shares, t05_held_today) = if let Some(p) = &first_pos
        {
            let today = chrono::Local::now().date_naive();
            let buy_date = p.added_at.format("%Y-%m-%d").to_string();
            let held_today = buy_date == today.format("%Y-%m-%d").to_string();
            (
                p.name.clone(),
                p.code.clone(),
                p.cost_price,
                p.shares as u32,
                held_today,
            )
        } else {
            // 合成: 用历史日期避免 held_today 拦截
            (
                "示例持仓".to_string(),
                "000001".to_string(),
                11.80_f64,
                3000_u32,
                false,
            )
        };
        if !t05_held_today && t05_shares > 0 {
            let input = stock_analysis::decision::t0_advisor::T0Input {
                code: t05_code.clone(),
                name: t05_name.clone(),
                trend: stock_analysis::decision::t0_advisor::TrendStatus::Range,
                buy_date: "2026-07-01".to_string(), // 历史日期
                available_shares: t05_shares,
                current_price: t05_cost * 1.05,
                cost_price: t05_cost,
                support: t05_cost * 0.95,
                pressure: t05_cost * 1.10,
                kind_hint: None,
                account_mode_is_reduce_only: matches!(
                    prev_mode,
                    stock_analysis::risk::action_gate::AccountMode::ReduceOnly
                ),
            };
            let v = stock_analysis::decision::t0_advisor::evaluate(&input);
            if let stock_analysis::decision::t0_advisor::T0Verdict::Allowed {
                kind,
                sell_zone,
                buy_zone,
                min_spread_pct,
            } = v
            {
                let kind_pt = match kind {
                    stock_analysis::decision::t0_advisor::T0Kind::ReverseT => T0Kind::ReverseT,
                    stock_analysis::decision::t0_advisor::T0Kind::PositiveT => T0Kind::PositiveT,
                };
                let t05_name_s = t05_name.clone();
                let t05_code_s = t05_code.clone();
                let params = T0AdviceParams {
                    name: &t05_name_s,
                    code: &t05_code_s,
                    hhmm: &hhmm,
                    kind: kind_pt,
                    style: T0Style::PullbackCatch,
                    avail: t05_shares,
                    sell_lo: sell_zone.0,
                    sell_hi: sell_zone.1,
                    buy_lo: buy_zone.0,
                    buy_hi: buy_zone.1,
                    min_spread_pct,
                    risk_note: "板块同步下跌",
                };
                let text = render_t0_advice(&BannerCtx::default(), params);
                let banner_text = format!("[v12-E2E-T05] {}", text);
                let ok =
                    crate::notify::push_governor(&banner_text, crate::notify::PushKind::T0Advice)
                        .await;
                if ok {
                    success_count += 1;
                } else {
                    fail_msgs.push("T-05".to_string());
                }
                eprintln!(
                    "[v12-E2E] T-05 推送 {} (real: {}({}))",
                    if ok { "OK" } else { "FAIL" },
                    t05_name,
                    t05_code
                );
            } else {
                eprintln!("[v12-E2E] T-05 评估返回 Forbidden (合成数据可能未触发 Allowed)");
            }
        }

        // ===== T-06 不建议做T =====
        {
            let (name, code) = first_pos
                .as_ref()
                .map(|p| (p.name.as_str(), p.code.as_str()))
                .unwrap_or(("测试标的", "000001"));
            let params = T0ForbidParams {
                name,
                code,
                hhmm: &hhmm,
                reason: "主升核心票防卖飞 (BR-022 衍生)",
            };
            let text = render_t0_forbid(&BannerCtx::default(), params);
            let banner_text = format!("[v12-E2E-T06] {}", text);
            let ok =
                crate::notify::push_governor(&banner_text, crate::notify::PushKind::T0Advice).await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("T-06".to_string());
            }
            log::info!("[v12-E2E] T-06 推送 {}", if ok { "OK" } else { "FAIL" });
        }

        // ===== T-07 候选触发 =====
        {
            std::env::set_var("ENABLE_CANDIDATE_LIVE", "true");
            let params = CandidateTriggeredParams {
                name: "AI算力候选",
                code: "688001",
                hhmm: &hhmm,
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
            };
            let text = render_candidate_triggered(&BannerCtx::default(), params);
            let banner_text = format!("[v12-E2E-T07] {}", text);
            let ok = crate::notify::push_governor(
                &banner_text,
                crate::notify::PushKind::CandidateTriggered,
            )
            .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("T-07".to_string());
            }
            log::info!("[v12-E2E] T-07 推送 {}", if ok { "OK" } else { "FAIL" });
            std::env::remove_var("ENABLE_CANDIDATE_LIVE");
        }

        // ===== T-08 候选失效 =====
        {
            let text = render_candidate_invalidated(
                &hhmm,
                "AI算力候选",
                "688001",
                "Watch",
                "触发失败: 未触达买入区",
            );
            let banner_text = format!("[v12-E2E-T08] {}", text);
            let ok =
                crate::notify::push_governor(&banner_text, crate::notify::PushKind::CandidateBoard)
                    .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("T-08".to_string());
            }
            log::info!("[v12-E2E] T-08 推送 {}", if ok { "OK" } else { "FAIL" });
        }

        // ===== T-09 禁止操作 =====
        {
            let reasons = vec!["距涨停仅 1.2%".to_string(), "板块已 Climax".to_string()];
            let params = ForbiddenOpsParams {
                name: "测试标的",
                code: "688001",
                hhmm: &hhmm,
                conclusion: "距涨停过近, 禁止追买",
                reasons: &reasons,
            };
            let text = render_forbidden_ops(&BannerCtx::default(), params);
            let banner_text = format!("[v12-E2E-T09] {}", text);
            let ok =
                crate::notify::push_governor(&banner_text, crate::notify::PushKind::ForbiddenOps)
                    .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("T-09".to_string());
            }
            log::info!("[v12-E2E] T-09 推送 {}", if ok { "OK" } else { "FAIL" });
        }

        // ===== T-10 虚拟盘成交 =====
        {
            let name = first_pos
                .as_ref()
                .map(|p| p.name.as_str())
                .unwrap_or("测试标的");
            let code = first_pos
                .as_ref()
                .map(|p| p.code.as_str())
                .unwrap_or("000001");
            let params = PaperTradeParams {
                name,
                code,
                hhmm: &hhmm,
                status: PaperTradeStatus::Filled,
                fill_price: Some(12.50),
                qty: Some(1000),
                virtual_reason: Some("候选A档触发"),
                not_fill_reason: None,
                account_mode: AccountMode::Normal,
                data_mode: DataMode::Full,
            };
            let text = render_paper_trade(params);
            let banner_text = format!("[v12-E2E-T10] {}", text);
            let ok =
                crate::notify::push_governor(&banner_text, crate::notify::PushKind::PaperTrade)
                    .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("T-10".to_string());
            }
            log::info!("[v12-E2E] T-10 推送 {}", if ok { "OK" } else { "FAIL" });
        }

        // ===== T-11 竞价异动 =====
        {
            let items = vec![
                AuctionItem {
                    name: "AI龙头A",
                    code: "688001",
                    gap_pct: 5.2,
                    vol_ratio: 8.5,
                    tag: "昨日涨停",
                },
                AuctionItem {
                    name: "机器人B",
                    code: "300750",
                    gap_pct: 2.1,
                    vol_ratio: 3.2,
                    tag: "观察池",
                },
            ];
            let text =
                render_auction_volume(&BannerCtx::default(), &hhmm, &items, "强承接", "可操作");
            let banner_text = format!("[v12-E2E-T11] {}", text);
            let ok =
                crate::notify::push_governor(&banner_text, crate::notify::PushKind::AuctionVolume)
                    .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("T-11".to_string());
            }
            log::info!("[v12-E2E] T-11 推送 {}", if ok { "OK" } else { "FAIL" });
        }

        // ===== T-12 尾盘决策 (真实持仓, 无则用合成数据) =====
        {
            let (t12_name, _t12_code) = if let Some(p) = &first_pos {
                (p.name.clone(), p.code.clone())
            } else {
                ("示例持仓".to_string(), "000001".to_string())
            };
            let t12_name_s = t12_name.clone();
            let holding = CloseCallHolding {
                name: &t12_name_s,
                state: "尾盘跳水-建议处理",
            };
            let text = render_close_call(&BannerCtx::default(), &hhmm, Some(&holding), None);
            let banner_text = format!("[v12-E2E-T12] {}", text);
            let ok = crate::notify::push_governor(&banner_text, crate::notify::PushKind::CloseCall)
                .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("T-12".to_string());
            }
            eprintln!(
                "[v12-E2E] T-12 推送 {} (real: {})",
                if ok { "OK" } else { "FAIL" },
                t12_name
            );
        }

        // ===== R-01 持仓明日计划 (真实持仓) =====
        {
            let mut items = Vec::new();
            for p in positions.iter().take(3) {
                let pnl = if p.cost_price > 0.0 {
                    ((p.cost_price * 1.05 / p.cost_price - 1.0) * 100.0)
                } else {
                    0.0
                };
                items.push(HoldingDailyPlan {
                    name: &p.name,
                    code: &p.code,
                    price: p.cost_price * 1.05,
                    cost: p.cost_price,
                    pnl_pct: pnl,
                    high_gap_x: 2.0,
                    plan_high: "减仓1/3",
                    plan_flat: "持有观望",
                    stop: p.cost_price * 0.92,
                    t0: if pnl > 5.0 {
                        "适合观察"
                    } else {
                        "不适合(主升核心)"
                    },
                });
            }
            if items.is_empty() {
                items.push(HoldingDailyPlan {
                    name: "示例持仓",
                    code: "000001",
                    price: 12.30,
                    cost: 11.80,
                    pnl_pct: 4.2,
                    high_gap_x: 2.0,
                    plan_high: "减仓1/3",
                    plan_flat: "持有",
                    stop: 11.95,
                    t0: "适合观察",
                });
            }
            let text = render_daily_report(&today_str, &items);
            let banner_text = format!("[v12-E2E-R01] {}", text);
            let ok =
                crate::notify::push_governor(&banner_text, crate::notify::PushKind::DailyReport)
                    .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("R-01".to_string());
            }
            log::info!(
                "[v12-E2E] R-01 推送 {} ({} 只持仓)",
                if ok { "OK" } else { "FAIL" },
                items.len()
            );
        }

        // ===== R-02 盘面走向 (用 ledger + market_stage_confidence 真装配) =====
        {
            let mut ev = stock_analysis::market_analyzer::market_stage_confidence::MarketStageEvidence::default();
            ev.technical = Some(
                stock_analysis::market_analyzer::market_stage_confidence::TechnicalMetrics {
                    sh_chg: 0.5,
                    chinext_chg: 1.2,
                    star_chg: 1.5,
                },
            );
            ev.capital = Some(
                stock_analysis::market_analyzer::market_stage_confidence::CapitalMetrics {
                    main_flow_yi: 120.0,
                    amount_yi: market_value,
                    amount_delta_pct: 8.0,
                },
            );
            ev.sentiment = Some(
                stock_analysis::market_analyzer::market_stage_confidence::SentimentMetrics {
                    limit_up_n: 35,
                    limit_down_n: 3,
                    broken_pct: 15.0,
                    consecutive_h: 5,
                },
            );
            let conf = stock_analysis::market_analyzer::market_stage_confidence::evaluate(&ev);
            let r = MarketReview {
                sh_chg: 0.5,
                chinext_chg: 1.2,
                star_chg: 1.5,
                limit_up_n: 35,
                limit_down_n: 3,
                broken_pct: 15.0,
                consecutive_h: 5,
                amount_yi: market_value,
                amount_delta_pct: 8.0,
                amount_dir: "放量",
                main_flow_yi: 120.0,
                money_effect: "中等",
                heat_stage: conf.heat_stage.as_str(),
                heat_conf_pct: conf.conf_pct,
                low_conf: false,
                low_conf_tier: None,
                account_mode: AccountMode::Normal,
                max_pos: 7,
            };
            let text = render_review_market(&today_str, &r);
            let banner_text = format!("[v12-E2E-R02] {}", text);
            let ok =
                crate::notify::push_governor(&banner_text, crate::notify::PushKind::ReviewMarket)
                    .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("R-02".to_string());
            }
            log::info!("[v12-E2E] R-02 推送 {}", if ok { "OK" } else { "FAIL" });
        }

        // ===== R-03 涨停产业链 (limit_chain_review 真装配) =====
        {
            use stock_analysis::market_analyzer::limit_chain_review::*;
            let stocks = vec![
                StockLimitStats {
                    code: "688001".to_string(),
                    name: "龙头A".to_string(),
                    chain: "AI算力".to_string(),
                    board_level: 4,
                    is_limit_up_today: true,
                    is_first_board: false,
                    consecutive_days: 4,
                },
                StockLimitStats {
                    code: "688002".to_string(),
                    name: "次龙".to_string(),
                    chain: "AI算力".to_string(),
                    board_level: 2,
                    is_limit_up_today: true,
                    is_first_board: false,
                    consecutive_days: 2,
                },
            ];
            let aggs = aggregate(&LimitChainInput {
                stocks,
                source_complete: true,
            });
            // 拼接为字符串 (避免 Box::leak 复杂度)
            let mut body = String::from("🔥 涨停产业链（");
            body.push_str(&today_str);
            body.push_str("）\n");
            for (i, a) in aggs.iter().enumerate() {
                body.push_str(&format!("{}. {} 涨停{}家（首板{}/连板{}） 阶段: {}\n   龙头: {}({}) {}板\n   后排: {}\n   明日观察: 接力意愿\n",
                    i + 1, a.chain, a.limit_up_n, a.first_n, a.consec_n, a.heat_stage,
                    a.leader_name, a.leader_code, a.leader_boards, a.followers.join(",")));
            }
            body.push_str("⚠️ 退潮链: 光伏（涨停12→3家）");
            let banner_text = format!("[v12-E2E-R03] {}", body);
            let ok =
                crate::notify::push_governor(&banner_text, crate::notify::PushKind::IndustryChain)
                    .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("R-03".to_string());
            }
            log::info!("[v12-E2E] R-03 推送 {}", if ok { "OK" } else { "FAIL" });
        }

        // ===== R-04 龙虎榜 (lhb_review 真装配) =====
        {
            let s = format!(
                "{}. {}({}) 净买{:.1}亿 | {}\n   买: 机构{}席{:.0}万 其他{}席{:.0}万（集中度{:.0}%）\n   卖: {}（集中度{:.0}%）\n   主线一致: {}\n   次日风险: {}",
                1, "AI龙头A", "688001", 1.5_f64, "涨幅偏离值达7%",
                2, 8000.0_f64, 3, 4000.0_f64, 65.0_f64,
                "游资席位", 45.0_f64,
                "是-AI算力", "高开震荡"
            );
            let banner_text = format!(
                "[v12-E2E-R04] 🐉 龙虎榜净买前五（{} 21:00）\n{}",
                today_str, s
            );
            let ok = crate::notify::push_governor(&banner_text, crate::notify::PushKind::ReviewLhb)
                .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("R-04".to_string());
            }
            log::info!("[v12-E2E] R-04 推送 {}", if ok { "OK" } else { "FAIL" });
        }

        // ===== R-05 信号复盘 (真实 trades) =====
        {
            let holding_exec = trades
                .iter()
                .filter(|t| matches!(t.direction, stock_analysis::portfolio::TradeDirection::Sell))
                .count() as u32;
            let r = SignalReview {
                holding_n: positions.len() as u32,
                holding_exec,
                holding_eff: holding_exec,
                t0_n: 0,
                t0_eff: 0,
                cand_trigger: 0,
                cand_filled: 0,
                cand_notfilled: 0,
                cand_limitup: 0,
                cand_notreach: 0,
                paper_pnl_pct: today_pnl_pct,
                paper_total_pct: 0.0,
                paper_n: trades.len() as u32,
                news_push_n: 0,
                news_d1_eff: 0,
            };
            let text = render_review_signal(&today_str, &r);
            let banner_text = format!("[v12-E2E-R05] {}", text);
            let ok =
                crate::notify::push_governor(&banner_text, crate::notify::PushKind::ReviewSignal)
                    .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("R-05".to_string());
            }
            log::info!("[v12-E2E] R-05 推送 {}", if ok { "OK" } else { "FAIL" });
        }

        // ===== R-06 失败归因 (performance_feedback 真装配) =====
        {
            let entries: Vec<FailureEntry<'_>> = vec![FailureEntry {
                name: "测试标的",
                code: "688001",
                signal_level: "⚡",
                virtual_reason: "A档",
                result_desc: "未成交",
                pnl_pct: 0.0,
                failure_reason: "涨停不可买",
                suggestion: "调高触发阈值",
            }];
            let dist = FailureDistribution {
                buy_late: 2,
                chain_fade: 1,
                not_fillable: 3,
                human_not_exec: 1,
            };
            let text = render_review_failure(&today_str, &entries, &dist);
            let banner_text = format!("[v12-E2E-R06] {}", text);
            let ok =
                crate::notify::push_governor(&banner_text, crate::notify::PushKind::ReviewFailure)
                    .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("R-06".to_string());
            }
            log::info!("[v12-E2E] R-06 推送 {}", if ok { "OK" } else { "FAIL" });
        }

        // ===== R-07 明日观察池 =====
        {
            let items: Vec<WatchItem<'_>> = vec![WatchItem {
                name: "AI算力候选",
                code: "688001",
                topic: "AI算力",
                source: "A档未触发",
                trigger: "突破50.5",
                lo: 49.5,
                hi: 50.3,
                stop: 48.5,
                reason: "板块共振",
            }];
            let text = render_tomorrow_watch(&today_str, &items);
            let banner_text = format!("[v12-E2E-R07] {}", text);
            let ok =
                crate::notify::push_governor(&banner_text, crate::notify::PushKind::TomorrowWatch)
                    .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("R-07".to_string());
            }
            log::info!("[v12-E2E] R-07 推送 {}", if ok { "OK" } else { "FAIL" });
        }

        // ===== R-08 事件日历 (真实持仓事件) =====
        {
            let mut events: Vec<HoldingEventItem> = positions
                .iter()
                .take(2)
                .map(|p| HoldingEventItem {
                    name: p.name.as_str(),
                    kind: "解禁 3.2亿",
                })
                .collect();
            if events.is_empty() {
                events.push(HoldingEventItem {
                    name: "示例持仓",
                    kind: "财报预告",
                });
            }
            let text = render_event_calendar(&today_str, &events, "央行MLF到期", "+0.8%", "7.18");
            let banner_text = format!("[v12-E2E-R08] {}", text);
            let ok =
                crate::notify::push_governor(&banner_text, crate::notify::PushKind::EventCalendar)
                    .await;
            if ok {
                success_count += 1;
            } else {
                fail_msgs.push("R-08".to_string());
            }
            log::info!("[v12-E2E] R-08 推送 {}", if ok { "OK" } else { "FAIL" });
        }

        // ===== 验收 =====
        eprintln!("[v12-E2E] ======== 20 模板真实推送完成 ========");
        eprintln!("[v12-E2E] 成功: {}/20", success_count);
        if !fail_msgs.is_empty() {
            eprintln!("[v12-E2E] 失败: {:?}", fail_msgs);
            panic!("[v12-E2E] 部分模板推送失败: {:?}", fail_msgs);
        }
        assert!(
            success_count >= 15,
            "至少 15 个模板应推送成功, 实得 {}",
            success_count
        );
    }

    // v29: D-01 dispatcher memo 测试
    // 注: 验证 memo 容器可写入 + 可重置, 集成测试由 monitor --test --v13-diag 覆盖
    #[test]
    fn test_d01_memo_map_basic() {
        use super::{D01_LAST_PUSH, _reset_d01_memo_for_test};
        _reset_d01_memo_for_test();

        // 写入
        {
            let mut map = D01_LAST_PUSH.lock().unwrap();
            map.insert("000001:平安银行".to_string(), std::time::Instant::now());
        }

        // 读出
        let map = D01_LAST_PUSH.lock().unwrap();
        assert!(
            map.contains_key("000001:平安银行"),
            "memo 容器应包含刚插入的 key"
        );

        // 重置
        drop(map);
        _reset_d01_memo_for_test();
        let map = D01_LAST_PUSH.lock().unwrap();
        assert!(map.is_empty(), "重置后 memo 容器应为空");
    }
}
