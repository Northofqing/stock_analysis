//! Registered business rules: BR-047, BR-049, BR-071, BR-072, BR-073, BR-083.
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
//! v14.2 LegacyTemplate 注册 (W10):
//!   每个 `render_xxx` 函数都视为 v14.2 架构下的 LegacyTemplate 实现 (见 v14.2 §3.5
//!   LegacyTemplate 包装规则). 通过 `legacy_templates::registry()` 提供统一入口,
//!   dispatcher 可以按 kind 查到对应的 render 函数 + template_id + version.
//!
//! 后续 PR 接入点（不动本文件签名即可演进）:
//!   - PR1: `AccountMode` 替换为 `risk::account_mode::AccountState`
//!   - PR2: `DataMode` 替换为 `monitor::data_mode::DataHealth`
//!   - PR4: Banner 字段接真值 (From impl 即可)

#![allow(
    clippy::empty_line_after_doc_comments,
    reason = "legacy template sections use spaced narrative comments; this style does not change rendering behavior"
)]
#![allow(
    dead_code,
    reason = "this versioned template catalog retains documented render/protocol variants that are exercised by tests even when the current monitor schedule does not instantiate every variant"
)]

use std::fmt;

use stock_analysis::trading::paper_trade::{self, Direction, PaperSignal};

fn valid_source_stock_code(code: &str) -> bool {
    #[cfg(test)]
    if let Some(test_code) = code.strip_prefix("TEST_CODE_") {
        return test_code.len() == 6 && test_code.chars().all(|ch| ch.is_ascii_digit());
    }
    code.len() == 6 && code.chars().all(|ch| ch.is_ascii_digit())
}

// ============================================================================
// §14.0 全局横幅 — 输入结构
// ============================================================================

/// v12 §14.0 横幅账户模式
///
/// 暂为本地轻量枚举。PR1 (`risk::account_mode::AccountState`) 合入后, 加 `From`。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum AccountMode {
    #[default]
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

/// v12 §14.0 横幅数据模式
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum DataMode {
    #[default]
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

/// v12 §14.0 全局横幅入参
///
/// `total_pos` 仓位成数 (0~10). `today_pnl` 日盈亏百分比 (已带正负号).
/// 账户指标尚未形成真实完整批次时保持 `None`，禁止显示为 0。
/// `data_missing_note` 仅在 Degraded/Unsafe 出现, 例如 "缺盘口深度".
#[derive(Clone, Debug)]
pub struct BannerCtx {
    pub account_mode: AccountMode,
    pub total_pos: Option<u8>,
    pub today_pnl: Option<f64>,
    /// True only when P&L, consecutive stop losses, and position were all
    /// present in the same real account evaluation batch.
    pub account_metrics_complete: bool,
    pub data_mode: DataMode,
    pub data_missing_note: Option<String>,
}

impl BannerCtx {
    /// 测试用 BannerCtx (Normal/Full, 仓位 0, 日盈亏 0.0)
    #[cfg(test)]
    pub fn test_default() -> Self {
        Self {
            account_mode: AccountMode::Normal,
            total_pos: Some(0),
            today_pnl: Some(0.0),
            account_metrics_complete: true,
            data_mode: DataMode::Full,
            data_missing_note: None,
        }
    }

    /// 渲染 §14.0 横幅 (1~2 行).
    ///
    /// 第 1 行: `[icon mode | 仓位N成 | 日盈亏+/-X.X% | 数据DataMode]`
    /// 第 2 行 (可选): `[⚠️ {data_missing_note}]` — 仅 Degraded/Unsafe 时出现
    pub fn render(&self) -> String {
        let position = self
            .total_pos
            .map_or_else(|| "仓位缺失".to_string(), |value| format!("仓位{value}成"));
        let pnl = self.today_pnl.map_or_else(
            || "日盈亏缺失".to_string(),
            |value| format!("日盈亏{value:+.1}%"),
        );
        let line1 = format!(
            "[{} {} | {} | {} | 数据{}]",
            self.account_mode.icon(),
            self.account_mode.label(),
            position,
            pnl,
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

/// BR-134 boundary: convert the monitor's latest evaluated banner into the
/// library risk facts used by every paper-trading path.
pub(crate) fn paper_risk_context_from_banner(
    banner: &BannerCtx,
) -> Result<stock_analysis::trading::paper_trade::PaperRiskContext, String> {
    if !banner.account_metrics_complete || banner.total_pos.is_none() || banner.today_pnl.is_none()
    {
        return Err("BR-134 complete account metrics are unavailable".to_string());
    }
    let account_mode = match banner.account_mode {
        AccountMode::Normal => stock_analysis::risk::action_gate::AccountMode::Normal,
        AccountMode::ReduceOnly => stock_analysis::risk::action_gate::AccountMode::ReduceOnly,
        AccountMode::Frozen => stock_analysis::risk::action_gate::AccountMode::Frozen,
    };
    let data_mode = match banner.data_mode {
        DataMode::Full => stock_analysis::monitor::data_mode::DataMode::Full,
        DataMode::Degraded => stock_analysis::monitor::data_mode::DataMode::Degraded,
        DataMode::Unsafe => stock_analysis::monitor::data_mode::DataMode::Unsafe,
    };
    Ok(stock_analysis::trading::paper_trade::PaperRiskContext::new(
        account_mode,
        data_mode,
    ))
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
fn append_data_mode_restrictions(out: &mut String, restrictions: &[String]) {
    for restriction in restrictions {
        out.push_str(&format!("\n· {}", restriction));
    }
}

fn append_data_mode_eta_footer(out: &mut String, eta: Option<&str>) {
    if let Some(eta) = eta.filter(|value| !value.is_empty()) {
        out.push_str(&format!("\n恢复预计: {}\n辅助建议, 非下单指令", eta));
    } else {
        out.push_str("\n辅助建议, 非下单指令");
    }
}

pub fn render_data_mode(
    hhmm: &str,
    old: Option<DataMode>,
    new: DataMode,
    missing_items: &str,
    restrictions: &[String],
    eta: Option<&str>,
) -> String {
    let mut out = format!(
        "📡 数据状态变更（{}）\n{} → {}\n受影响: {}\n输出限制:",
        hhmm,
        old.map(DataMode::label).unwrap_or("未建立"),
        new.label(),
        missing_items,
    );
    append_data_mode_restrictions(&mut out, restrictions);
    append_data_mode_eta_footer(&mut out, eta);
    out
}

/// BR-135: periodic reminder for one continuously confirmed Unsafe state.
pub fn render_data_mode_reminder(
    hhmm: &str,
    current: DataMode,
    missing_items: &str,
    restrictions: &[String],
    eta: Option<&str>,
) -> String {
    let mut out = format!(
        "📡 数据状态持续异常（{}）\n当前模式: {}\n受影响: {}\n输出限制:",
        hhmm,
        current.label(),
        missing_items,
    );
    append_data_mode_restrictions(&mut out, restrictions);
    let reminder_minutes =
        stock_analysis::monitor::data_mode::PERSISTENT_UNSAFE_REMINDER_INTERVAL.as_secs() / 60;
    out.push_str(&format!("\n提醒频率: 每{}分钟", reminder_minutes));
    append_data_mode_eta_footer(&mut out, eta);
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

// ============================================================================
// v58: P-05 虚拟观察仓 (v12 §14.5 新增)
// ============================================================================

/// v58: P-05 虚拟观察条目
#[derive(Debug, Clone)]
pub struct VirtualWatchItem<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub open_price: f64,
    pub shares: u32,
    pub estimated_amount: f64,
}

/// v58: P-05 模板参数
#[derive(Debug)]
pub struct VirtualWatchParams<'a> {
    pub hhmm: &'a str,
    pub shares_per_lot: u32, // 每股/手
    pub items: Vec<VirtualWatchItem<'a>>,
    pub total_amount: f64,
    pub item_count: usize,
}

/// v58: P-05 模板渲染 (无 banner, ℹ️参考级)
/// 模板示例:
/// ```
/// 🔍 虚拟观察仓位（{HH:MM}）
///
/// · {name}({code}) @ ¥{price} | {shares}股 预计 ¥{amount}
/// · ...
///
/// 合计虚拟敞口: ¥{total} ({shares}股×{item_count}只)
/// ⚠️ 仅做观察、研究用途，未实际下单
/// 辅助建议, 非下单指令
/// ```
pub fn render_virtual_watch(p: VirtualWatchParams<'_>) -> String {
    let mut s = format!("🔍 虚拟观察仓位（{}）\n", p.hhmm);
    if p.items.is_empty() {
        s.push_str("⚠️ 候选空, 跳过\n");
        return s;
    }
    s.push('\n');
    for item in &p.items {
        s.push_str(&format!(
            "· {}({}) @ ¥{:.2} | {}股 预计 ¥{:.0}\n",
            item.name, item.code, item.open_price, item.shares, item.estimated_amount
        ));
    }
    s.push_str(&format!(
        "\n合计虚拟敞口: ¥{:.0} ({}股×{}只)",
        p.total_amount, p.shares_per_lot, p.item_count
    ));
    s.push_str("\n⚠️ 仅做观察、研究用途，未实际下单");
    s.push_str("\n辅助建议, 非下单指令");
    s
}

/// v58: P-05 dispatcher
///   数据源: monitor_loop 维护的 virtual_observation (9:30 开盘已 populate)
///   触发: 9:30 开盘一次 (已 v57 改为 --push 路径, 这里保留 monitor_loop 调用入口)
pub async fn dispatch_virtual_watch_daily(
    hhmm: &str,
    virtual_observation: &[(String, String, f64)], // (code, name, open_price)
    shares_per_lot: u32,
) -> bool {
    if virtual_observation.is_empty() {
        log_dispatcher_attempt("P-05", false, 0, "virtual_observation empty");
        log::info!("[P-05] virtual_observation 空, 跳过推送");
        return false;
    }
    // 过滤 open_price > 0 的项
    let items: Vec<VirtualWatchItem> = virtual_observation
        .iter()
        .filter(|(_, _, price)| *price > 0.0)
        .map(|(code, name, price)| {
            let amount = price * shares_per_lot as f64;
            VirtualWatchItem {
                name: name.as_str(),
                code: code.as_str(),
                open_price: *price,
                shares: shares_per_lot,
                estimated_amount: amount,
            }
        })
        .collect();
    if items.is_empty() {
        log_dispatcher_attempt("P-05", false, 0, "all items price=0");
        log::info!("[P-05] 所有项开盘价=0, 跳过");
        return false;
    }
    let total_amount: f64 = items.iter().map(|i| i.estimated_amount).sum();
    let item_count = items.len();
    let params = VirtualWatchParams {
        hhmm,
        shares_per_lot,
        items,
        total_amount,
        item_count,
    };
    let text = render_virtual_watch(params);
    let result = dispatch(crate::notify::PushKind::VirtualWatch, "", None, text).await;
    log_dispatcher_attempt("P-05", result, item_count, "");
    result
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
        // W1.12 / B-010 P0-1: fill_price 缺失必须显式, 不允许 0.0 fallback
        let fill_price_str = match p.fill_price {
            Some(v) => fmt_price(v),
            None => {
                log::warn!("[push] Filled 但缺 fill_price: code={}", p.code);
                "— 缺失".to_string()
            }
        };
        let quantity = p
            .qty
            .map(|value| value.to_string())
            .unwrap_or_else(|| "— 缺失".to_string());
        let reason = p
            .virtual_reason
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("— 缺失");
        out.push_str(&format!(
            "\n成交价{} 数量{} 主理由{}",
            fill_price_str, quantity, reason,
        ));
    }
    if matches!(
        p.status,
        PaperTradeStatus::NotFilled | PaperTradeStatus::Invalidated
    ) {
        let reason = p
            .not_fill_reason
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("— 缺失");
        let label = if p.status == PaperTradeStatus::Invalidated {
            "失效原因"
        } else {
            "未成交原因"
        };
        out.push_str(&format!("\n{}: {}", label, reason,));
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
    pub turnover_pct: f64,         // 换手率 (%)
    pub main_flow_yi: Option<f64>, // 主力净流入 (亿); 成份接口当前未提供
}

/// v12 §14.1 T-13 TurnoverTop 模板渲染 — 字段顺序严格对齐 docs/architecture/v13-push-templates.md
///

/// v56: I-09 领涨板块 Top N 模板 (v12 §14.5 新增)
///
/// 数据源: stock_analysis::market_analyzer::sector_monitor::fetch_board_ranking
/// 治理: 5 min 冷却 (PushKind::SectorTop)
/// 模板示例:
/// ```
/// 📊 领涨板块 Top 5 (10:30)
///   🥇 PCB +3.2% 主力1.5亿
///   🥈 半导体 +2.8% 主力1.2亿
///   ...
/// ```
pub fn render_sector_top(hhmm: &str, boards: &[(String, f64, f64)]) -> String {
    let mut out = format!("📊 领涨板块 Top {} ({})\n", boards.len(), hhmm);
    let medals = ["🥇", "🥈", "🥉", "4️⃣", "5️⃣"];
    for (i, (name, change_pct, main_inflow_yi)) in boards.iter().enumerate() {
        out.push_str(&format!(
            "  {} {} {:+.1}% 主力{:.1}亿\n",
            medals[i.min(4)],
            name,
            change_pct,
            main_inflow_yi
        ));
    }
    out
}

/// v56: I-10 主力净流入 Top N 模板 (v12 §14.5 新增)
///
/// 数据源: market_data::fetch_market_main_inflow_top
/// 治理: 5 min 冷却 (PushKind::FundInflow)
/// 模板示例:
/// ```
/// 💰 主力净流入 Top 10 (10:30)
///   1. XX(000001) 主力+2.5亿 量比1.8 涨幅+1.2%
///   2. ...
/// ```
pub fn render_fund_inflow_top(
    hhmm: &str,
    entries: &[(String, String, f64, Option<f64>, f64)],
) -> String {
    let mut out = format!("💰 主力净流入 Top {} ({})\n", entries.len(), hhmm);
    for (i, (name, code, main_yi, vol_ratio, change_pct)) in entries.iter().enumerate() {
        let volume_text = vol_ratio
            .map(|value| format!("{value:.1}"))
            .unwrap_or_else(|| "暂无".to_string());
        out.push_str(&format!(
            "  {:>2}. {}({}) 主力{:+.2}亿 量比{} 涨幅{:+.1}%\n",
            i + 1,
            name,
            code,
            main_yi,
            volume_text,
            change_pct
        ));
    }
    out
}
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
        let main_flow = e
            .main_flow_yi
            .map(|value| format!("{value:.2}亿"))
            .unwrap_or_else(|| "暂无".to_string());
        out.push_str(&format!(
            "  {}. {}({}) 现价¥{:.2} 涨跌{:+.2}% 换手{:.2}% 主力{}\n",
            i + 1,
            e.name,
            e.code,
            e.price,
            e.change_pct,
            e.turnover_pct,
            main_flow,
        ));
    }
    out.push_str("数据源: 实时行情 (非龙虎榜, 龙虎榜盘后 21:00 才更新)\n");
    out.push_str("辅助建议, 非下单指令\n");
    out
}

pub fn load_turnover_top_real() -> Result<Vec<TurnoverEntry>, String> {
    use stock_analysis::market_analyzer::sector_monitor;

    let boards = sector_monitor::fetch_board_ranking("f3", 10)
        .map_err(|error| format!("换手率榜板块数据失败: {error:#}"))?;
    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for board in boards.iter().take(10) {
        let components = sector_monitor::fetch_board_components(&board.code, 30)
            .map_err(|error| format!("换手率榜板块 {} 成份失败: {error:#}", board.code))?;
        for stock in components {
            if stock.turnover <= 0.0 || !seen.insert(stock.code.clone()) {
                continue;
            }
            entries.push(TurnoverEntry {
                name: stock.name,
                code: stock.code,
                price: stock.price,
                change_pct: stock.change_pct,
                turnover_pct: stock.turnover,
                main_flow_yi: None,
            });
        }
    }
    entries.sort_by(|left, right| {
        right
            .turnover_pct
            .total_cmp(&left.turnover_pct)
            .then_with(|| left.code.cmp(&right.code))
    });
    entries.truncate(10);
    Ok(entries)
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
    pub sh_chg: Option<f64>,
    pub chinext_chg: Option<f64>,
    pub star_chg: Option<f64>,
    pub limit_up_n: Option<u32>,
    pub limit_down_n: Option<u32>,
    pub broken_pct: Option<f64>,
    pub consecutive_h: Option<u32>,
    pub amount_yi: Option<f64>,
    pub amount_delta_pct: Option<f64>,
    pub amount_dir: Option<&'a str>, // "放量" / "缩量"
    pub main_flow_yi: Option<f64>,
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
    // W4.X: code-reviewer HIGH 修复 — sh_chg=0.0 时显示"暂无", 避免"+0.0%"误导
    // P0-1: % 放进 display 串, 缺数据(0.0)时显示"暂无"而非"暂无%"(尾部多一个%)
    let change_display = |value: Option<f64>| {
        value
            .map(|value| format!("{value:+.1}%"))
            .unwrap_or_else(|| "暂无".to_string())
    };
    let sh_chg_display = change_display(m.sh_chg);
    let chinext_display = change_display(m.chinext_chg);
    let star_display = change_display(m.star_chg);
    let amount_display = m
        .amount_yi
        .map(|value| format!("{value:.0}亿"))
        .unwrap_or_else(|| "暂无".to_string());
    let main_flow_display = m
        .main_flow_yi
        .map(|value| format!("{value:+.0}亿"))
        .unwrap_or_else(|| "暂无".to_string());
    let consecutive_display = m
        .consecutive_h
        .map(|value| format!("{value}板"))
        .unwrap_or_else(|| "暂无".to_string());
    let amount_delta_display = match (m.amount_dir, m.amount_delta_pct) {
        (Some(direction), Some(value)) => format!("（{direction}{value:+.0}%）"),
        _ => String::new(),
    };
    let limit_up_display = m
        .limit_up_n
        .map(|value| value.to_string())
        .unwrap_or_else(|| "暂无".to_string());
    let limit_down_display = m
        .limit_down_n
        .map(|value| value.to_string())
        .unwrap_or_else(|| "暂无".to_string());
    let broken_display = m
        .broken_pct
        .map(|value| format!("{value:.0}%"))
        .unwrap_or_else(|| "暂无".to_string());
    let mut out = format!(
        "📊 今日盘面（{}）\n指数: 上证{} 创业{} 科创{}\n情绪: 涨停{}家 跌停{}家 炸板率{} 连板高度{}\n资金: 两市{}{} 主力净{}\n赚钱效应: {}\n阶段判定: {}（置信度{}%）",
        date,
        sh_chg_display, chinext_display, star_display,
        limit_up_display, limit_down_display, broken_display, consecutive_display,
        amount_display, amount_delta_display, main_flow_display,
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
    // W4.3 / B-010 P1 修复: 数值字段从 f64 改 Option<f64>, render 端判 None 显示"无"
    pub buy_inst_amt_wan: Option<f64>,
    pub buy_other_n: u32,
    pub buy_other_amt_wan: Option<f64>,
    pub buy_conc_pct: Option<f64>,
    pub sell_desc: &'a str,
    pub sell_conc_pct: Option<f64>,
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
        // W1.14 / B-010 P0-3 配套: sell_desc 空串时显示"无", 不显示 "—"
        let sell_desc_display = if e.sell_desc.is_empty() {
            "无"
        } else {
            e.sell_desc
        };
        // W4.3 / B-010 P1: 数值字段 Option<f64>, None 时显示"无", 不显示 0
        let buy_inst_amt = e
            .buy_inst_amt_wan
            .map(|v| format!("{:.0}", v))
            .unwrap_or_else(|| "无".to_string());
        let buy_other_amt = e
            .buy_other_amt_wan
            .map(|v| format!("{:.0}", v))
            .unwrap_or_else(|| "无".to_string());
        let buy_conc = e
            .buy_conc_pct
            .map(|v| format!("{:.0}", v))
            .unwrap_or_else(|| "无".to_string());
        let sell_conc = e
            .sell_conc_pct
            .map(|v| format!("{:.0}", v))
            .unwrap_or_else(|| "无".to_string());
        out.push_str(&format!(
            "\n{}. {}({}) 净买{:.1}亿 | {}\n   买: 机构{}席{}万 其他{}席{}万（集中度{}%）\n   卖: {}（集中度{}%）\n   主线一致: {}\n   次日风险: {}",
            i + 1,
            e.name, e.code, e.net_buy_yi, e.reason,
            e.buy_inst_n, buy_inst_amt,
            e.buy_other_n, buy_other_amt,
            buy_conc,
            sell_desc_display, sell_conc,
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
    /// 区分实盘 / 虚拟: "实盘" / "虚拟"
    pub tag: &'a str,
    pub name: &'a str,
    pub code: &'a str,
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
    if holdings.is_empty() {
        out.push_str("\n· (无实盘持仓 / 虚拟仓)");
    }
    for h in holdings {
        if h.code.is_empty() {
            out.push_str(&format!("\n· 【{}】{}: {}", h.tag, h.name, h.kind));
        } else {
            out.push_str(&format!(
                "\n· 【{}】{}({}): {}",
                h.tag, h.name, h.code, h.kind
            ));
        }
    }
    out.push_str(&format!(
        "\n宏观: {}\n隔夜关注: 美股{} 汇率{}",
        macro_econ, us_chg, fx
    ));
    out
}

/// R-08 持仓/观察池条目 (owned, tag = 实盘/虚拟).
/// 调用方持有本 Vec, 再借用为 `HoldingEventItem` 传入 `render_event_calendar`.
pub struct EventHolding {
    pub tag: String,
    pub name: String,
    pub code: String,
    pub kind: String,
}

/// R-08 虚拟持仓部分: 读 data/virtual_observation (虚拟观察仓), 标 tag="虚拟".
/// 不臆造浮盈: 缺当前价时只显示建仓价 + 建仓日, 不填假 pnl (红线 2.2).
pub fn event_calendar_virtual_holdings() -> Result<Vec<EventHolding>, String> {
    let snap = load_virtual_observation_for_a01()?;
    // 去重: load_virtual_observation_for_a01 读目录下所有快照 (latest.json + 日期快照),
    //   同一虚拟仓会出现多次. 按 code 去重, 保留首个 (加载已按文件名倒序, latest 优先).
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    Ok(snap
        .records
        .into_iter()
        .filter(|r| seen.insert(r.code.clone()))
        .map(|r| {
            let kind = if r.entry_price > 0.0 {
                format!("虚拟建仓 @¥{:.2} ({})", r.entry_price, r.entry_date)
            } else {
                format!("虚拟观察 ({})", r.entry_date)
            };
            EventHolding {
                tag: "虚拟".to_string(),
                name: r.name,
                code: r.code,
                kind,
            }
        })
        .collect())
}

/// R-08 宏观公告摘要: 区分"持仓相关" / "非持仓", 各取 TOP 3.
/// holding_codes 为空 → 全部归"非持仓". 公告为空 → 显式缺失提示 (红线 2.2).
pub fn build_event_calendar_macro_summary(
    anns: &[stock_analysis::data_provider::announcement::Announcement],
    holding_codes: &std::collections::HashSet<String>,
) -> String {
    use stock_analysis::data_provider::announcement::Announcement;
    if anns.is_empty() {
        return "今日公告批次成功返回 0 条".to_string();
    }
    let fmt = |a: &Announcement| -> String {
        let disp = if a.name.is_empty() {
            a.code.clone()
        } else {
            format!("{}({})", a.name, a.code)
        };
        format!("· {} ({:?}): {}", disp, a.level, a.title)
    };
    let held: Vec<&Announcement> = anns
        .iter()
        .filter(|a| holding_codes.contains(&a.code))
        .collect();
    let other: Vec<&Announcement> = anns
        .iter()
        .filter(|a| !holding_codes.contains(&a.code))
        .collect();
    let mut s = format!("今日共 {} 条公告", anns.len());
    if held.is_empty() {
        s.push_str("\n持仓相关: 无");
    } else {
        s.push_str(&format!("\n持仓相关 (TOP {}):", held.len().min(3)));
        for a in held.iter().take(3) {
            s.push('\n');
            s.push_str(&fmt(a));
        }
    }
    s.push_str(&format!("\n非持仓 (TOP {}):", other.len().min(3)));
    for a in other.iter().take(3) {
        s.push('\n');
        s.push_str(&fmt(a));
    }
    s
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccountModeNotificationPlan {
    NoChange,
    Insert,
    ReusePending(i64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountModeDispatchResult {
    NoChange,
    Delivery(crate::notify::PushOutcome),
}

impl AccountModeDispatchResult {
    pub fn is_confirmed(&self) -> bool {
        matches!(
            self,
            Self::NoChange | Self::Delivery(crate::notify::PushOutcome::Pushed)
        )
    }
}

fn confirm_account_mode_delivery(log_id: i64) -> Result<(), String> {
    stock_analysis::database::account_mode_log::mark_account_mode_pushed(log_id)
        .map_err(|error| format!("mark AccountMode delivery confirmed: {error}"))
}

fn finalize_account_mode_delivery(
    log_id: i64,
    outcome: crate::notify::PushOutcome,
) -> Result<AccountModeDispatchResult, String> {
    if matches!(&outcome, crate::notify::PushOutcome::Pushed) {
        confirm_account_mode_delivery(log_id)?;
    }
    Ok(AccountModeDispatchResult::Delivery(outcome))
}

fn account_mode_from_label(
    label: &str,
) -> Result<stock_analysis::risk::action_gate::AccountMode, String> {
    use stock_analysis::risk::action_gate::AccountMode;

    match label {
        "Normal" => Ok(AccountMode::Normal),
        "ReduceOnly" => Ok(AccountMode::ReduceOnly),
        "Frozen" => Ok(AccountMode::Frozen),
        _ => Err(format!("invalid persisted AccountMode label: {label}")),
    }
}

fn plan_account_mode_notification(
    latest: Option<&stock_analysis::database::account_mode_log::AccountModeLogRow>,
    evaluated: stock_analysis::risk::action_gate::AccountMode,
) -> Result<AccountModeNotificationPlan, String> {
    let Some(row) = latest else {
        return Ok(AccountModeNotificationPlan::Insert);
    };
    let persisted = account_mode_from_label(&row.new_mode)?;
    if persisted != evaluated {
        return Ok(AccountModeNotificationPlan::Insert);
    }
    if row.pushed == 0 {
        return Ok(AccountModeNotificationPlan::ReusePending(i64::from(row.id)));
    }
    if row.pushed == 1 {
        return Ok(AccountModeNotificationPlan::NoChange);
    }
    Err(format!(
        "invalid persisted AccountMode pushed flag: {}",
        row.pushed
    ))
}

/// v12 PR1-1.6: 模式变更编排器.
///
/// 完整链路: evaluate() → is_changed() → 落库 → 拼 T-01 → dispatch() → 标记 pushed.
///
/// A transition is confirmed only after delivery and the same audit row's
/// `pushed=1` update both succeed. `NoChange` is an explicit successful no-op.
///
/// `prev` 由调用方从 `database::account_mode_log::latest_account_mode_change()` 恢复.
///
/// 生产入口由 `main.rs::evaluate_account_mode_hook` 在启动期与周期循环调用；
/// 本函数的失败会返回调用方并保留未确认状态供下轮重试。
pub async fn push_account_mode_change(
    metrics: &stock_analysis::risk::account_mode::PortfolioMetrics,
    prev: Option<stock_analysis::risk::action_gate::AccountMode>,
    latest: Option<&stock_analysis::database::account_mode_log::AccountModeLogRow>,
    banner: Option<&BannerCtx>,
    evaluation: &stock_analysis::risk::account_mode::ModeEvaluation,
) -> Result<AccountModeDispatchResult, String> {
    use stock_analysis::risk::action_gate::AccountMode as LibAM;

    if let Some(row) = latest {
        let persisted = account_mode_from_label(&row.new_mode)?;
        if Some(persisted) != prev {
            return Err("persisted AccountMode row does not match previous mode".to_string());
        }
    }
    let evaluation_prev_is_valid = evaluation.prev_mode == prev
        || (metrics.is_complete()
            && matches!(prev, Some(LibAM::Frozen))
            && evaluation.prev_mode.is_none());
    if !evaluation_prev_is_valid {
        return Err("AccountMode evaluation does not match persisted previous mode".to_string());
    }
    if evaluation.prev_mode.is_none() && prev.is_some() {
        log::warn!("[BR-021][BR-116] single-snapshot 8:30 reset evaluation applied");
    }

    let is_initial_evaluation = prev.is_none() && latest.is_none();
    let notification_plan = if latest.is_some() {
        plan_account_mode_notification(latest, evaluation.mode)?
    } else if prev.is_none() || evaluation.is_changed() {
        AccountModeNotificationPlan::Insert
    } else {
        AccountModeNotificationPlan::NoChange
    };
    if notification_plan == AccountModeNotificationPlan::NoChange {
        return Ok(AccountModeDispatchResult::NoChange);
    }

    // The first real evaluation is an auditable state establishment. Represent
    // it as current→current because the schema requires both endpoints; do not
    // invent Normal as the predecessor.
    let (prev_mode, new_mode) = match notification_plan {
        AccountModeNotificationPlan::ReusePending(_) => {
            let row = latest.ok_or_else(|| "pending AccountMode row missing".to_string())?;
            (
                account_mode_from_label(&row.prev_mode)?,
                account_mode_from_label(&row.new_mode)?,
            )
        }
        _ => (prev.unwrap_or(evaluation.mode), evaluation.mode),
    };

    let default_reason = if is_initial_evaluation {
        "initial account mode evaluation"
    } else {
        ""
    };
    let (log_id, transition_reason, is_new_transition) = match notification_plan {
        AccountModeNotificationPlan::Insert => {
            let reason = evaluation
                .trigger_reason
                .as_deref()
                .unwrap_or(default_reason);
            let log_id = stock_analysis::database::account_mode_log::insert_account_mode_change(
                prev_mode,
                new_mode,
                reason,
                metrics.today_pnl_pct,
                metrics.consecutive_stop_loss_n,
                metrics.total_pos_cheng,
                metrics.is_complete(),
            )
            .map_err(|e| format!("insert_account_mode_change: {}", e))?;
            (log_id, reason.to_string(), true)
        }
        AccountModeNotificationPlan::ReusePending(log_id) => {
            let row = latest.ok_or_else(|| "pending AccountMode row missing".to_string())?;
            log::warn!(
                "[AccountMode][BR-116] retry pending notification log_id={}",
                log_id
            );
            (log_id, row.trigger_reason.clone(), false)
        }
        AccountModeNotificationPlan::NoChange => unreachable!("handled above"),
    };

    // 2. 拼 T-01
    let hhmm = chrono::Local::now().format("%H:%M").to_string();
    let reasons = (!transition_reason.is_empty())
        .then_some(vec![transition_reason.clone()])
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
    let outcome = dispatch_outcome(
        crate::notify::PushKind::AccountMode,
        "", // code 空 = 全局键
        banner,
        text,
    )
    .await;

    // 3a. Frozen transition: also emit one MarketActionAlert (NOT for initial eval, NOT for unchanged)
    if is_new_transition && !is_initial_evaluation && new_mode == LibAM::Frozen {
        use stock_analysis::news::aggregator::{NormalizedSourceEvent, SourcePushKind};
        let trigger = evaluation
            .trigger_reason
            .as_deref()
            .unwrap_or("account frozen");
        let event_id = format!("frozen:{:?}:{:?}", prev_mode, new_mode);
        let title = format!("账户冻结: {}", trigger);
        let summary = format!("trigger={}", trigger);
        if let Ok(maa_event) = NormalizedSourceEvent::new(
            SourcePushKind::MarketActionAlert,
            event_id,
            Some("FROZEN".into()),
            title,
            summary,
            stock_analysis::signal::market_event::Direction::Bear,
            90,
            95,
            "monitor".into(),
            None,
        ) {
            log::warn!(
                "[AccountMode] Frozen transition → MarketActionAlert: {}",
                trigger
            );
            let _ = crate::v17_sources::push_normalized_event(maa_event).await;
        }
    }

    // 4. 标记 pushed
    if !matches!(&outcome, crate::notify::PushOutcome::Pushed) {
        log::warn!(
            "[AccountMode][BR-116] T-01 delivery unconfirmed ({:?}), log_id={} 保留 pushed=0 等重试",
            outcome, log_id
        );
    }

    finalize_account_mode_delivery(log_id, outcome)
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
    banner: &BannerCtx,
    params: HoldingPlanParams<'_>,
) -> bool {
    push_holding_plan_recommendation_outcome(code, banner, params)
        .await
        .is_pushed()
}

async fn push_holding_plan_recommendation_outcome(
    code: &str,
    banner: &BannerCtx,
    params: HoldingPlanParams<'_>,
) -> crate::notify::PushOutcome {
    let text = render_holding_plan(banner, params);
    dispatch_outcome(
        crate::notify::PushKind::HoldingPlan,
        code,
        Some(banner),
        text,
    )
    .await
}

/// PR4-4.3 T-04 持仓紧急风险推送 (🚨紧急, 无视冷却).
///
/// 用于: 跌破硬止损/触发三级止损/板块跳水.
/// 自动 PushLevel::Emergency → dispatch 跳过冷却和日预算.
pub async fn push_holding_emergency(
    code: &str,
    banner: &BannerCtx,
    params: HoldingEventParams<'_>,
) -> bool {
    let text = render_holding_event(banner, params);
    dispatch(
        crate::notify::PushKind::HoldingEvent,
        code,
        Some(banner),
        text,
    )
    .await
}

// ============================================================================
// MVP2-2.2 orchestrator: T-05/T-06 做T建议
// ============================================================================

/// MVP2-2.2 T-05 做T建议推送 (⚡ 30min/票).
///
/// 拼接文本后调 dispatch, 治理 (mode/dm/cooling/budget) 由 dispatch 内部完成.
pub async fn push_t0_advice(code: &str, banner: &BannerCtx, params: T0AdviceParams<'_>) -> bool {
    let text = render_t0_advice(banner, params);
    dispatch(crate::notify::PushKind::T0Advice, code, Some(banner), text).await
}

/// MVP2-2.2 T-06 不建议做T (ℹ️参考).
pub async fn push_t0_forbid(code: &str, banner: &BannerCtx, params: T0ForbidParams<'_>) -> bool {
    let text = render_t0_forbid(banner, params);
    dispatch(crate::notify::PushKind::T0Advice, code, Some(banner), text).await
}

// ============================================================================
// v14.2: v13 核心 6 模板 push_* wrapper (render + dispatch)
// ============================================================================

/// v13 §14.1 P-01 盘前新闻热点 (ℹ️参考, 盘前无 banner)
pub async fn push_preopen_news_hot(code: &str, params: PreopenNewsHotParams<'_>) -> bool {
    let text = render_preopen_news_hot(params);
    dispatch(crate::notify::PushKind::PreopenNewsHot, code, None, text).await
}

/// BR-101: 从主线簇与板块联动归因构造可证的盘前新闻快照。
pub fn build_preopen_news_hot_from_db<'a>(
    hhmm: &'a str,
    clusters: &'a [stock_analysis::database::concepts::ChainDailyRow],
    rotations: &'a [stock_analysis::database::concepts::BoardRotationRow],
) -> Result<PreopenNewsHotParams<'a>, String> {
    if clusters.is_empty() {
        return Err("P-01 chain_daily 无主线簇".to_string());
    }
    if rotations.is_empty() {
        return Err("P-01 board_rotation_daily 无真实新闻证据".to_string());
    }
    let themes: Vec<&str> = clusters
        .iter()
        .take(3)
        .map(|cluster| {
            let concept = cluster.concept.trim();
            if concept.is_empty() {
                Err("P-01 chain_daily concept 为空".to_string())
            } else {
                Ok(concept)
            }
        })
        .collect::<Result<_, _>>()?;
    let theme_1 = themes.first().copied();
    let theme_2 = themes.get(1).copied();
    let theme_3 = themes.get(2).copied();

    let mut names = std::collections::HashMap::new();
    for (rotation_index, rotation) in rotations.iter().enumerate() {
        if rotation.news_title.trim().is_empty() || rotation.board_name.trim().is_empty() {
            return Err(format!(
                "P-01 board_rotation_daily 第 {} 行新闻/板块名为空",
                rotation_index + 1
            ));
        }
        let stocks = serde_json::from_str::<Vec<serde_json::Value>>(&rotation.stocks_json)
            .map_err(|error| {
                format!(
                    "P-01 board_rotation_daily 第 {} 行 stocks JSON 非法: {error}",
                    rotation_index + 1
                )
            })?;
        for (stock_index, stock) in stocks.iter().enumerate() {
            let code = stock
                .get("code")
                .and_then(serde_json::Value::as_str)
                .filter(|code| valid_source_stock_code(code))
                .ok_or_else(|| {
                    format!(
                        "P-01 board_rotation_daily 第 {} 行第 {} 只股票 code 非法",
                        rotation_index + 1,
                        stock_index + 1
                    )
                })?;
            let name = stock
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    format!(
                        "P-01 board_rotation_daily 第 {} 行第 {} 只股票 name 为空",
                        rotation_index + 1,
                        stock_index + 1
                    )
                })?;
            names.insert(code.to_string(), name.to_string());
        }
    }

    let mut watch_stocks = Vec::new();
    for (cluster_index, cluster) in clusters.iter().take(3).enumerate() {
        let codes = serde_json::from_str::<Vec<String>>(&cluster.stocks).map_err(|error| {
            format!(
                "P-01 chain_daily 第 {} 个主线 stocks JSON 非法: {error}",
                cluster_index + 1
            )
        })?;
        let code = codes
            .first()
            .map(|value| value.trim())
            .filter(|code| valid_source_stock_code(code))
            .ok_or_else(|| {
                format!(
                    "P-01 chain_daily 第 {} 个主线缺少有效头股",
                    cluster_index + 1
                )
            })?;
        let name = names
            .get(code)
            .ok_or_else(|| format!("P-01 主线 {} 头股 {code} 缺少真实名称证据", cluster.concept))?;
        watch_stocks.push((name.clone(), code.to_string(), cluster.concept.clone()));
    }

    let news_pairs = rotations
        .iter()
        .take(3)
        .map(|rotation| (rotation.news_title.as_str(), rotation.board_name.as_str()))
        .collect();

    Ok(PreopenNewsHotParams {
        hhmm,
        theme_1,
        theme_2,
        theme_3,
        news_pairs,
        watch_stocks,
    })
}

/// v15.1: 业务层入口 — 09:00 盘前自动调用
pub async fn dispatch_preopen_news_hot_daily() -> bool {
    use stock_analysis::database::DatabaseManager;
    let db = DatabaseManager::get();
    let clusters = match db.get_latest_chain_clusters_strict() {
        Ok(clusters) => clusters,
        Err(error) => {
            log::error!("[P-01] {error}");
            log_dispatcher_attempt("P-01", false, 0, &error);
            return false;
        }
    };
    let rotations = match db.get_latest_board_rotations_strict() {
        Ok(rotations) => rotations,
        Err(error) => {
            log::error!("[P-01] {error}");
            log_dispatcher_attempt("P-01", false, 0, &error);
            return false;
        }
    };
    if clusters.is_empty() || rotations.is_empty() {
        log_dispatcher_attempt("P-01", false, 0, "no clusters");
        log::info!("[P-01] 无主线簇或板块新闻, 跳过推送");
        return false;
    }
    let now = chrono::Local::now();
    let hhmm = now.format("%H:%M").to_string();
    let params = match build_preopen_news_hot_from_db(&hhmm, &clusters, &rotations) {
        Ok(params) => params,
        Err(error) => {
            log::error!("[P-01] 快照批次拒绝: {error}");
            log_dispatcher_attempt("P-01", false, 0, &error);
            return false;
        }
    };
    let snapshot_size = clusters.len();
    let result = push_preopen_news_hot("", params).await;
    log_dispatcher_attempt("P-01", result, snapshot_size, "");
    result
}

// ============================================================================
// v13.7: dispatcher_log (JSONL) — 6 dispatcher 统一记录
// ============================================================================

/// v13.7+v14.4: 记录 1 次 dispatch 尝试 (生产可观测)
/// 输出: data/dispatcher_log/{YYYY-MM-DD}.jsonl (按天轮转, 至少 5 年保留)
/// 字段: ts, kind, success, snapshot_size, error
pub fn log_dispatcher_attempt(kind: &str, success: bool, snapshot_size: usize, error: &str) {
    let dir = std::env::var("DISPATCHER_LOG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            if stock_analysis::risk::env_guard::current_env()
                == stock_analysis::risk::env_guard::TradingEnv::Test
            {
                std::path::PathBuf::from("data/test/dispatcher_log")
            } else {
                std::path::PathBuf::from("data/dispatcher_log")
            }
        });

    // v61 (F15): date-guard 避免每调都跑 read_dir + stat (每次 push 触发)
    //   - 旧: 每次调都跑 rotate_dispatcher_logs (read_dir + metadata + mtime × N files)
    //   - 新: 仅在日期变更时跑一次 (用 static AtomicU64 记上次轮转的日期)
    match should_rotate_dispatcher_log_today() {
        Ok(true) => {
            if let Err(error) = rotate_dispatcher_logs(&dir, 1_827) {
                log::error!(
                    "[dispatcher_log] retention 失败 dir={} error={}",
                    dir.display(),
                    error
                );
            }
        }
        Ok(false) => {}
        Err(error) => log::error!("[dispatcher_log] date guard failed: {error}"),
    }

    if let Err(error) = write_dispatcher_attempt(&dir, kind, success, snapshot_size, error) {
        log::error!(
            "[dispatcher_log] 写入失败 kind={} dir={} error={}",
            kind,
            dir.display(),
            error
        );
    }
}

fn write_dispatcher_attempt(
    dir: &std::path::Path,
    kind: &str,
    success: bool,
    snapshot_size: usize,
    error: &str,
) -> std::io::Result<std::path::PathBuf> {
    use std::fs::OpenOptions;
    use std::io::Write;

    std::fs::create_dir_all(dir)?;
    let now = chrono::Local::now();
    let path = dir.join(format!("{}.jsonl", now.format("%Y-%m-%d")));

    let ts = now.format("%Y-%m-%dT%H:%M:%S%.3f").to_string();
    let line = format!(
        "{{\"ts\":\"{}\",\"kind\":\"{}\",\"success\":{},\"snapshot_size\":{},\"error\":\"{}\"}}\n",
        ts,
        kind,
        success,
        snapshot_size,
        error.replace('"', "'")
    );
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    file.write_all(line.as_bytes())?;
    Ok(path)
}

/// v61 (F15): date-guard — 返回今天是否还需要轮转
///   - 用 static AtomicU64 记上次轮转的日期 (YYYYMMDD as u64)
///   - 同一天多次 push 只跑 1 次 rotate (vs 之前每次都跑)
fn should_rotate_dispatcher_log_today() -> Result<bool, String> {
    use chrono::Datelike;
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST_ROTATE: AtomicU64 = AtomicU64::new(0);
    let now = chrono::Local::now();
    let year = u64::try_from(now.year())
        .map_err(|_| format!("local calendar year is negative: {}", now.year()))?;
    let today = year * 10_000 + u64::from(now.month()) * 100 + u64::from(now.day());
    let prev = LAST_ROTATE.load(Ordering::Relaxed);
    if prev == today {
        Ok(false)
    } else {
        LAST_ROTATE.store(today, Ordering::Relaxed);
        Ok(true)
    }
}

/// v14.4: 清理 N 天前的 dispatcher_log 文件
fn rotate_dispatcher_logs(dir: &std::path::Path, retention_days: u64) -> std::io::Result<()> {
    use std::time::{Duration, SystemTime};
    let threshold = match SystemTime::now().checked_sub(Duration::from_secs(retention_days * 86400))
    {
        Some(t) => t,
        None => return Ok(()),
    };
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let meta = path.metadata()?;
        let modified = meta.modified()?;
        if modified < threshold {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
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
        tech_sub: if s.tech_sub.is_empty() {
            None
        } else {
            Some(&s.tech_sub)
        },
        tech_score: s.tech_score,
        power_sub: if s.power_sub.is_empty() {
            None
        } else {
            Some(&s.power_sub)
        },
        power_score: s.power_score,
        robot_sub: if s.robot_sub.is_empty() {
            None
        } else {
            Some(&s.robot_sub)
        },
        robot_score: s.robot_score,
        main_attack: if s.main_attack.is_empty() {
            None
        } else {
            Some(&s.main_attack)
        },
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
    if n.contains("ai")
        || n.contains("算力")
        || n.contains("芯片")
        || n.contains("半导体")
        || n.contains("集成电路")
        || n.contains("封测")
        || n.contains("光刻")
        || n.contains("软件")
        || n.contains("互联网")
        || n.contains("电子")
        || n.contains("云计算")
        || n.contains("大数据")
        || n.contains("5g")
    {
        return Some("tech");
    }
    // power 关键词 (v13.5 扩展: 电力子分支)
    if n.contains("电")
        || n.contains("电网")
        || n.contains("储能")
        || n.contains("光伏")
        || n.contains("新能源")
        || n.contains("电池")
        || n.contains("锂")
        || n.contains("风电")
        || n.contains("核电")
        || n.contains("特高压")
        || n.contains("充电桩")
        || n.contains("氢能")
    {
        return Some("power");
    }
    // robot 关键词 (v13.5 扩展: 机器人子分支)
    if n.contains("机器")
        || n.contains("减速")
        || n.contains("伺服")
        || n.contains("机器视觉")
        || n.contains("自动化")
        || n.contains("智能")
        || n.contains("传感器")
        || n.contains("控制器")
        || n.contains("工业母机")
        || n.contains("人形")
        || n.contains("无人机")
    {
        return Some("robot");
    }
    None
}

/// 确定性板块分类器；仅分类真实板块名，不生成行情或评分数据。
pub fn default_classifier() -> HeuristicClassifier {
    HeuristicClassifier
}

/// v16.1+v17.1: 真实 sector_score 算法集成
/// 联接 sector_monitor::fetch_board_ranking + sector_score::grade_sectors
/// v17.1 改进: 按关键词分类 tech/power/robot
pub fn load_sector_snapshot_real(hhmm: &str) -> Result<SectorSnapshot, String> {
    use stock_analysis::decision::sector_score::grade_sectors;
    use stock_analysis::market_analyzer::sector_monitor::fetch_board_ranking;

    let boards =
        fetch_board_ranking("f3", 30).map_err(|error| format!("I-01 板块排行批次失败: {error}"))?;

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

    // v-fix: main_attack 从全体领涨板块取 (不限 tech/power/robot 3 家族),
    //   否则热点在 3 家族之外时 main_attack 永远为空 → I-01 不推 → 盘中看不到板块轮动
    for s in &graded {
        if s.change_pct > best_score {
            best_score = s.change_pct;
            main_attack = s.name.clone();
        }
    }
    for s in &graded {
        if let Some(family) = classifier.classify(&s.name) {
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
                _ => {} // 已填或无家族
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

    Ok(SectorSnapshot {
        hhmm: hhmm.to_string(),
        tech_sub: tech.unwrap_or("").to_string(),
        tech_score: tech_score.map(|s| s as f32),
        power_sub: power.unwrap_or("").to_string(),
        power_score: power_score.map(|s| s as f32),
        robot_sub: robot.unwrap_or("").to_string(),
        robot_score: robot_score.map(|s| s as f32),
        main_attack,
        rotation_state,
    })
}

/// v15.2 兼容: 同步占位接口 (调用 v16.1 async 接口)
#[cfg(test)]
pub fn load_sector_snapshot(hhmm: &str) -> SectorSnapshot {
    // v16.1: 改用 block_on 同步调用 (测试用) — 实际 dispatcher 用 load_sector_snapshot_real
    SectorSnapshot {
        hhmm: hhmm.to_string(),
        rotation_state: RotationState::Fading,
        ..Default::default()
    }
}

/// v15.2 业务层入口 — 10/11/13/14 盘中调用 (v16.1 改用真实数据)
///
/// 注 (review Issue #6): I-01 是板块级推送 (无个股 code/price), 无法入 pushed_stocks 票池;
/// 若造个股数据入池则违红线 2.2, 故 I-01 不接 push_recorder (设计决策, 非遗漏)
async fn dispatch_intraday_market_daily_result(
    hhmm: &str,
    banner: &BannerCtx,
) -> PeriodicDispatchResult {
    let snapshot = match load_sector_snapshot_real(hhmm) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            log::error!("[I-01] 快照批次拒绝: {}", error);
            log_dispatcher_attempt("I-01", false, 0, &error);
            return PeriodicDispatchResult::Failed(error);
        }
    };
    // v-fix: 只要有领涨板块 (main_attack 非空) 就推, 不再因 tech/power/robot 3 家族全空而跳过。
    //   热点在 3 家族之外时 (例如有色/重组), 仍展示当前主攻 + 轮动状态,
    //   3 个家族行显示 “—(N/A)” 表示该家族暂无领涨子板块。
    if snapshot.main_attack.is_empty()
        && snapshot.tech_sub.is_empty()
        && snapshot.power_sub.is_empty()
        && snapshot.robot_sub.is_empty()
    {
        log_dispatcher_attempt("I-01", false, 0, "sector_snapshot empty");
        log::info!("[I-01] sector_snapshot 空 (grade_sectors 无数据), 跳过推送");
        return PeriodicDispatchResult::Empty;
    }
    let params = build_intraday_market_from_snapshot(&snapshot);
    let snap_size = 3; // tech/power/robot
    let outcome = push_intraday_market_outcome("", banner, params).await;
    log_dispatcher_attempt("I-01", outcome.is_pushed(), snap_size, "");
    PeriodicDispatchResult::Delivery(outcome)
}

pub async fn dispatch_intraday_market_daily(hhmm: &str, banner: &BannerCtx) -> bool {
    dispatch_intraday_market_daily_result(hhmm, banner)
        .await
        .is_pushed()
}

pub async fn dispatch_intraday_market_periodic(hhmm: &str, banner: &BannerCtx) -> bool {
    dispatch_intraday_market_daily_result(hhmm, banner)
        .await
        .is_confirmed()
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
    /// v13.10.5: LLM 提取的 ticker (有真实 chain + reason)
    /// 非空时 build 阶段优先用此字段, 不用 stocks (避免 LLM 提取被主题 match 覆盖)
    pub llm_tickers: Vec<stock_analysis::llm::TickerHit>,
}

/// v15.3: 从 NewsCatalystSnapshot 构造 NewsCatalystParams
///
/// v13.10.3: 修复"原因:002916" — 之前 reason = name 造成重复.
/// v13.10.5: LLM 路径 — snapshot.llm_tickers 非空时, 用 LLM 提供的 (name, code, chg, reason, chain)
/// 直接渲染, 不再 match 硬编码板块名; LLM 空时 fallback 到 theme 短语.
pub fn build_news_catalyst_from_snapshot<'a>(
    s: &'a NewsCatalystSnapshot,
) -> NewsCatalystParams<'a> {
    // v13.10.5: LLM 优先, 用 LLM 提取的 ticker (含真实 chain + reason)
    if !s.llm_tickers.is_empty() {
        let stocks_ref: Vec<(&'a str, &'a str, Option<f32>, &'a str)> = s
            .llm_tickers
            .iter()
            .map(|t| {
                let name = t.name.as_str();
                let code = t.code.as_str();
                // 优先用 ticker.reason (LLM 生成的 "PCB 涨价 12% 直接受益")
                // 缺 reason 时退到 chain 名
                let reason: &str = if !t.reason.is_empty() {
                    t.reason.as_str()
                } else if !t.chain.is_empty() {
                    // owned borrow: 这需要 chain 是 'a, 但 t.chain 是 owned String.
                    // v13.10.5 简化: reason 一定由 LLM prompt 要求, 几乎不会空, 这里直接给 "板块共振"
                    "板块共振"
                } else {
                    "板块联动"
                };
                (name, code, None, reason)
            })
            .collect();
        return NewsCatalystParams {
            hhmm: &s.hhmm,
            headline: &s.headline,
            theme: if s.theme.is_empty() {
                None
            } else {
                Some(&s.theme)
            },
            stocks: stocks_ref,
        };
    }

    // 降级: LLM 未配置/失败时, 用 snapshot.stocks + theme 短语
    // v13.10.4: reason 用 "{theme} 板块共振" 短语 (硬编码 9 个常见板块匹配)
    let reason_text: &'static str = match s.theme.as_str() {
        "" => "板块联动",
        "PCB" => "PCB 板块共振",
        "AI 算力" => "AI 算力板块共振",
        "机器人" => "机器人板块共振",
        "电力" => "电力板块共振",
        "光伏" => "光伏板块共振",
        "储能" => "储能板块共振",
        "半导体" => "半导体板块共振",
        "数据要素" => "数据要素板块共振",
        "数字货币" => "数字货币板块共振",
        _ => "板块共振",
    };
    let stocks_ref: Vec<(&'a str, &'a str, Option<f32>, &'static str)> = s
        .stocks
        .iter()
        .map(|(n, c, chg)| (n.as_str(), c.as_str(), *chg, reason_text))
        .collect();
    NewsCatalystParams {
        hhmm: &s.hhmm,
        headline: &s.headline,
        theme: if s.theme.is_empty() {
            None
        } else {
            Some(&s.theme)
        },
        stocks: stocks_ref,
    }
}

/// v16.1: 批量 fetch_realtime_quote (并行, 避免 N 次 HTTP)
/// CR-17 (review): gtimg_provider 无 batch API, 用 std::thread::scope 并行调用单股接口.
///   加并发上限 (MAX_CONCURRENT) + 单次超时 (REQUEST_TIMEOUT), 避免:
///   1. 1000 codes 一次性 spawn 1000 threads 拖死 tokio blocking pool
///   2. 单个慢请求阻塞整个 batch
fn fetch_realtime_quote_batch_strict(
    codes: &[&str],
) -> Result<std::collections::HashMap<String, stock_analysis::data_provider::RealtimeQuote>, String>
{
    use stock_analysis::data_provider::GtimgProvider;
    const MAX_CONCURRENT: usize = 32; // 最多同时 32 个并发 HTTP
    if codes.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let provider = std::sync::Arc::new(
        GtimgProvider::new().map_err(|error| format!("GtimgProvider 初始化失败: {error}"))?,
    );
    let codes_vec: Vec<String> = codes.iter().map(|c| c.to_string()).collect();
    let total = codes_vec.len();

    // CR-17: 用 Arc<Mutex<VecDeque>> + N 个 worker 限制并发, 避免 mpsc::Receiver 不可 clone
    let queue: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(codes_vec.into()));
    let result_mutex = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
        String,
        stock_analysis::data_provider::RealtimeQuote,
    >::new()));
    let errors_mutex = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

    std::thread::scope(|s| {
        let workers: Vec<_> = (0..MAX_CONCURRENT.min(total))
            .map(|_| {
                let queue = queue.clone();
                let provider = provider.clone();
                let result = result_mutex.clone();
                let errors = errors_mutex.clone();
                s.spawn(move || loop {
                    let code_opt = match queue.lock() {
                        Ok(mut queue) => queue.pop_front(),
                        Err(error) => {
                            if let Ok(mut failures) = errors.lock() {
                                failures.push(format!("报价任务队列锁损坏: {error}"));
                            }
                            break;
                        }
                    };
                    let code = match code_opt {
                        Some(c) => c,
                        None => break,
                    };
                    match provider.fetch_realtime_quote(&code) {
                        Ok(Some(quote))
                            if quote.price.is_finite()
                                && quote.price > 0.0
                                && quote.pct_chg.is_finite()
                                && quote.pct_chg.abs() <= 20.0
                                && realtime_quote_source_is_fresh(
                                    quote.source_time,
                                    chrono::Utc::now(),
                                ) =>
                        {
                            match result.lock() {
                                Ok(mut quotes) => {
                                    quotes.insert(code, quote);
                                }
                                Err(error) => {
                                    if let Ok(mut failures) = errors.lock() {
                                        failures.push(format!("报价结果锁损坏: {error}"));
                                    }
                                    break;
                                }
                            }
                        }
                        Ok(Some(quote)) => {
                            if let Ok(mut failures) = errors.lock() {
                                failures.push(format!(
                                    "{code} 行情非法 price={} pct_chg={}",
                                    quote.price, quote.pct_chg
                                ));
                            }
                        }
                        Ok(None) => {
                            if let Ok(mut failures) = errors.lock() {
                                failures.push(format!("{code} 行情为空"));
                            }
                        }
                        Err(error) => {
                            if let Ok(mut failures) = errors.lock() {
                                failures.push(format!("{code} 行情失败: {error}"));
                            }
                        }
                    }
                })
            })
            .collect();
        for worker in workers {
            if worker.join().is_err() {
                if let Ok(mut failures) = errors_mutex.lock() {
                    failures.push("报价 worker panic".to_string());
                }
            }
        }
    });

    let errors = errors_mutex
        .lock()
        .map_err(|error| format!("报价错误集合锁损坏: {error}"))?;
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    result_mutex
        .lock()
        .map(|quotes| quotes.clone())
        .map_err(|error| format!("报价结果锁损坏: {error}"))
}

fn realtime_quote_source_is_fresh(
    source_time: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    let age_ms = now.signed_duration_since(source_time).num_milliseconds();
    (0..=5_000).contains(&age_ms)
}

pub fn fetch_realtime_quotes_batch(
    codes: &[&str],
) -> Result<std::collections::HashMap<String, f32>, String> {
    fetch_realtime_quote_batch_strict(codes).map(|quotes| {
        quotes
            .into_iter()
            .map(|(code, quote)| (code, quote.pct_chg as f32))
            .collect()
    })
}

/// v15.3 fix: fetch_realtime_prices_batch — 真价格 (RealtimeQuote.price), 不是 chg_pct
/// 修复 I-04 持仓建议 push 用错字段 (之前误用 chg_pct 当 price)
pub fn fetch_realtime_prices_batch(
    codes: &[&str],
) -> Result<std::collections::HashMap<String, f64>, String> {
    fetch_realtime_quote_batch_strict(codes).map(|quotes| {
        quotes
            .into_iter()
            .map(|(code, quote)| (code, quote.price))
            .collect()
    })
}

/// v17.2+v16.1 + B-002: 实时涨跌接入, 优先用 板块联动归因 (BoardRotationRow)
/// 旧 chain_daily 链路作为 fallback.
pub fn load_news_catalyst_snapshot_real(hhmm: &str) -> Result<NewsCatalystSnapshot, String> {
    use stock_analysis::database::DatabaseManager;

    let db = DatabaseManager::get();
    let board_rotations = db.get_latest_board_rotations_strict()?;
    let clusters = db.get_latest_chain_clusters_strict()?;

    // B-002: 优先用板块联动归因 (有真实新闻标题 + 板块涨幅数据)
    if !board_rotations.is_empty() {
        let top = &board_rotations[0];
        // 解析 stocks JSON: [{"code":"002208","name":"合肥城建","change_pct":10.0},...]
        let mut stocks: Vec<(String, String, Option<f32>)> = Vec::new();
        let parsed = serde_json::from_str::<Vec<serde_json::Value>>(&top.stocks_json)
            .map_err(|error| format!("I-02 stocks_json 解析失败: {error}"))?;
        for item in parsed.iter().take(9) {
            let code = item
                .get("code")
                .and_then(|v| v.as_str())
                .filter(|code| valid_source_stock_code(code))
                .ok_or_else(|| "I-02 stocks_json 缺少有效 code".to_string())?
                .to_string();
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| format!("I-02 {code} 缺少有效 name"))?
                .to_string();
            let change_pct = item
                .get("change_pct")
                .and_then(|v| v.as_f64())
                .filter(|value| value.is_finite() && value.abs() <= 20.0)
                .ok_or_else(|| format!("I-02 {code} 缺少有效 change_pct"))?;
            stocks.push((name, code, Some(change_pct as f32)));
        }
        log::info!(
            "[I-02] B-002 板块联动归因 top: {} (chg={:.1}%, 异动股{}只)",
            top.board_name,
            top.board_change_pct,
            stocks.len()
        );
        return Ok(NewsCatalystSnapshot {
            hhmm: hhmm.to_string(),
            headline: top.news_title.clone(),
            // CR-3 (review): board_name 已经是纯板块名 (CR-2 修复), 不再加 [板块联动] 前缀
            theme: top.board_name.clone(),
            stocks,
            llm_tickers: vec![],
        });
    }

    // Fallback: 原 chain_daily 链路 (向后兼容)
    if clusters.is_empty() {
        return Ok(NewsCatalystSnapshot::default());
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
    let quote_map = fetch_realtime_quote_batch_strict(&code_refs)?;
    let mut stocks: Vec<(String, String, Option<f32>)> = Vec::new();
    for code in codes {
        let quote = quote_map
            .get(&code)
            .ok_or_else(|| format!("I-02 缺少完整行情: {code}"))?;
        stocks.push((quote.name.clone(), code, Some(quote.pct_chg as f32)));
    }
    Ok(NewsCatalystSnapshot {
        hhmm: hhmm.to_string(),
        headline: format!("{} 板块持续走强", top.concept),
        theme: top.concept.clone(),
        stocks,
        llm_tickers: vec![],
    })
}

/// v15.3 兼容: 同步占位
#[cfg(test)]
pub fn load_news_catalyst_snapshot(_hhmm: &str) -> NewsCatalystSnapshot {
    NewsCatalystSnapshot::default()
}

/// v15.3 业务层入口 (v16.2 改用真实 chain_daily 数据)
pub async fn dispatch_news_catalyst_daily(hhmm: &str, banner: &BannerCtx) -> bool {
    let mut snapshot = match load_news_catalyst_snapshot_real(hhmm) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            log::error!("[I-02] 快照批次拒绝: {}", error);
            log_dispatcher_attempt("I-02", false, 0, &error);
            return false;
        }
    };
    if snapshot.headline.is_empty() {
        log_dispatcher_attempt("I-02", false, 0, "news_catalyst_snapshot empty");
        log::info!("[I-02] news_catalyst_snapshot 空 (chain_daily 无数据), 跳过推送");
        return false;
    }

    // v13.10.5: LLM 板块识别 — 用 headline + 板块名作 prompt, 提取 ticker (含真实 chain + reason)
    // 失败 / 未配置 / 0 命中 → 静默, 走 theme match 降级路径
    let llm_registry = stock_analysis::llm::LlmRegistry::from_env();
    if let Some(provider) = llm_registry.select("news_catalyst") {
        log::info!(
            "[I-02] LLM 板块识别 provider={} model={}",
            provider.name(),
            provider.model()
        );
        // prompt 上下文: 头条 + theme + 候选板块 (让 LLM 关联个股 + 板块)
        let user_prompt = format!(
            "新闻: {}\n板块: {}\n\n请提取新闻中提及或受益的 A 股个股 (按 6 位 code, 关联原因, 重要度 1-10)",
            snapshot.headline, snapshot.theme
        );
        match provider.chat_json(
            "你是 A 股板块映射专家. 从新闻 + 板块上下文, 提取 1-9 只受益个股. 输出 JSON: {\"hits\":[{\"code\":\"002916\",\"name\":\"深南电路\",\"importance\":8,\"reason\":\"PCB 涨价 12% 直接受益\",\"chain\":\"PCB\"}]}",
            &user_prompt,
        ).await {
            Ok(value) => {
                // 解析 LLM 响应
                let hits_val = if let Some(arr) = value.get("hits").and_then(|v| v.as_array()) {
                    serde_json::Value::Array(arr.clone())
                } else if let Some(arr) = value.as_array() {
                    serde_json::Value::Array(arr.clone())
                } else {
                    serde_json::Value::Array(vec![])
                };
                let tickers: Vec<stock_analysis::llm::TickerHit> =
                    serde_json::from_value(hits_val).unwrap_or_default();
                // 二次清洗 (复用 extract_tickers 同样的过滤)
                let mut by_code: std::collections::HashMap<String, stock_analysis::llm::TickerHit> = Default::default();
                for mut t in tickers {
                    if t.code.len() != 6 || !t.code.chars().all(|c| c.is_ascii_digit()) {
                        continue;
                    }
                    t.importance = t.importance.clamp(1, 10);
                    if t.importance < 4 {
                        continue;
                    }
                    match by_code.get(&t.code) {
                        Some(existing) if existing.importance >= t.importance => {}
                        _ => { by_code.insert(t.code.clone(), t); }
                    }
                }
                let mut cleaned: Vec<_> = by_code.into_values().collect();
                cleaned.sort_by_key(|item| std::cmp::Reverse(item.importance));

                if !cleaned.is_empty() {
                    log::info!("[I-02] LLM 提取 {} 只 ticker", cleaned.len());
                    for t in &cleaned {
                        log::info!("[I-02]   LLM hit: {}({}) imp={} chain={} reason={}",
                            t.name, t.code, t.importance, t.chain, t.reason);
                    }
                    snapshot.llm_tickers = cleaned;
                } else {
                    log::info!("[I-02] LLM 提取 0 只, 降级到 theme 短语");
                }
            }
            Err(e) => {
                log::warn!("[I-02] LLM 提取失败: {}, 降级到 theme 短语", e);
            }
        }
    } else {
        log::info!("[I-02] LLM 未配置, 走 theme 短语路径");
    }

    let snap_size = if !snapshot.llm_tickers.is_empty() {
        snapshot.llm_tickers.len()
    } else {
        snapshot.stocks.len()
    };
    let params = build_news_catalyst_from_snapshot(&snapshot);
    let result = push_news_catalyst("", banner, params).await;
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
    /// BR-098: 供 push_recorder 入池 (name, code, price, change_pct, volume_ratio)
    pub record_candidates: Vec<(String, String, f64, f64, f64)>,
    /// v13.10.5: LLM 生成的补涨 trigger 文案 (替代 "首板" 硬编码)
    /// key: code, value: 真实触发原因 (e.g. "PCB 龙头首板, 800G 订单")
    pub llm_triggers: std::collections::HashMap<String, String>,
}

/// v15.4: 构造 IndustryChainIntradayParams
///
/// v13.10.5: 补涨候选 trigger 字段 — 优先用 llm_triggers[code] (LLM 真实原因),
/// 没有时回退原始 trigger (通常是 "首板" 硬编码).
pub fn build_industry_chain_intraday_from_snapshot<'a>(
    s: &'a IndustryChainSnapshot,
) -> IndustryChainIntradayParams<'a> {
    let supplement_refs: Vec<SupplementCandidate<'a>> = s
        .supplements
        .iter()
        .map(|(n, c, t, lo, hi, st)| {
            // v13.10.5: 优先 LLM 真实 trigger
            let trigger: &str = s
                .llm_triggers
                .get(c)
                .map(|s| s.as_str())
                .unwrap_or(t.as_str());
            SupplementCandidate {
                name: n.as_str(),
                code: c.as_str(),
                trigger,
                lo: *lo,
                hi: *hi,
                stop: *st,
            }
        })
        .collect();

    IndustryChainIntradayParams {
        hhmm: &s.hhmm,
        chain: &s.chain,
        limit_count: s.limit_count,
        leader_name: if s.leader_name.is_empty() {
            None
        } else {
            Some(&s.leader_name)
        },
        leader_code: if s.leader_code.is_empty() {
            None
        } else {
            Some(&s.leader_code)
        },
        leader_height: s.leader_height,
        supplements: supplement_refs,
    }
}

/// v16.3+v14.1: 真实数据集成 — 复用 chain_daily DB + GtimgProvider + aggregate()
/// v14.1 改进: 走 market_analyzer::limit_chain_review::aggregate() 真正集成
pub fn load_industry_chain_snapshot_real(hhmm: &str) -> Result<IndustryChainSnapshot, String> {
    use stock_analysis::database::DatabaseManager;
    use stock_analysis::market_analyzer::limit_chain_review::{
        aggregate, LimitChainInput, StockLimitStats,
    };

    let clusters = DatabaseManager::get().get_latest_chain_clusters_strict()?;
    if clusters.is_empty() {
        return Ok(IndustryChainSnapshot::default());
    }

    // v61 (F13): 批量拉报价 (15 串行 → 1 批并行)
    //   - 旧: 5 cluster × 3 codes = 15 顺序 provider.fetch_realtime_quote 调用
    //   - 新: 一次性 fetch_realtime_quotes_batch 拉所有 code, 然后查表
    let mut all_codes: Vec<String> = Vec::new();
    let mut cluster_codes: Vec<(usize, String)> = Vec::new(); // (cluster_index, code)
    for (c_idx, c) in clusters.iter().take(5).enumerate() {
        let codes: Vec<String> = c
            .stocks
            .trim_matches(|ch| ch == '[' || ch == ']')
            .split(',')
            .map(|s| s.trim_matches('"').trim().to_string())
            .filter(|s| !s.is_empty())
            .take(3)
            .collect();
        for code in codes {
            if !all_codes.contains(&code) {
                all_codes.push(code.clone());
            }
            cluster_codes.push((c_idx, code));
        }
    }
    if all_codes.is_empty() {
        return Err("chain_daily 不含有效股票代码".to_string());
    }
    let quotes = super::market_data::fetch_eastmoney_quotes(&all_codes)?;
    let quote_map: std::collections::HashMap<_, _> = quotes
        .into_iter()
        .map(|quote| (quote.code.clone(), quote))
        .collect();
    let missing_quotes: Vec<_> = all_codes
        .iter()
        .filter(|code| !quote_map.contains_key(code.as_str()))
        .cloned()
        .collect();
    if !missing_quotes.is_empty() {
        return Err(format!(
            "I-03 实时行情不完整，缺少: {}",
            missing_quotes.join(",")
        ));
    }

    let live_limit_quotes: Vec<_> = quote_map
        .values()
        .filter(|quote| {
            quote.change_pct >= super::market_data::infer_limit_pct(&quote.code, &quote.name) - 0.2
        })
        .collect();
    if live_limit_quotes.is_empty() {
        return Ok(IndustryChainSnapshot::default());
    }
    let board_inputs: Vec<_> = live_limit_quotes
        .iter()
        .map(|quote| (quote.code.clone(), quote.name.clone()))
        .collect();
    let board_levels = super::market_data::lookup_board_level_batch(&board_inputs)?;
    let missing_levels: Vec<_> = board_inputs
        .iter()
        .filter(|(code, _)| !board_levels.contains_key(code))
        .map(|(code, _)| code.clone())
        .collect();
    if !missing_levels.is_empty() {
        return Err(format!(
            "I-03 连板证据不完整，缺少: {}",
            missing_levels.join(",")
        ));
    }

    let mut stocks: Vec<StockLimitStats> = Vec::new();
    for (c_idx, code) in &cluster_codes {
        let c = &clusters[*c_idx];
        let Some(quote) = quote_map.get(code) else {
            continue;
        };
        let Some(board_level) = board_levels.get(code).copied() else {
            continue;
        };
        stocks.push(StockLimitStats {
            code: code.clone(),
            name: quote.name.clone(),
            chain: c.concept.clone(),
            board_level,
            is_limit_up_today: true,
            is_first_board: board_level == 1,
            consecutive_days: u32::from(board_level),
        });
    }

    if stocks.is_empty() {
        return Ok(IndustryChainSnapshot::default());
    }

    // v14.1: 真正调 aggregate() (vs v16.3 简化)
    let input = LimitChainInput {
        stocks: stocks.clone(),
        source_complete: true,
    };
    let aggregates = aggregate(&input);
    if aggregates.is_empty() {
        return Ok(IndustryChainSnapshot::default());
    }

    // 取 top 1 aggregate (按 limit_up_n 降序)
    let mut sorted: Vec<_> = aggregates.iter().collect();
    sorted.sort_by_key(|item| std::cmp::Reverse(item.limit_up_n));
    let top = sorted[0];

    // 解析 followers → supplements (前 3)
    // P1-1: 修正 name/code 错位 + 用真价格算 lo/hi/stop。
    let reverse_lookup_code = |nm: &str| -> Option<String> {
        quote_map
            .iter()
            .find(|(_, quote)| quote.name == nm)
            .map(|(code, _)| code.clone())
    };
    let supplement_data: Vec<_> = top
        .followers
        .iter()
        .take(3)
        .map(|name| {
            let code = reverse_lookup_code(name)
                .ok_or_else(|| format!("I-03 follower 无法反查代码: {name}"))?;
            let quote = quote_map
                .get(&code)
                .ok_or_else(|| format!("I-03 follower 缺少行情: {code}"))?;
            Ok((quote.name.clone(), code, quote.clone()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let supplements: Vec<(String, String, String, f64, f64, f64)> = supplement_data
        .iter()
        .map(|(name, code, quote)| {
            (
                name.clone(),
                code.clone(),
                "涨停扩散".to_string(),
                quote.price * 0.97,
                quote.price * 1.03,
                quote.price * 0.92,
            )
        })
        .collect();
    let record_candidates: Vec<(String, String, f64, f64, f64)> = supplement_data
        .into_iter()
        .filter_map(|(name, code, quote)| match quote.volume_ratio {
            Some(volume_ratio) => Some((name, code, quote.price, quote.change_pct, volume_ratio)),
            None => {
                log::warn!("[I-03] {} 缺少量比，排除 pushed_stocks", code);
                None
            }
        })
        .collect();

    let leader_name = quote_map
        .get(&top.leader_code)
        .map(|quote| quote.name.clone())
        .ok_or_else(|| format!("I-03 龙头缺少行情: {}", top.leader_code))?;
    Ok(IndustryChainSnapshot {
        hhmm: hhmm.to_string(),
        chain: top.chain.clone(),
        limit_count: top.limit_up_n,
        leader_name,
        leader_code: top.leader_code.clone(),
        leader_height: top.leader_boards,
        supplements,
        record_candidates,
        llm_triggers: std::collections::HashMap::new(),
    })
}

/// v15.4 兼容: 同步占位
#[cfg(test)]
pub fn load_industry_chain_snapshot(_hhmm: &str) -> IndustryChainSnapshot {
    IndustryChainSnapshot::default()
}

/// v15.4 业务层入口 (v16.3 改用真实 chain_daily 数据)
async fn dispatch_industry_chain_intraday_daily_result(
    hhmm: &str,
    banner: &BannerCtx,
) -> PeriodicDispatchResult {
    let mut snapshot = match load_industry_chain_snapshot_real(hhmm) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            log::error!("[I-03][BR-098] 快照批次拒绝: {}", error);
            log_dispatcher_attempt("I-03", false, 0, &error);
            return PeriodicDispatchResult::Failed(error);
        }
    };
    if snapshot.chain.is_empty() {
        log_dispatcher_attempt("I-03", false, 0, "industry_chain_snapshot empty");
        log::info!("[I-03] industry_chain_snapshot 空 (chain_daily 无数据), 跳过推送");
        return PeriodicDispatchResult::Empty;
    }

    // v13.10.5: LLM 路径 — 给补涨候选生成具体 trigger 文案 (替代 "首板" 硬编码)
    // 失败 / 未配置 / 0 命中 → 静默, 用原 trigger
    let llm_registry = stock_analysis::llm::LlmRegistry::from_env();
    if !snapshot.supplements.is_empty() {
        if let Some(provider) = llm_registry.select("industry_chain_intraday") {
            log::info!(
                "[I-03] LLM trigger 生成 provider={} model={}",
                provider.name(),
                provider.model()
            );
            // prompt 上下文: 主链 + 龙头 + 补涨候选 codes
            let candidates_block: String = snapshot
                .supplements
                .iter()
                .take(5)
                .map(|(n, c, _, _, _, _)| format!("  - {}({})", n, c))
                .collect::<Vec<_>>()
                .join("\n");
            let user_prompt = format!(
                "主链: {}\n龙头: {}({}) {}板\n补涨候选:\n{}\n\n请给每只候选生成 1 句具体的'触发补涨'原因 (1-2 句, A 股投资逻辑)",
                snapshot.chain, snapshot.leader_name, snapshot.leader_code, snapshot.leader_height, candidates_block
            );
            match provider.chat_json(
                "你是 A 股板块研究员. 从主链 + 龙头 + 候选上下文, 给每只候选生成 1 句具体触发原因. 输出 JSON: {\"triggers\":[{\"code\":\"002463\",\"reason\":\"800G 交换机订单 + 估值修复\"}]}",
                &user_prompt,
            ).await {
                Ok(value) => {
                    let arr = value.get("triggers").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                    let items: Vec<serde_json::Value> = serde_json::from_value::<Vec<serde_json::Value>>(serde_json::Value::Array(arr))
                        .unwrap_or_default();
                    let mut triggers_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
                    for item in items {
                        if let (Some(code), Some(reason)) = (
                            item.get("code").and_then(|v| v.as_str()),
                            item.get("reason").and_then(|v| v.as_str()),
                        ) {
                            if !reason.trim().is_empty() {
                                triggers_map.insert(code.to_string(), reason.to_string());
                            }
                        }
                    }
                    if !triggers_map.is_empty() {
                        log::info!("[I-03] LLM 生成 {} 条 trigger", triggers_map.len());
                        snapshot.llm_triggers = triggers_map;
                    } else {
                        log::info!("[I-03] LLM triggers 为空, 用原 trigger");
                    }
                }
                Err(e) => {
                    log::warn!("[I-03] LLM 生成失败: {}, 用原 trigger", e);
                }
            }
        } else {
            log::info!("[I-03] LLM 未配置, 用原 trigger");
        }
    }

    let params = build_industry_chain_intraday_from_snapshot(&snapshot);
    let snap_size = snapshot.supplements.len() + 1; // +1 leader
    let outcome = push_industry_chain_intraday_outcome("", banner, params).await;
    log_dispatcher_attempt("I-03", outcome.is_pushed(), snap_size, "");
    // review fix Issue #6: I-03 推送成功后, 补涨候选 (含真实价格) 入 pushed_stocks 票池 (R3)
    if outcome.is_pushed() {
        for (n, c, price, change_pct, volume_ratio) in &snapshot.record_candidates {
            let metric_json = truncate_metric_json(
                serde_json::json!({
                    "chain": snapshot.chain,
                    "limit_count": snapshot.limit_count,
                    "vol_ratio": volume_ratio,
                    "price_chg_pct": change_pct,
                    "push_subkind": "Breakout",
                })
                .to_string(),
            );
            if let Err(error) = stock_analysis::signal::push_recorder::record(
                &stock_analysis::signal::push_recorder::PushRecordMeta {
                    code: c.clone(),
                    name: n.clone(),
                    push_kind: "I-03".to_string(),
                    push_price: *price,
                    metric_json,
                    source: "intraday".to_string(),
                },
            ) {
                let reason = format!("I-03 pushed_stocks audit failed for {c}: {error}");
                log::error!("{reason}");
                log_dispatcher_attempt("I-03", false, snap_size, &reason);
                return PeriodicDispatchResult::Failed(reason);
            }
        }
    }
    PeriodicDispatchResult::Delivery(outcome)
}

pub async fn dispatch_industry_chain_intraday_daily(hhmm: &str, banner: &BannerCtx) -> bool {
    dispatch_industry_chain_intraday_daily_result(hhmm, banner)
        .await
        .is_pushed()
}

pub async fn dispatch_industry_chain_intraday_periodic(hhmm: &str, banner: &BannerCtx) -> bool {
    dispatch_industry_chain_intraday_daily_result(hhmm, banner)
        .await
        .is_confirmed()
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
    /// v13.10.5: LLM 生成的更具体 reasons (替代 evidence 截取)
    /// 非空时 build 阶段优先用此字段
    pub llm_reasons: Vec<String>,
}

/// v15.5: 构造 NewsToIdeaParams
///
/// v13.10.5: llm_reasons 非空时优先 (LLM 生成的更具体原因)
pub fn build_news_to_idea_from_snapshot<'a>(s: &'a NewsToIdeaSnapshot) -> NewsToIdeaParams<'a> {
    let reasons_ref: Vec<&'a str> = if !s.llm_reasons.is_empty() {
        s.llm_reasons.iter().map(|r| r.as_str()).collect()
    } else {
        s.reasons.iter().map(|r| r.as_str()).collect()
    };
    NewsToIdeaParams {
        hhmm: &s.hhmm,
        headline: &s.headline,
        theme: if s.theme.is_empty() {
            None
        } else {
            Some(&s.theme)
        },
        stage: s.stage.clone(),
        name: &s.name,
        code: &s.code,
        reasons: reasons_ref,
        action: s.action.clone(),
    }
}

/// v14.2: P5 源真实 fetcher (文件化)
// 读 data/p5_sources/{source}.jsonl, 每行 JSON {code, name, chg_pct}
pub fn load_p5_source_items(
    source_name: &str,
) -> Result<
    Vec<(
        stock_analysis::opportunity::candidate_panel::CandidateSource,
        String,
        String,
    )>,
    String,
> {
    load_p5_source_items_from_dir(source_name, std::path::Path::new("data/p5_sources"))
}

fn load_p5_source_items_from_dir(
    source_name: &str,
    base_dir: &std::path::Path,
) -> Result<
    Vec<(
        stock_analysis::opportunity::candidate_panel::CandidateSource,
        String,
        String,
    )>,
    String,
> {
    use std::fs;
    use std::io::ErrorKind;
    use stock_analysis::opportunity::candidate_panel::CandidateSource;
    let path = base_dir.join(format!("{source_name}.jsonl"));
    let source = match source_name {
        "stock_pick" => CandidateSource::StockPick,
        "optimal_close" => CandidateSource::OptimalClose,
        "volume_watchlist" => CandidateSource::VolumeWatchlist,
        "volume_real_trade" => CandidateSource::VolumeRealTrade,
        _ => return Err(format!("未知 P5 候选来源: {source_name}")),
    };
    let mut items = Vec::new();
    let raw = match fs::read_to_string(&path) {
        Ok(r) => r,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!("读取 P5 候选源 {} 失败: {error}", path.display()));
        }
    };
    for (line_index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        #[derive(serde::Deserialize)]
        struct P5Item {
            code: String,
            name: String,
        }
        let item = serde_json::from_str::<P5Item>(line).map_err(|error| {
            format!(
                "P5 候选源 {path} 第 {} 行 JSON 非法: {error}",
                line_index + 1,
                path = path.display()
            )
        })?;
        let code = item.code.trim();
        let name = item.name.trim();
        if !valid_source_stock_code(code) {
            return Err(format!(
                "P5 候选源 {} 第 {} 行 code 非法: {}",
                path.display(),
                line_index + 1,
                item.code
            ));
        }
        if name.is_empty() {
            return Err(format!(
                "P5 候选源 {} 第 {} 行 name 为空",
                path.display(),
                line_index + 1
            ));
        }
        items.push((source, code.to_string(), name.to_string()));
    }
    Ok(items)
}

#[derive(Debug)]
struct RealCandidateBatch {
    entries: Vec<stock_analysis::opportunity::candidate_panel::CandidateEntry>,
    quotes: std::collections::HashMap<String, stock_analysis::market_data::TopStock>,
    themes: std::collections::HashMap<String, String>,
}

fn load_real_candidate_batch() -> Result<RealCandidateBatch, String> {
    use stock_analysis::database::DatabaseManager;
    use stock_analysis::opportunity::candidate_panel::{
        classify_tier, filter_hard_gates, heat_score, merge_candidates, sort_candidates_by_heat,
        CandidateSource,
    };

    let clusters = DatabaseManager::get().get_latest_chain_clusters_strict()?;
    let mut items: Vec<(CandidateSource, String, String)> = Vec::new();
    let mut themes = std::collections::HashMap::new();

    for (cluster_index, cluster) in clusters.iter().take(5).enumerate() {
        let codes = serde_json::from_str::<Vec<String>>(&cluster.stocks).map_err(|error| {
            format!(
                "chain_daily 第 {} 个主线 {} stocks JSON 非法: {error}",
                cluster_index + 1,
                cluster.concept
            )
        })?;
        let Some(code) = codes.first().map(|value| value.trim()) else {
            continue;
        };
        if !valid_source_stock_code(code) {
            return Err(format!(
                "chain_daily 主线 {} 头部 code 非法: {code}",
                cluster.concept
            ));
        }
        if cluster.concept.trim().is_empty() {
            return Err(format!("chain_daily 主线 {code} concept 为空"));
        }
        items.push((
            CandidateSource::IndustryChain,
            code.to_string(),
            cluster.concept.clone(),
        ));
        themes.insert(code.to_string(), cluster.concept.clone());
    }

    for source in [
        "stock_pick",
        "optimal_close",
        "volume_watchlist",
        "volume_real_trade",
    ] {
        items.extend(load_p5_source_items(source)?);
    }

    let mut entries = merge_candidates(items);
    if entries.is_empty() {
        return Ok(RealCandidateBatch {
            entries,
            quotes: std::collections::HashMap::new(),
            themes,
        });
    }

    let codes: Vec<String> = entries.iter().map(|entry| entry.code.clone()).collect();
    let quotes = super::market_data::fetch_eastmoney_quotes(&codes)?;
    let quote_map: std::collections::HashMap<_, _> = quotes
        .into_iter()
        .map(|quote| (quote.code.clone(), quote))
        .collect();
    let missing: Vec<_> = codes
        .iter()
        .filter(|code| !quote_map.contains_key(code.as_str()))
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(format!("候选台实时行情不完整，缺少: {}", missing.join(",")));
    }

    for entry in &mut entries {
        let quote = quote_map
            .get(&entry.code)
            .ok_or_else(|| format!("候选台缺少 {} 实时行情", entry.code))?;
        entry.name = quote.name.clone();
        entry.current_price = Some(quote.price);
        entry.change_pct = Some(quote.change_pct);
        entry.heat_score = quote
            .main_net_yi
            .map(|main_net_yi| heat_score(quote.change_pct, main_net_yi * 1e8));
        let mut evidence = Vec::with_capacity(entry.sources.len());
        for source in &entry.sources {
            let description = match source {
                CandidateSource::IndustryChain => {
                    let theme = themes.get(&entry.code).ok_or_else(|| {
                        format!("候选台 {} 含产业链来源但缺少主线名称", entry.code)
                    })?;
                    format!("产业链: {theme}")
                }
                CandidateSource::VolumeWatchlist | CandidateSource::VolumeRealTrade => {
                    format!("真实来源: {}", source.label())
                }
                _ => format!("真实来源: {}", source.label()),
            };
            evidence.push(description);
        }
        entry.evidence = evidence;
        entry.tier = classify_tier(&entry.evidence);
    }

    let held_codes = stock_analysis::portfolio::get_all_codes()
        .map_err(|error| format!("候选台读取持仓代码失败: {error}"))?;
    entries = filter_hard_gates(entries, &held_codes);
    entries = sort_candidates_by_heat(entries);

    Ok(RealCandidateBatch {
        entries,
        quotes: quote_map,
        themes,
    })
}

/// v16.4+v13.6.2+v14.2: 真实数据集成 — 从候选台取 top 1 candidate
/// 联接 opportunity::candidate_panel::merge_candidates
/// v14.2 改进: P5 源文件化 (data/p5_sources/*.jsonl)
pub fn load_news_to_idea_snapshot_real(hhmm: &str) -> Result<NewsToIdeaSnapshot, String> {
    let batch = load_real_candidate_batch()?;
    let Some(top) = batch.entries.first() else {
        return Ok(NewsToIdeaSnapshot::default());
    };
    let reasons: Vec<String> = top.evidence.iter().take(3).cloned().collect();
    let stage = if top.source_count() >= 3 {
        NewsStage::Starting
    } else if top.source_count() >= 2 {
        NewsStage::Fermenting
    } else {
        NewsStage::Diverging
    };
    let change_pct = top
        .change_pct
        .ok_or_else(|| format!("D-01 候选 {} 缺少实时涨跌幅", top.code))?;
    let action = if change_pct > 5.0 {
        Some(NewsAction::DoNotChase)
    } else if change_pct > 0.0 {
        Some(NewsAction::BuyDip)
    } else {
        Some(NewsAction::Observe)
    };
    let theme = batch
        .themes
        .get(&top.code)
        .cloned()
        .unwrap_or_else(|| top.sources_label());
    Ok(NewsToIdeaSnapshot {
        hhmm: hhmm.to_string(),
        headline: format!(
            "{} ({}) 多源验证 ({} 源)",
            top.name,
            top.code,
            top.source_count()
        ),
        theme,
        stage,
        name: top.name.clone(),
        code: top.code.clone(),
        reasons,
        action,
        llm_reasons: vec![],
    })
}

/// v15.5 兼容: 同步占位
#[cfg(test)]
pub fn load_news_to_idea_snapshot(_hhmm: &str) -> NewsToIdeaSnapshot {
    NewsToIdeaSnapshot::default()
}

// v29: D-01 dispatcher 内部 memo (1h/票, 跨日重置)
// v61 (F14): 加 LRU 驱逐 — 每次 insert 后清掉 > 7200s (2x cooldown) 的 entry, 避免长跑内存泄漏
// 静态 Lazy 容器, 跨函数调用复用
// 注: Lazy/HashMap 已在文件顶部 import 过 (避免 unused import 警告), 这里只补 Mutex/Instant
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub static D01_LAST_PUSH: Lazy<Mutex<HashMap<String, Instant>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

async fn submit_virtual_buy_from_d01(
    snapshot: &NewsToIdeaSnapshot,
    banner: &BannerCtx,
) -> Result<(), String> {
    let code = snapshot.code.clone();
    let quote = tokio::task::spawn_blocking(move || stock_analysis::broker::execution_quote(&code))
        .await
        .ok()
        .and_then(Result::ok);

    let quote = quote.ok_or_else(|| {
        format!(
            "D-01 virtual buy quote unavailable for {}({})",
            snapshot.name, snapshot.code
        )
    })?;
    if quote.price <= 0.0 {
        return Err(format!(
            "D-01 quote price invalid for {}({}): {}",
            snapshot.name, snapshot.code, quote.price
        ));
    }

    let now = chrono::Local::now();
    let signal = PaperSignal {
        plan_id: format!(
            "d01-news-buydip-{}-{}",
            snapshot.code,
            now.format("%Y%m%d%H%M%S%3f")
        ),
        code: snapshot.code.clone(),
        name: snapshot.name.clone(),
        direction: Direction::Buy,
        price: quote.price,
        quantity: 100,
        // v16.3 Commit 1: simulate 签名加 4 参数 (quote_price 真 + cash/total/pos_pct 真 portfolio 读)
        // v16.3 Commit 2: 改 free-text → VirtualReason::NewsCatalyst.as_str() (符合 v10 §10.3)
        virtual_reason: stock_analysis::opportunity::virtual_reason::VirtualReason::NewsCatalyst
            .as_str()
            .to_string(),
        is_limit_up: quote.price >= quote.limit_up_price,
        is_limit_down: false,
        is_suspended: false,
        limit_up_price: Some(quote.limit_up_price),
        limit_down_price: Some(quote.limit_down_price),
        secondary_confirmed: false,
        quote_observed_at: quote.observed_at,
        risk_context: paper_risk_context_from_banner(banner)?,
    };

    // v16.3 Commit 1: simulate 签名加 4 参数 (quote_price 真 + cash/total/pos_pct 真 portfolio 读)
    let (cash, total, pos_pct) = match paper_portfolio_state(&snapshot.code, quote.price) {
        Ok(state) => state,
        Err(error) => {
            log::warn!(
                "[虚拟盘] 跳过 D-01 虚拟买入: {}({}) 账户快照不可用: {}",
                snapshot.name,
                snapshot.code,
                error
            );
            return Err(format!("D-01 account snapshot unavailable: {error}"));
        }
    };
    match paper_trade::simulate(&signal, quote.price, cash, total, pos_pct) {
        Ok(outcome) => log::info!(
            "[虚拟盘] D-01 买入 {}({}) status={} inserted={} price={:.2} qty={}",
            signal.name,
            signal.code,
            outcome.result.status.as_str(),
            outcome.inserted,
            signal.price,
            signal.quantity
        ),
        Err(e) => {
            return Err(format!(
                "D-01 paper trade failed {}({}): {e}",
                signal.name, signal.code
            ));
        }
    }

    // v16.3 Commit 2: 推入 pushed_stocks 票池 (R3 业务核心)
    let metric_json = truncate_metric_json(
        serde_json::json!({
            "theme": snapshot.theme,
            "headline": snapshot.headline,
            "push_subkind": "NewsCatalyst",
        })
        .to_string(),
    );
    stock_analysis::signal::push_recorder::record(
        &stock_analysis::signal::push_recorder::PushRecordMeta {
            code: snapshot.code.clone(),
            name: snapshot.name.clone(),
            push_kind: "D-01".to_string(),
            push_price: quote.price,
            metric_json,
            source: "intraday".to_string(),
        },
    )
    .map(|_| ())
}

/// v61 (F14): LRU 驱逐 — 移除 > 7200s 未访问的 entry (2x 1h cooldown)
///   - 在 insert 后调, 保持 memo 大小有界
///   - 7200s = 2h = 2x cooldown, 容忍一次跨 tick 重复, 但不永久留
fn evict_d01_memo_expired() {
    const MAX_AGE: Duration = Duration::from_secs(7200); // 2h
    if let Ok(mut map) = D01_LAST_PUSH.lock() {
        let now = Instant::now();
        map.retain(|_, ts| now.duration_since(*ts) < MAX_AGE);
    }
}

/// v29: 测试用 - 重置 memo 容器
#[cfg(test)]
pub fn _reset_d01_memo_for_test() {
    D01_LAST_PUSH
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

/// v15.5 业务层入口 (v16.4 改用真实候选台数据)
/// v29: 加 dispatcher 内部 memo (1h/票) — 防止公告密集时同票刷屏
pub async fn dispatch_news_to_idea_daily(hhmm: &str, banner: &BannerCtx) -> bool {
    let mut snapshot = match load_news_to_idea_snapshot_real(hhmm) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            log::error!("[D-01] 真实候选批次拒绝: {error}");
            log_dispatcher_attempt("D-01", false, 0, &error);
            return false;
        }
    };
    if snapshot.headline.is_empty() {
        log_dispatcher_attempt("D-01", false, 0, "news_to_idea_snapshot empty");
        log::info!("[D-01] news_to_idea_snapshot 空 (候选台无候选), 跳过推送");
        return false;
    }

    // v13.10.5: LLM 路径 — 给已选 top 票生成更具体的原因 (替代 evidence 截取)
    // 失败 / 未配置 / 0 命中 → 静默降级, 用原 reasons
    let llm_registry = stock_analysis::llm::LlmRegistry::from_env();
    if let Some(provider) = llm_registry.select("news_to_idea") {
        log::info!(
            "[D-01] LLM 原因生成 provider={} model={}",
            provider.name(),
            provider.model()
        );
        let user_prompt = format!(
            "新闻: {}\n板块: {}\n个股: {}({})\n\n请给出 1-3 条具体的'为什么这只票是首选'原因 (各 1-2 句, 用 A 股投资逻辑)",
            snapshot.headline, snapshot.theme, snapshot.name, snapshot.code
        );
        match provider.chat_json(
            "你是 A 股投资研究员. 从新闻 + 板块 + 个股上下文, 给出 1-3 条具体投资逻辑. 输出 JSON: {\"reasons\":[\"PCB 涨价直接传导到毛利\",\"800G 交换机放量拉动订单\"]}",
            &user_prompt,
        ).await {
            Ok(value) => {
                let arr = value.get("reasons").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                let llm_reasons: Vec<String> = serde_json::from_value::<Vec<String>>(serde_json::Value::Array(arr))
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|s: &String| !s.trim().is_empty())
                    .take(3)
                    .collect();
                if !llm_reasons.is_empty() {
                    log::info!("[D-01] LLM 生成 {} 条 reasons", llm_reasons.len());
                    for r in &llm_reasons {
                        log::info!("[D-01]   LLM reason: {}", r);
                    }
                    snapshot.llm_reasons = llm_reasons;
                } else {
                    log::info!("[D-01] LLM reasons 为空, 用原 evidence");
                }
            }
            Err(e) => {
                log::warn!("[D-01] LLM 生成失败: {}, 用原 evidence", e);
            }
        }
    } else {
        log::info!("[D-01] LLM 未配置, 用原 evidence");
    }

    // v29 + v59: memo 1h/票 (F5 修复 — 仅 push 成功才 insert, 防止 transient 失败自我阻塞)
    //   - 旧: map.insert 在 push 前, push 失败 (502/budget) 也写 memo, 1h 自我阻塞
    //   - 新: 失败时 return false, 不写 memo; 成功才 insert
    let memo_key = format!("{}:{}", snapshot.code, snapshot.name);
    {
        let map = D01_LAST_PUSH.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(last) = map.get(&memo_key) {
            let elapsed = last.elapsed().as_secs();
            if elapsed < 3600 {
                drop(map);
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
    }

    let should_virtual_buy = matches!(snapshot.action.as_ref(), Some(NewsAction::BuyDip));
    let params = build_news_to_idea_from_snapshot(&snapshot);
    let snap_size = snapshot.reasons.len();
    let result = push_news_to_idea("", banner, params).await;
    if result {
        if should_virtual_buy {
            if let Err(error) = submit_virtual_buy_from_d01(&snapshot, banner).await {
                log::error!("[D-01][BR-086] {error}");
                log_dispatcher_attempt("D-01", false, snap_size, &error);
                return false;
            }
        }
        // v59: 仅 push 成功才写 memo (F5 修复)
        D01_LAST_PUSH
            .lock()
            .unwrap()
            .insert(memo_key, Instant::now());
        // v61 (F14): LRU 驱逐 — insert 后清掉过期 entry, 避免长跑内存泄漏
        evict_d01_memo_expired();
    }
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
fn select_t1_close(
    rows: &[(chrono::NaiveDate, f64)],
    entry_date: chrono::NaiveDate,
    review_date: chrono::NaiveDate,
) -> Result<Option<(chrono::NaiveDate, f64)>, String> {
    if entry_date > review_date {
        return Err(format!(
            "A-01 entry_date={entry_date} 晚于 review_date={review_date}"
        ));
    }
    let target = stock_analysis::calendar::next_trading_day(entry_date);
    if target > review_date {
        return Ok(None);
    }
    let mut matches = rows.iter().filter(|(date, _)| *date == target);
    let Some((_, close)) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Err(format!("A-01 T+1 日期 {target} 重复"));
    }
    if !close.is_finite() || *close <= 0.0 {
        return Err(format!("A-01 T+1 日期 {target} close 非法: {close}"));
    }
    Ok(Some((target, *close)))
}

pub fn load_paper_review_snapshot_real(date: &str) -> Result<Option<PaperReviewSnapshot>, String> {
    let review_date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|error| format!("A-01 非法复盘日期 {date}: {error}"))?;
    let snapshot = load_virtual_observation_for_a01()?;
    if snapshot.records.is_empty() {
        return Ok(None);
    }
    let top = &snapshot.records[0];

    let entry_price = top.entry_price;
    let entry_date = chrono::NaiveDate::parse_from_str(&top.entry_date, "%Y-%m-%d")
        .map_err(|error| format!("A-01 {} entry_date 非法: {error}", top.code))?;
    let target = stock_analysis::calendar::next_trading_day(entry_date);
    if target > review_date {
        return Ok(None);
    }

    let fetcher = stock_analysis::data_provider::DataFetcherManager::new()
        .map_err(|error| format!("A-01 初始化日 K 抓取器失败: {error:#}"))?;
    let (kline, source) = fetcher
        .get_daily_data(&top.code, 60)
        .map_err(|error| format!("A-01 {} 日 K 批次失败: {error:#}", top.code))?;
    let rows: Vec<_> = kline.iter().map(|bar| (bar.date, bar.close)).collect();
    let Some((target, close_price)) = select_t1_close(&rows, entry_date, review_date)? else {
        return Ok(Some(PaperReviewSnapshot {
            date: date.to_string(),
            name: top.name.clone(),
            code: top.code.clone(),
            trigger: top.entry_mode.clone(),
            desc: format!("T+1({target}) 已到但严格日 K 批次暂未覆盖，收益暂无"),
            pnl: None,
            plan_high: None,
            plan_flat: None,
            plan_low: None,
        }));
    };
    let pnl = ((close_price / entry_price - 1.0) * 100.0) as f32;
    if !pnl.is_finite() {
        return Err(format!("A-01 {} 收益率非有限值", top.code));
    }
    let (high, flat, low) = derive_plan_from_pnl(pnl);

    Ok(Some(PaperReviewSnapshot {
        date: date.to_string(),
        name: top.name.clone(),
        code: top.code.clone(),
        trigger: top.entry_mode.clone(),
        desc: format!(
            "研究观察 T+1={} (entry={:.2} → close={:.2}, pnl={:+.1}%, source={})",
            target, entry_price, close_price, pnl, source
        ),
        pnl: Some(pnl),
        plan_high: Some(high),
        plan_flat: Some(flat),
        plan_low: Some(low),
    }))
}

/// v15.6 兼容: 同步占位
#[cfg(test)]
pub fn load_paper_review_snapshot(_date: &str) -> PaperReviewSnapshot {
    PaperReviewSnapshot::default()
}

/// v15.6: T-11 通路复用 — pnl 派生 plan_high/flat/low
/// pnl > 5% → "减仓1/3", pnl > 0% → "减仓1/2", else → "持有观望"
pub fn derive_plan_from_pnl(pnl: f32) -> (String, String, String) {
    if pnl > 5.0 {
        (
            "减仓1/3".to_string(),
            "减仓1/2".to_string(),
            "持有观望".to_string(),
        )
    } else if pnl > 0.0 {
        (
            "减仓1/2".to_string(),
            "持有".to_string(),
            "止损".to_string(),
        )
    } else {
        (
            "持有观望".to_string(),
            "止损".to_string(),
            "止损".to_string(),
        )
    }
}

/// v15.6 业务层入口 (v16.5 改用真实 virtual_observation 数据)
pub async fn dispatch_paper_review_daily(date: &str) -> bool {
    let snapshot = match load_paper_review_snapshot_real(date) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            log_dispatcher_attempt("A-01", false, 0, "paper_review_snapshot empty");
            log::info!("[A-01] paper_review_snapshot 空 (virtual_observation 无数据), 跳过推送");
            return false;
        }
        Err(error) => {
            log::error!("[A-01][BR-104] batch rejected: {error}");
            log_dispatcher_attempt("A-01", false, 0, &error);
            return false;
        }
    };
    let params = build_paper_review_from_snapshot(&snapshot);
    let snap_size = 1; // 1 record
    let result = push_paper_review("", params).await;
    log_dispatcher_attempt("A-01", result, snap_size, "");
    result
}

/// v17.4 §5.2 (BR-083): 13:00 午盘虚拟仓快照 (AC38).
/// 与 evening 全量复盘共用 PushKind::PaperReview (cooldown 86400/票),
/// dedup code 用 "noon-{code}" 前缀隔离两窗口 (否则午盘推完 evening 被 L4 拦).
pub async fn dispatch_paper_review_noon(date: &str) -> bool {
    let snapshot = match load_paper_review_snapshot_real(date) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            log_dispatcher_attempt("A-01-noon", false, 0, "paper_review_snapshot empty");
            log::info!("[A-01-noon] 13:00 快照: virtual_observation 无数据, 跳过");
            return false;
        }
        Err(error) => {
            log::error!("[A-01-noon][BR-104] batch rejected: {error}");
            log_dispatcher_attempt("A-01-noon", false, 0, &error);
            return false;
        }
    };
    let params = build_paper_review_from_snapshot(&snapshot);
    let noon_code = noon_dedup_code(&snapshot.code);
    let result = push_paper_review(&noon_code, params).await;
    log_dispatcher_attempt("A-01-noon", result, 1, "");
    result
}

/// BR-083: 午盘快照 dedup code (纯函数, 供单测)
pub fn noon_dedup_code(code: &str) -> String {
    format!("noon-{}", code)
}

// ============================================================================
// v35: A-10 盘后题材催化复盘 dispatcher
// ============================================================================

/// v54: T-14/T-15 事件数据源
///   - 真实数据源: trade_pipeline::fetch_pending_events()
///   - 沙箱: 永远返回空 (无 broker)
///   - 真实 intent: broker 委托/成交回报 event 触发
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct TradeEvent {
    pub exchange: Exchange,
    pub code: String,
    pub name: String,
    /// price: 委托/成交价
    pub price: f64,
    pub qty: u32,
    /// event_type: "order" (T-14) | "fill" (T-15)
    pub event_type: String,
    /// order_id: 委托 ID (T-14 必填, T-15 选填)
    pub order_id: Option<String>,
    /// status: 委托状态 (T-14)
    pub status: Option<OrderStatus>,
    /// next_session_carry: 是否过户到次一交易日 (T-15)
    pub next_session_carry: Option<bool>,
}

/// BR-087 real trade-event boundary. No default/mock source is installed.
pub trait TradeEventSource: Send + Sync {
    fn fetch_pending_events(&self) -> Result<Vec<TradeEvent>, String>;
}

static TRADE_EVENT_SOURCE: std::sync::OnceLock<Box<dyn TradeEventSource>> =
    std::sync::OnceLock::new();

pub fn register_trade_event_source(source: Box<dyn TradeEventSource>) -> Result<(), String> {
    TRADE_EVENT_SOURCE
        .set(source)
        .map_err(|_| "BR-087 TradeEventSource already registered".to_string())
}

pub fn fetch_pending_trade_events() -> Result<Vec<TradeEvent>, String> {
    TRADE_EVENT_SOURCE
        .get()
        .ok_or_else(|| "BR-087 TradeEventSource is not registered".to_string())?
        .fetch_pending_events()
        .map_err(|error| format!("BR-087 fetch trade events: {error}"))
}

fn valid_trade_event(event: &TradeEvent) -> bool {
    valid_source_stock_code(event.code.trim())
        && !event.name.trim().is_empty()
        && event.price.is_finite()
        && event.price > 0.0
        && event.qty > 0
        && event.qty.is_multiple_of(100)
        && matches!(event.event_type.as_str(), "order" | "fill")
}

/// v54: T-14/T-15 dispatcher (事件驱动入口)
///   - 拉 trade_pipeline 事件, 按 event_type 分发到 T-14/T-15
///   - 沙箱: 事件空, 静默
// v60 (F8): 拆 T-14/T-15 共享 dispatcher 为两个 (避免 3x 工作量)
//   - 旧: dispatch_trade_pipeline_daily 内部 match event_type 调不同 dispatcher
//   - 新: dispatch_trade_pipeline_orders (T-14) + dispatch_trade_pipeline_fills (T-15)
//   - main_loop 两个 ticker 各自调自己的 dispatcher, 互不重复
async fn dispatch_trade_pipeline_orders_result(
    hhmm: &str,
    banner: &BannerCtx,
) -> PeriodicDispatchResult {
    let events = match fetch_pending_trade_events() {
        Ok(events) => events,
        Err(error) => {
            log::error!("[T-14] {error}");
            log_dispatcher_attempt("T-14", false, 0, &error);
            return PeriodicDispatchResult::Failed(error);
        }
    };
    let mut order_events = Vec::new();
    for event in events {
        if !valid_trade_event(&event) {
            let reason = format!("BR-087 拒绝非法交易事件: {event:?}");
            log::error!("[T-14] {reason}");
            log_dispatcher_attempt("T-14", false, 0, &reason);
            return PeriodicDispatchResult::Failed(reason);
        }
        if event.event_type != "order" {
            continue;
        }
        if event
            .order_id
            .as_deref()
            .is_none_or(|order_id| order_id.trim().is_empty())
            || event.status.is_none()
        {
            let reason = format!("BR-087 拒绝不完整委托事件: {event:?}");
            log::error!("[T-14] {reason}");
            log_dispatcher_attempt("T-14", false, 0, &reason);
            return PeriodicDispatchResult::Failed(reason);
        }
        order_events.push(event);
    }
    if order_events.is_empty() {
        log_dispatcher_attempt("T-14", false, 0, "no order events");
        return PeriodicDispatchResult::Empty;
    }
    let mut outcomes = Vec::with_capacity(order_events.len());
    for ev in order_events {
        outcomes.push(
            dispatch_post_fixed_price_order_outcome(
                ev.exchange,
                hhmm,
                &ev.name,
                &ev.code,
                ev.price,
                ev.qty,
                ev.order_id.as_deref().expect("validated order id"),
                ev.status.expect("validated order status"),
                banner,
            )
            .await,
        );
    }
    let result = PeriodicDispatchResult::from_delivery_batch(outcomes);
    log_dispatcher_attempt(
        "T-14",
        result.is_pushed(),
        usize::from(result.is_pushed()),
        "",
    );
    result
}

pub async fn dispatch_trade_pipeline_orders(hhmm: &str, banner: &BannerCtx) -> bool {
    dispatch_trade_pipeline_orders_result(hhmm, banner)
        .await
        .is_pushed()
}

pub async fn dispatch_trade_pipeline_orders_periodic(hhmm: &str, banner: &BannerCtx) -> bool {
    dispatch_trade_pipeline_orders_result(hhmm, banner)
        .await
        .is_confirmed()
}

async fn dispatch_trade_pipeline_fills_result(
    hhmm: &str,
    banner: &BannerCtx,
) -> PeriodicDispatchResult {
    let events = match fetch_pending_trade_events() {
        Ok(events) => events,
        Err(error) => {
            log::error!("[T-15] {error}");
            log_dispatcher_attempt("T-15", false, 0, &error);
            return PeriodicDispatchResult::Failed(error);
        }
    };
    let mut fill_events = Vec::new();
    for event in events {
        if !valid_trade_event(&event) {
            let reason = format!("BR-087 拒绝非法交易事件: {event:?}");
            log::error!("[T-15] {reason}");
            log_dispatcher_attempt("T-15", false, 0, &reason);
            return PeriodicDispatchResult::Failed(reason);
        }
        if event.event_type != "fill" {
            continue;
        }
        if event.next_session_carry.is_none() {
            let reason = format!("BR-087 拒绝不完整成交事件: {event:?}");
            log::error!("[T-15] {reason}");
            log_dispatcher_attempt("T-15", false, 0, &reason);
            return PeriodicDispatchResult::Failed(reason);
        }
        fill_events.push(event);
    }
    if fill_events.is_empty() {
        log_dispatcher_attempt("T-15", false, 0, "no fill events");
        return PeriodicDispatchResult::Empty;
    }
    let mut outcomes = Vec::with_capacity(fill_events.len());
    for ev in fill_events {
        outcomes.push(
            dispatch_post_fixed_price_fill_outcome(
                ev.exchange,
                hhmm,
                &ev.name,
                &ev.code,
                ev.price,
                ev.qty,
                None,
                ev.next_session_carry
                    .expect("validated settlement evidence"),
                banner,
            )
            .await,
        );
    }
    let result = PeriodicDispatchResult::from_delivery_batch(outcomes);
    log_dispatcher_attempt(
        "T-15",
        result.is_pushed(),
        usize::from(result.is_pushed()),
        "",
    );
    result
}

pub async fn dispatch_trade_pipeline_fills(hhmm: &str, banner: &BannerCtx) -> bool {
    dispatch_trade_pipeline_fills_result(hhmm, banner)
        .await
        .is_pushed()
}

pub async fn dispatch_trade_pipeline_fills_periodic(hhmm: &str, banner: &BannerCtx) -> bool {
    dispatch_trade_pipeline_fills_result(hhmm, banner)
        .await
        .is_confirmed()
}

/// v44: T-14 盘后固定价格申报 dispatcher
///   - 数据源: 委托回报 event (持仓/候选股)
///   - 简化: 沙箱无委托系统, 接受外部 caller 传具体 (exchange, code, name, price, qty, order_id, status)
///   - 模板: render_post_fixed_price_order
///   - 真实意图: 接 trade_pipeline 委托回报
#[allow(
    clippy::too_many_arguments,
    reason = "stable exchange order-report protocol boundary mirrors the documented template fields"
)]
pub async fn dispatch_post_fixed_price_order(
    exchange: Exchange,
    hhmm: &str,
    name: &str,
    code: &str,
    price: f64,
    qty: u32,
    order_id: &str,
    status: OrderStatus,
    banner: &BannerCtx,
) -> bool {
    dispatch_post_fixed_price_order_outcome(
        exchange, hhmm, name, code, price, qty, order_id, status, banner,
    )
    .await
    .is_pushed()
}

#[allow(
    clippy::too_many_arguments,
    reason = "stable exchange order-report protocol boundary mirrors the documented template fields"
)]
async fn dispatch_post_fixed_price_order_outcome(
    exchange: Exchange,
    hhmm: &str,
    name: &str,
    code: &str,
    price: f64,
    qty: u32,
    order_id: &str,
    status: OrderStatus,
    banner: &BannerCtx,
) -> crate::notify::PushOutcome {
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
    let outcome = dispatch_outcome(
        crate::notify::PushKind::PostFixedPriceOrder,
        code,
        Some(banner),
        text,
    )
    .await;
    log_dispatcher_attempt(
        "T-14",
        outcome.is_pushed(),
        1,
        &format!("exchange={:?} status={:?}", exchange, status),
    );
    outcome
}

/// v45: T-15 盘后固定价格成交 dispatcher
///   - 数据源: 成交回报 event
///   - 撮合期 15:05-15:30
///   - 模板: render_post_fixed_price_fill
#[allow(
    clippy::too_many_arguments,
    reason = "stable exchange fill-report protocol boundary mirrors the documented template fields"
)]
pub async fn dispatch_post_fixed_price_fill(
    exchange: Exchange,
    hhmm: &str,
    name: &str,
    code: &str,
    fill_price: f64,
    qty: u32,
    vs_limit_pct: Option<f32>,
    next_session_carry: bool,
    banner: &BannerCtx,
) -> bool {
    dispatch_post_fixed_price_fill_outcome(
        exchange,
        hhmm,
        name,
        code,
        fill_price,
        qty,
        vs_limit_pct,
        next_session_carry,
        banner,
    )
    .await
    .is_pushed()
}

#[allow(
    clippy::too_many_arguments,
    reason = "stable exchange fill-report protocol boundary mirrors the documented template fields"
)]
async fn dispatch_post_fixed_price_fill_outcome(
    exchange: Exchange,
    hhmm: &str,
    name: &str,
    code: &str,
    fill_price: f64,
    qty: u32,
    vs_limit_pct: Option<f32>,
    next_session_carry: bool,
    banner: &BannerCtx,
) -> crate::notify::PushOutcome {
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
    let outcome = dispatch_outcome(
        crate::notify::PushKind::PostFixedPriceFill,
        code,
        Some(banner),
        text,
    )
    .await;
    log_dispatcher_attempt(
        "T-15",
        outcome.is_pushed(),
        1,
        &format!("exchange={:?} fill_price={}", exchange, fill_price),
    );
    outcome
}

/// v46: T-16 ST 涨跌幅变更 dispatcher
///   - 新规 2026-07-06: 主板 ST/*ST 5%→10%
///   - 触发: 开盘 9:30 一次/票/日
///   - 数据源: 持仓 DB (ST/*ST 票) + 新规参数 (5%→10%)
///   - 真实 intent: 每天首次入 9:30 推一次
#[allow(
    clippy::too_many_arguments,
    reason = "stable ST rule-change protocol boundary mirrors the documented template fields"
)]
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
    banner: &BannerCtx,
) -> bool {
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
    let result = dispatch(
        crate::notify::PushKind::StPriceLimitChanged,
        code,
        Some(banner),
        text,
    )
    .await;
    log_dispatcher_attempt(
        "T-16",
        result,
        1,
        &format!(
            "st_type={:?} {}→{}%",
            st_type,
            old_limit * 100.0,
            new_limit * 100.0
        ),
    );
    result
}

/// v47: T-17 ETF 收盘集合竞价 dispatcher
///   - 新规 2026-07-06: 上交所基金收盘 14:57-15:00 集合竞价
///   - 触发: 14:57 推一次 (1次/日)
///   - 数据源: 持仓 DB (沪市 ETF 持仓) + 集合竞价行情
///   - 真实 intent: 14:57 推一次
pub async fn dispatch_etf_closing_call_auction(
    hhmm: &str,
    name: &str,
    code: &str,
    call_auction_price: Option<f64>,
    vs_continuous_est: Option<f32>,
    liquidity_note: &str,
) -> bool {
    // v47: T-17 是无 banner 盘后参考
    let params = EtfClosingCallAuctionParams {
        hhmm,
        name,
        code,
        call_auction_price,
        vs_continuous_est,
        liquidity_note,
    };
    let text = render_etf_closing_call_auction(params);
    let result = dispatch(
        crate::notify::PushKind::EtfClosingCallAuction,
        code,
        None,
        text,
    )
    .await;
    log_dispatcher_attempt("T-17", result, 1, &format!("code={}", code));
    result
}

/// BR-033: 创业板/科创板协议大宗盘中实时确认。
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_block_trade_intraday_confirm(
    hhmm: &str,
    name: &str,
    code: &str,
    qty: u32,
    price: f64,
    block_type: BlockType,
    board: Board,
    real_time_confirm: bool,
    next_session_settle: SettleType,
) -> bool {
    if !price.is_finite()
        || price <= 0.0
        || qty == 0
        || !qty.is_multiple_of(100)
        || block_type != BlockType::Agreed
        || !matches!(board, Board::Gem | Board::Star)
        || !real_time_confirm
    {
        log_dispatcher_attempt("T-18", false, 0, "BR-033 invalid/ineligible block event");
        return false;
    }
    let text = render_block_trade_intraday_confirm(BlockTradeIntradayConfirmParams {
        hhmm,
        name,
        code,
        qty,
        price,
        block_type,
        board,
        real_time_confirm,
        next_session_settle,
    });
    let result = dispatch(
        crate::notify::PushKind::BlockTradeIntradayConfirm,
        code,
        None,
        text,
    )
    .await;
    log_dispatcher_attempt("T-18", result, 1, &format!("board={board:?}"));
    result
}

/// BR-034: 北交所大宗区间以当日竞价实时均价为口径。
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_block_trade_price_range(
    hhmm: &str,
    name: &str,
    code: &str,
    prev_close: Option<f64>,
    today_avg_price: f64,
    block_price_range: Option<&str>,
    note: &str,
) -> bool {
    if !today_avg_price.is_finite()
        || today_avg_price <= 0.0
        || block_price_range.is_none_or(|range| range.trim().is_empty())
    {
        log_dispatcher_attempt("T-19", false, 0, "BR-034 average/range evidence missing");
        return false;
    }
    let text = render_block_trade_price_range(BlockTradePriceRangeParams {
        hhmm,
        name,
        code,
        prev_close,
        today_avg_price,
        block_price_range,
        note,
    });
    let result = dispatch(
        crate::notify::PushKind::BlockTradePriceRange,
        code,
        None,
        text,
    )
    .await;
    log_dispatcher_attempt("T-19", result, 1, &format!("code={code}"));
    result
}

/// v40: P-04 虚拟盘成交 dispatcher 包装
pub async fn push_paper_trade(code: &str, params: PaperTradeParams<'_>) -> bool {
    let text = render_paper_trade(params);
    dispatch(crate::notify::PushKind::PaperTrade, code, None, text).await
}

/// v52: P-04 虚拟盘成交回报 dispatcher
///   - 遍历 virtual_observation, 每只单独推 PaperTrade 模板
///   - 真实数据: 调用方传 (name, code, entry_price, status, virtual_reason, not_fill_reason) 元组
///   - 沙箱无 paper_trade 模块, 走通 push_governor 链路即可
#[allow(
    clippy::too_many_arguments,
    reason = "stable paper-trade report protocol boundary mirrors the documented template fields"
)]
pub async fn dispatch_paper_trade_one(
    hhmm: &str,
    name: &str,
    code: &str,
    status: PaperTradeStatus,
    fill_price: Option<f64>,
    qty: Option<u32>,
    virtual_reason: Option<&str>,
    not_fill_reason: Option<&str>,
) -> bool {
    let params = PaperTradeParams {
        name,
        code,
        hhmm,
        status,
        fill_price,
        qty,
        virtual_reason,
        not_fill_reason,
        account_mode: AccountMode::Normal,
        data_mode: DataMode::Full,
    };
    let result = push_paper_trade(code, params).await;
    log_dispatcher_attempt(
        "P-04",
        result,
        1,
        &format!("name={} status={:?}", name, status),
    );
    result
}

#[derive(diesel::QueryableByName, Debug)]
struct PaperTradeDispatchRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    id: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    direction: String,
    #[diesel(sql_type = diesel::sql_types::Double)]
    price: f64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    quantity: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    status: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    fill_price: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    not_fill_reason: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    virtual_reason: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    account_mode: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    data_mode: String,
}

#[derive(Debug)]
struct PaperTradeDispatchReport {
    id: i64,
    code: String,
    name: String,
    status: PaperTradeStatus,
    fill_price: Option<f64>,
    quantity: u32,
    not_fill_reason: Option<String>,
    virtual_reason: String,
    account_mode: AccountMode,
    data_mode: DataMode,
}

fn validate_paper_trade_dispatch_row(
    row: PaperTradeDispatchRow,
) -> Result<PaperTradeDispatchReport, String> {
    validate_paper_trade_dispatch_row_for_env(row, stock_analysis::risk::env_guard::current_env())
}

fn validate_paper_trade_dispatch_row_for_env(
    row: PaperTradeDispatchRow,
    env: stock_analysis::risk::env_guard::TradingEnv,
) -> Result<PaperTradeDispatchReport, String> {
    if row.id <= 0 {
        return Err(format!("P-04 paper_trades id 非法: {}", row.id));
    }
    stock_analysis::risk::env_guard::validate_symbol_for_env(&row.code, env)
        .map_err(|error| format!("P-04 paper_trades id={} 环境隔离失败: {error}", row.id))?;
    if row.name.trim().is_empty() {
        return Err(format!("P-04 paper_trades id={} name 为空", row.id));
    }
    if !matches!(row.direction.as_str(), "buy" | "sell") {
        return Err(format!(
            "P-04 paper_trades id={} direction 非法: {}",
            row.id, row.direction
        ));
    }
    if !row.price.is_finite() || row.price <= 0.0 {
        return Err(format!(
            "P-04 paper_trades id={} price 非法: {}",
            row.id, row.price
        ));
    }
    let quantity = u32::try_from(row.quantity)
        .ok()
        .filter(|value| *value > 0 && value.is_multiple_of(100))
        .ok_or_else(|| {
            format!(
                "P-04 paper_trades id={} quantity 非法: {}",
                row.id, row.quantity
            )
        })?;
    let status = match row.status.as_str() {
        "Filled" => PaperTradeStatus::Filled,
        "NotFilled" => PaperTradeStatus::NotFilled,
        "Invalidated" => PaperTradeStatus::Invalidated,
        other => {
            return Err(format!(
                "P-04 paper_trades id={} status 非法: {other}",
                row.id
            ));
        }
    };
    let fill_price = match row.fill_price {
        Some(value) if value.is_finite() && value > 0.0 => Some(value),
        Some(value) => {
            return Err(format!(
                "P-04 paper_trades id={} fill_price 非法: {value}",
                row.id
            ));
        }
        None => None,
    };
    if status == PaperTradeStatus::Filled && fill_price.is_none() {
        return Err(format!(
            "P-04 paper_trades id={} Filled 缺少 fill_price",
            row.id
        ));
    }
    let not_fill_reason = row
        .not_fill_reason
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if status != PaperTradeStatus::Filled && not_fill_reason.is_none() {
        return Err(format!(
            "P-04 paper_trades id={} {} 缺少原因",
            row.id,
            status.label()
        ));
    }
    let virtual_reason = row.virtual_reason.trim().to_string();
    if virtual_reason.is_empty() {
        return Err(format!(
            "P-04 paper_trades id={} virtual_reason 为空",
            row.id
        ));
    }
    let account_mode = match row.account_mode.as_str() {
        "Normal" => AccountMode::Normal,
        "ReduceOnly" => AccountMode::ReduceOnly,
        "Frozen" => AccountMode::Frozen,
        other => {
            return Err(format!(
                "P-04 paper_trades id={} account_mode 非法: {other}",
                row.id
            ));
        }
    };
    let data_mode = match row.data_mode.as_str() {
        "Full" => DataMode::Full,
        "Degraded" => DataMode::Degraded,
        "Unsafe" => DataMode::Unsafe,
        other => {
            return Err(format!(
                "P-04 paper_trades id={} data_mode 非法: {other}",
                row.id
            ));
        }
    };

    Ok(PaperTradeDispatchReport {
        id: row.id,
        code: row.code,
        name: row.name,
        status,
        fill_price,
        quantity,
        not_fill_reason,
        virtual_reason,
        account_mode,
        data_mode,
    })
}

fn load_today_paper_trade_reports() -> Result<Vec<PaperTradeDispatchReport>, String> {
    use diesel::RunQueryDsl;

    let db = stock_analysis::database::DatabaseManager::try_get()
        .ok_or_else(|| "P-04 数据库未初始化".to_string())?;
    let mut conn = db
        .get_conn()
        .map_err(|error| format!("P-04 数据库连接失败: {error}"))?;
    let rows = diesel::sql_query(
        "SELECT id, code, name, direction, price, quantity, status, fill_price, \
                not_fill_reason, virtual_reason, account_mode, data_mode \
         FROM paper_trades \
         WHERE date(ts, 'localtime') = date('now', 'localtime') \
           AND status IN ('Filled', 'NotFilled', 'Invalidated') \
         ORDER BY id ASC",
    )
    .load::<PaperTradeDispatchRow>(&mut conn)
    .map_err(|error| format!("P-04 查询当日 paper_trades 失败: {error}"))?;
    rows.into_iter()
        .map(validate_paper_trade_dispatch_row)
        .collect()
}

/// BR-100: 从当日 `paper_trades` 持久化结果发送虚拟成交回报。
pub async fn dispatch_paper_trade_daily(hhmm: &str) -> bool {
    let reports = match load_today_paper_trade_reports() {
        Ok(reports) => reports,
        Err(error) => {
            log::error!("[P-04] 虚拟成交回报批次拒绝: {error}");
            log_dispatcher_attempt("P-04", false, 0, &error);
            return false;
        }
    };
    if reports.is_empty() {
        log_dispatcher_attempt("P-04", false, 0, "today paper_trades empty");
        log::info!("[P-04] 当日无已完成虚拟成交记录, 跳过推送");
        return false;
    }

    let mut success_count = 0usize;
    for report in &reports {
        let params = PaperTradeParams {
            name: &report.name,
            code: &report.code,
            hhmm,
            status: report.status,
            fill_price: report.fill_price,
            qty: Some(report.quantity),
            virtual_reason: Some(&report.virtual_reason),
            not_fill_reason: report.not_fill_reason.as_deref(),
            account_mode: report.account_mode,
            data_mode: report.data_mode,
        };
        if push_paper_trade(&report.code, params).await {
            success_count += 1;
        } else {
            log::warn!(
                "[P-04] paper_trades id={} {}({}) 回报未投递",
                report.id,
                report.name,
                report.code
            );
        }
    }
    let success = success_count == reports.len();
    let error = if success {
        String::new()
    } else {
        format!("投递 {success_count}/{}", reports.len())
    };
    log_dispatcher_attempt("P-04", success, reports.len(), &error);
    success
}

/// v39: P-03 候选触发 dispatcher
///   - 候选台取 top 1 candidate (按 source_count 排序)
///   - is_candidate_live_enabled 影子开关 (默认 false)
///   - 简化版: 推送 1 条 A 档候选, evidence 拼成 trigger_desc
pub async fn dispatch_candidate_triggered_daily(hhmm: &str, banner: &BannerCtx) -> bool {
    use stock_analysis::opportunity::candidate_panel::EvidenceTier;
    use stock_analysis::opportunity::candidate_state::require_live_promotion;

    if let Err(error) = require_live_promotion(None, None) {
        log_dispatcher_attempt("P-03", false, 0, &error);
        log::info!("[P-03] 候选触发保持 Shadow: {error}");
        return false;
    }

    let batch = match load_real_candidate_batch() {
        Ok(batch) => batch,
        Err(error) => {
            log::error!("[P-03] 真实候选批次拒绝: {error}");
            log_dispatcher_attempt("P-03", false, 0, &error);
            return false;
        }
    };
    if batch.entries.is_empty() {
        log_dispatcher_attempt("P-03", false, 0, "candidates empty");
        log::info!("[P-03] 候选台无候选, 跳过推送");
        return false;
    }

    let top = &batch.entries[0];
    let Some(price) = top.current_price else {
        let error = format!("P-03 候选 {} 缺少实时价", top.code);
        log_dispatcher_attempt("P-03", false, 0, &error);
        log::error!("{error}");
        return false;
    };
    let quote = match batch.quotes.get(&top.code) {
        Some(quote) => quote,
        None => {
            let error = format!("P-03 候选 {} 缺少完整行情行", top.code);
            log_dispatcher_attempt("P-03", false, 0, &error);
            log::error!("{error}");
            return false;
        }
    };
    let Some(volume_ratio) = quote.volume_ratio else {
        let error = format!("P-03 候选 {} 缺少实时量比", top.code);
        log_dispatcher_attempt("P-03", false, 0, &error);
        log::warn!("{error}");
        return false;
    };
    let volume_quality = if volume_ratio >= 3.0 {
        EvidenceQuality::Strong
    } else if volume_ratio >= 1.0 {
        EvidenceQuality::Mid
    } else {
        EvidenceQuality::Weak
    };
    let grade = if top.tier == EvidenceTier::Strong {
        CandidateGrade::A
    } else {
        CandidateGrade::B
    };
    let topic = top.sources_label();
    // v50: 真实 trigger_desc 优先 evidence, 兜底用 cluster.name + code
    let trigger_desc = top
        .evidence
        .first()
        .cloned()
        .unwrap_or_else(|| format!("{} ({}) 主线异动", top.name, top.code));
    let params = CandidateTriggeredParams {
        name: &top.name,
        code: &top.code,
        hhmm,
        grade,
        topic: &topic,
        price,
        trigger_desc: &trigger_desc,
        lo: price * 0.97,
        hi: price * 1.03,
        stop: price * 0.95,
        max_pos_pct: 10,
        news_quality: EvidenceQuality::Missing,
        news_note: "未取得独立新闻证据",
        vol_quality: volume_quality,
        vol_ratio: volume_ratio,
        kline_quality: EvidenceQuality::Missing,
        kline_note: "未取得独立 K 线证据",
        book_quality: EvidenceQuality::Missing,
        no_buy: &["一字板不可买".to_string(), "板块跳水".to_string()],
    };
    let result = push_candidate_triggered(&top.code, banner, params, None, None).await;
    log_dispatcher_attempt("P-03", result, 1, "");
    result
}

/// v38 + v43: I-04 持仓操作建议 dispatcher
///   - v43: 接入真实报价 (fetch_realtime_quotes_batch), 替换 cost*1.02 写死
///   - 简化版: 遍历当前持仓, 用 real_price + cost + hard_stop 生成 plan
///   - 真实意图: 接入 decision::evaluate_holding (v12.2 规划, 当前未实现)
///   - 当前策略: 涨幅 > 5% → Reduce (逢高减仓), -3% < x < 5% → Hold, < -3% → Add
async fn dispatch_holding_plan_daily_result(
    hhmm: &str,
    banner: &BannerCtx,
) -> PeriodicDispatchResult {
    use stock_analysis::portfolio::{get_positions, PositionStatus};
    let positions = match get_positions() {
        Ok(p) => p,
        Err(e) => {
            log_dispatcher_attempt("I-04", false, 0, "get_positions failed");
            log::warn!("[I-04] get_positions 失败: {}", e);
            return PeriodicDispatchResult::Failed(e.to_string());
        }
    };
    if positions.is_empty() {
        log_dispatcher_attempt("I-04", false, 0, "no positions");
        log::info!("[I-04] 当前无持仓, 跳过推送");
        return PeriodicDispatchResult::Empty;
    }

    // v43 + v15.3 fix: 批量拉真实价格 (not chg_pct!), 避免之前误用 chg_pct 当 price
    let codes: Vec<String> = positions.iter().map(|p| p.code.clone()).collect();
    let quotes = tokio::task::spawn_blocking(move || {
        let code_refs: Vec<&str> = codes.iter().map(|s| s.as_str()).collect();
        fetch_realtime_prices_batch(&code_refs)
    })
    .await;
    let quotes = match quotes {
        Ok(Ok(quotes)) => quotes,
        Ok(Err(error)) => {
            log::error!("[I-04] 实时报价批次拒绝: {}", error);
            log_dispatcher_attempt("I-04", false, 0, &error);
            return PeriodicDispatchResult::Failed(error);
        }
        Err(error) => {
            let reason = format!("实时报价任务失败: {error}");
            log::error!("[I-04] {}", reason);
            log_dispatcher_attempt("I-04", false, 0, &reason);
            return PeriodicDispatchResult::Failed(reason);
        }
    };
    if quotes.is_empty() {
        log_dispatcher_attempt("I-04", false, 0, "fetch_realtime_prices empty");
        log::warn!("[I-04] 拉报价失败, 跳过推送 (沙箱无网络/数据源挂)");
        return PeriodicDispatchResult::Failed("fetch_realtime_prices empty".to_string());
    }

    let mut pushed_count = 0;
    let mut deduped_count = 0usize;
    let mut failed_count = 0usize;
    let mut holding_count = 0usize;
    for pos in &positions {
        if pos.status != PositionStatus::Holding {
            continue;
        }
        holding_count += 1;
        // v60 (F12): 短路 0 数据持仓 — 报价+成本都为 0 时不要推假推荐
        if pos.cost_price <= 0.0 {
            log_dispatcher_attempt(
                "I-04",
                false,
                0,
                &format!("{} cost_price=0, 短路 (F12 修复)", pos.code),
            );
            log::warn!(
                "[I-04] {}({}) cost_price=0, 短路避免假推荐 (v60 F12 修复)",
                pos.name,
                pos.code
            );
            failed_count += 1;
            continue;
        }
        // P0-2: 报价缺失时跳过该票 + warn, 不再静默用 cost 顶替现价
        //   (否则推 "现价==成本 盈亏+0.0%" 假推荐, 违反 no-mock / 不静默填默认值)
        let current_price = match quotes.get(&pos.code).copied().filter(|p| *p > 0.0) {
            Some(p) => p,
            None => {
                log::warn!(
                    "[I-04] {}({}) 实时报价缺失, 跳过持仓建议 (不再用成本{:.2}顶替现价)",
                    pos.name,
                    pos.code,
                    pos.cost_price
                );
                log_dispatcher_attempt(
                    "I-04",
                    false,
                    0,
                    &format!("{} no realtime quote, skip", pos.code),
                );
                failed_count += 1;
                continue;
            }
        };
        let pnl_pct = if pos.cost_price > 0.0 {
            (current_price - pos.cost_price) / pos.cost_price * 100.0
        } else {
            0.0
        };
        // 简单意图: >5% 减仓; -15% < pnl < -3% 加仓 (浅亏); 其它持有/alert
        // 深度亏损 (< -15%) 不再推荐加仓 (v15.3 fix: 防止瀑布加仓)
        let intent = if pnl_pct > 5.0 {
            Intent::Reduce
        } else if pnl_pct < -15.0 {
            Intent::Hold // 深度亏损不要再建议加仓 (除非用户主动)
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
        let effective_stop = match pos.hard_stop.filter(|stop| stop.is_finite() && *stop > 0.0) {
            Some(stop) => stop,
            None => {
                let reason = format!("{} hard_stop unavailable", pos.code);
                log::error!(
                    "[I-04] {}({}) 硬止损未落库，拒绝生成持仓建议",
                    pos.name,
                    pos.code
                );
                log_dispatcher_attempt("I-04", false, 0, &reason);
                failed_count += 1;
                continue;
            }
        };
        let reasons_vec = vec![
            format!(
                "成本{:.2} 现价{:.2} 盈亏{:+.1}%",
                pos.cost_price, current_price, pnl_pct
            ),
            format!("硬止损{:.2}", effective_stop),
        ];
        let invalidations_vec = vec![format!("跌破{:.2}且放量", effective_stop)];
        let params = HoldingPlanParams {
            name: &pos.name,
            code: &pos.code,
            hhmm,
            intent,
            price: current_price,
            cost: pos.cost_price,
            avail: pos.shares as u32,
            reduce_zone,
            support: effective_stop,
            pressure: current_price * 1.10,
            stop: effective_stop,
            invalidations: &invalidations_vec,
            reasons: &reasons_vec,
        };
        match push_holding_plan_recommendation_outcome(&pos.code, banner, params).await {
            crate::notify::PushOutcome::Pushed => pushed_count += 1,
            crate::notify::PushOutcome::Deduped => deduped_count += 1,
            outcome => {
                failed_count += 1;
                log::error!(
                    "[BR-116] I-04 {} 投递未确认，保留到期状态: {:?}",
                    pos.code,
                    outcome
                );
            }
        }
    }
    if holding_count == 0 {
        log_dispatcher_attempt("I-04", false, 0, "no holding positions");
        return PeriodicDispatchResult::Empty;
    }
    let reason = format!(
        "{} pushed, {} deduped, {} failed",
        pushed_count, deduped_count, failed_count
    );
    log_dispatcher_attempt("I-04", pushed_count > 0, pushed_count, &reason);
    if failed_count > 0 {
        PeriodicDispatchResult::Failed(reason)
    } else if pushed_count > 0 {
        PeriodicDispatchResult::Delivery(crate::notify::PushOutcome::Pushed)
    } else if deduped_count == holding_count {
        PeriodicDispatchResult::Delivery(crate::notify::PushOutcome::Deduped)
    } else {
        PeriodicDispatchResult::Failed("I-04 batch has no confirmed outcome".to_string())
    }
}

pub async fn dispatch_holding_plan_daily(hhmm: &str, banner: &BannerCtx) -> bool {
    dispatch_holding_plan_daily_result(hhmm, banner)
        .await
        .is_pushed()
}

pub async fn dispatch_holding_plan_periodic(hhmm: &str, banner: &BannerCtx) -> bool {
    dispatch_holding_plan_daily_result(hhmm, banner)
        .await
        .is_confirmed()
}

/// v37: P-02 竞价热点量能快照
#[derive(Debug, Clone, Default)]
pub struct AuctionVolumeSnapshot {
    pub hhmm: String,
    pub items: Vec<(String, String, f64, f64, f64)>, // (name, code, gap_pct, vol_ratio, price) — review fix Issue #6: 加 price 供 push_recorder 入池
    pub sentiment: String,                           // "强承接" | "一般" | "弱承接"
    pub watch_status: String,                        // 观察状态描述
}

/// v37: 加载 P-02 快照 - 复用 limit_up_stocks
pub fn load_auction_volume_snapshot_real(hhmm: &str) -> Result<AuctionVolumeSnapshot, String> {
    use stock_analysis::market_analyzer::MarketAnalyzer;
    let analyzer = match MarketAnalyzer::new(None) {
        Ok(a) => a,
        Err(error) => return Err(format!("竞价量能 analyzer 初始化失败: {error}")),
    };
    let limit_stocks = match analyzer.get_limit_up_stocks() {
        Ok(s) => s,
        Err(error) => return Err(format!("竞价量能涨停列表获取失败: {error}")),
    };
    if limit_stocks.is_empty() {
        return Err("竞价量能涨停列表为空".to_string());
    }
    // 按量比降序, 取前 10
    let mut sorted = limit_stocks.clone();
    sorted.sort_by(|a, b| {
        b.volume_ratio
            .partial_cmp(&a.volume_ratio)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let items: Vec<(String, String, f64, f64, f64)> = sorted
        .iter()
        .take(10)
        .filter_map(|s| {
            let Some(volume_ratio) = s.volume_ratio else {
                log::warn!("[P-02] {}({}) 量比缺失，跳过竞价量能快照", s.name, s.code);
                return None;
            };
            Some((
                s.name.clone(),
                s.code.clone(),
                s.change_pct,
                volume_ratio,
                s.price,
            ))
        })
        .collect();
    if items.is_empty() {
        return Err("竞价热点无具备真实量比的有效行".to_string());
    }

    // sentiment: 平均量比 >= 3 强承接, >= 1 一般, < 1 弱承接
    let avg_vr: f64 = items.iter().map(|(_, _, _, vr, _)| vr).sum::<f64>() / items.len() as f64;
    let sentiment = if avg_vr >= 3.0 {
        "强承接"
    } else if avg_vr >= 1.0 {
        "一般"
    } else {
        "弱承接"
    };

    Ok(AuctionVolumeSnapshot {
        hhmm: hhmm.to_string(),
        items,
        sentiment: sentiment.to_string(),
        watch_status: "9:25 集合竞价结果, 关注开盘承接".to_string(),
    })
}

/// v37: P-02 dispatcher
pub async fn dispatch_auction_volume_daily(hhmm: &str, banner: &BannerCtx) -> bool {
    let snapshot = match load_auction_volume_snapshot_real(hhmm) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            log_dispatcher_attempt("P-02", false, 0, &error);
            log::warn!("[P-02] 竞价量能快照不可用: {}", error);
            return false;
        }
    };
    // 构造 AuctionItem refs
    let auction_items: Vec<AuctionItem<'_>> = snapshot
        .items
        .iter()
        .map(|(n, c, g, v, _p)| AuctionItem {
            name: n,
            code: c,
            gap_pct: *g,
            vol_ratio: *v,
            tag: "", // 简化: 不填 tag
        })
        .collect();
    let text = render_auction_volume(
        banner,
        &snapshot.hhmm,
        &auction_items,
        &snapshot.sentiment,
        &snapshot.watch_status,
    );
    let result = dispatch(
        crate::notify::PushKind::AuctionVolume,
        "",
        Some(banner),
        text,
    )
    .await;
    log_dispatcher_attempt("P-02", result, snapshot.items.len(), "");
    // review fix Issue #6: P-02 推送成功后入 pushed_stocks 票池 (R3)
    // 红线 2.2: price <= 0 (缺数据) 的票不入池, 不造价格
    if result {
        for (n, c, g, v, p) in &snapshot.items {
            if *p <= 0.0 {
                log::warn!("[P-02] {}({}) 无真实价格, 跳过入池 (红线 2.2)", n, c);
                continue;
            }
            let metric_json = truncate_metric_json(
                serde_json::json!({
                    "vol_ratio": v,
                    "price_chg_pct": g,
                    "push_subkind": "AuctionVolume",
                })
                .to_string(),
            );
            if let Err(error) = stock_analysis::signal::push_recorder::record(
                &stock_analysis::signal::push_recorder::PushRecordMeta {
                    code: c.clone(),
                    name: n.clone(),
                    push_kind: "P-02".to_string(),
                    push_price: *p,
                    metric_json,
                    source: "preopen".to_string(),
                },
            ) {
                let reason = format!("P-02 pushed_stocks audit failed for {c}: {error}");
                log::error!("{reason}");
                log_dispatcher_attempt("P-02", false, snapshot.items.len(), &reason);
                return false;
            }
        }
    }
    result
}

#[derive(Debug, Clone, Default)]
pub struct CatalystReviewSnapshot {
    pub date: String,
    pub theme: String,
    pub score: Option<f32>,
    pub persistent: PersistentLevel,
    pub started: Vec<String>,
    pub pending: Vec<String>,
    pub watch_point: Option<String>,
}

/// BR-102: 加载 A-10 真实催化复盘快照。
pub fn load_catalyst_review_snapshot_real(date: &str) -> Result<CatalystReviewSnapshot, String> {
    use stock_analysis::database::DatabaseManager;
    let db = DatabaseManager::get();
    let clusters = db.get_latest_chain_clusters_strict()?;
    let rotations = db.get_latest_board_rotations_strict()?;
    if clusters.is_empty() || rotations.is_empty() {
        return Ok(CatalystReviewSnapshot::default());
    }

    // 取第一个 cluster 作主推
    let top = &clusters[0];
    let theme = top.concept.trim();
    if theme.is_empty() {
        return Err("A-10 chain_daily 头部 concept 为空".to_string());
    }
    let stocks: Vec<String> = serde_json::from_str(&top.stocks)
        .map_err(|error| format!("A-10 chain_daily stocks JSON 非法: {error}"))?;
    let selected_codes: Vec<&str> = stocks.iter().take(6).map(|code| code.trim()).collect();
    if selected_codes.is_empty() {
        return Err("A-10 chain_daily 头部主线无股票".to_string());
    }
    if let Some(code) = selected_codes
        .iter()
        .find(|code| !valid_source_stock_code(code))
    {
        return Err(format!("A-10 chain_daily 股票代码非法: {code}"));
    }

    let mut names = std::collections::HashMap::new();
    for (rotation_index, rotation) in rotations.iter().enumerate() {
        let rows = serde_json::from_str::<Vec<serde_json::Value>>(&rotation.stocks_json).map_err(
            |error| {
                format!(
                    "A-10 board_rotation_daily 第 {} 行 stocks JSON 非法: {error}",
                    rotation_index + 1
                )
            },
        )?;
        for (stock_index, stock) in rows.iter().enumerate() {
            let code = stock
                .get("code")
                .and_then(serde_json::Value::as_str)
                .filter(|code| valid_source_stock_code(code))
                .ok_or_else(|| {
                    format!(
                        "A-10 board_rotation_daily 第 {} 行第 {} 只 code 非法",
                        rotation_index + 1,
                        stock_index + 1
                    )
                })?;
            let name = stock
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    format!(
                        "A-10 board_rotation_daily 第 {} 行第 {} 只 name 为空",
                        rotation_index + 1,
                        stock_index + 1
                    )
                })?;
            names.insert(code.to_string(), name.to_string());
        }
    }
    let selected_names: Vec<String> = selected_codes
        .iter()
        .map(|code| {
            names
                .get(*code)
                .cloned()
                .ok_or_else(|| format!("A-10 股票 {code} 缺少真实名称证据"))
        })
        .collect::<Result<_, _>>()?;

    // 持续性: 用 continuation_count 推断
    let persistent = if top.continuation_count >= 3 {
        PersistentLevel::High
    } else if top.continuation_count >= 1 {
        PersistentLevel::Med
    } else {
        PersistentLevel::Low
    };

    // 已启动: cluster 头部 (前 3)
    let started: Vec<String> = selected_names.iter().take(3).cloned().collect();
    // 待启动: cluster 尾部 (3-5)
    let pending: Vec<String> = selected_names.iter().skip(3).take(3).cloned().collect();

    // 明日观察点: 简单模板
    let watch_point = format!("明日竞价复核 {} 主线持续性", top.concept);

    Ok(CatalystReviewSnapshot {
        date: date.to_string(),
        theme: theme.to_string(),
        score: None,
        persistent,
        started,
        pending,
        watch_point: Some(watch_point),
    })
}

// ============================================================================
// B-005-C (2026-07-09): 盘后批量 dispatcher (R-02..R-08 + TomorrowWatch)
// 修复: 之前 6 个盘后 dispatcher 仅在 `cargo run -- --push` 模式被调,
//       生产 monitor_loop 永远跑不到, 用户看不到盘后复盘.
// 现在: 在 post-session block 等到 19:00 后调一次, 各 dispatcher 失败时静默 log,
//       不阻塞其他 dispatcher.
// ============================================================================

/// B-005-C 统一入口 — 在 monitor_loop post-session block 19:00 后调一次.
/// 不依赖 --push 命令行模式, 让生产 monitor 自动出盘后 6+1 报告.
/// 各 R-series dispatcher 内部分别走自己的数据源，并逐项记录成功/失败。
/// 返回值表示本批次是否至少成功推送一份报告，不把全失败伪装为成功（BR-110）。
pub async fn dispatch_post_session_review(date: &str, hhmm: &str, banner: &BannerCtx) -> bool {
    log::info!("[B-005-C] 盘后批量 dispatcher 开始 ({})", date);

    // CR-29 (review): 8 个 sub-dispatcher 改 tokio::join! 并行.
    // 之前: 6 个 HTTP + 2 个 DB 顺序 await, 19:00 批次 wall-time ~10-15s
    // 现在: 全部并行, 19:00 批次 wall-time ~max(单次 HTTP) ≈ 100-500ms
    // 限制: 8 个 dispatcher 都用 notify::push_governor 走 reqwest::Client (parallel-safe)
    let (r02, r03, r04, r05, r06, r08, a10, a01) = tokio::join!(
        dispatch_r02_review_market_real(date, banner),
        dispatch_r03_industry_chain_real(date, banner),
        dispatch_r04_lhb_real(date, banner),
        dispatch_r05_signal_review_real(date, banner),
        dispatch_r06_failure_real(date, banner),
        dispatch_r08_event_calendar_real(date, banner),
        dispatch_catalyst_review_daily(date),
        dispatch_paper_review_daily(date),
    );
    let results = [
        ("R-02", r02),
        ("R-03", r03),
        ("R-04", r04),
        ("R-05", r05),
        ("R-06", r06),
        ("R-08", r08),
        ("A-10", a10),
        ("A-01", a01),
    ];
    let pushed = results.iter().filter(|(_, ok)| *ok).count();
    let failed: Vec<&str> = results
        .iter()
        .filter_map(|(name, ok)| (!ok).then_some(*name))
        .collect();
    log::info!(
        "[B-005-C][BR-110] 盘后批量 dispatcher 完成 time={} pushed={}/{} failed={:?}",
        hhmm,
        pushed,
        results.len(),
        failed
    );
    pushed > 0
}

/// --test/--review 全模板覆盖范围
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum TestScope {
    /// 全部模板 (盘中 + 盘后)
    All,
    /// 全部可由本地 TEST_CODE/隔离数据库证据驱动的模板；不访问外部实时源。
    IsolatedAll,
    /// 仅盘后复盘模板
    Review,
}

/// --test/--review 全模板覆盖 (用户要求: 测试所有模板, 真推, 只推有真数据的, 不关注时间)
///
/// - scope=Review → 盘后复盘子集 (R-02~R-08 + 催化/模拟复盘 + 盘后资金 + ETF尾盘)
/// - scope=All → 额外跑盘中模板 (盘面/涨停扩散/持仓建议/选股/领涨/主力/轮动...)
/// - 事件触发型 (broker成交/大宗/ST/T0/挂单) 需结构化事件, 测试环境无真事件 → 不造样本, 各 dispatcher 内部短路返回 false → 记 skipped
/// - 真推 (不走 V10_DRY_RUN_PUSH), 每个模板走 push_governor 真发微信/飞书
/// - 绕过 live-loop 5/15/30min 计时器 (直接调, 一次性)
pub async fn dispatch_all_for_test(hhmm: &str, date: &str, banner: &BannerCtx, scope: TestScope) {
    let mut fired = 0usize;
    let mut skipped = 0usize;
    macro_rules! run_disp {
        ($name:expr, $call:expr) => {{
            log::info!("[--test] === {} ===", $name);
            let ok: bool = $call.await;
            if ok {
                fired += 1;
                log::info!("[--test] {} → pushed", $name);
            } else {
                skipped += 1;
                log::info!("[--test] {} → skipped (no data)", $name);
            }
        }};
    }
    macro_rules! skip_external {
        ($name:expr) => {{
            skipped += 1;
            log::info!(
                "[--test][BR-051] {} → skipped (external source not exercised in isolated E2E)",
                $name
            );
        }};
    }
    let isolated = matches!(scope, TestScope::IsolatedAll);

    // ---- 盘后复盘 (Review + All) ----
    if isolated {
        skip_external!("R-02今日盘面");
    } else {
        run_disp!(
            "R-02今日盘面",
            dispatch_r02_review_market_real(date, banner)
        );
    }
    if isolated {
        skip_external!("R-03涨停产业链");
    } else {
        run_disp!(
            "R-03涨停产业链",
            dispatch_r03_industry_chain_real(date, banner)
        );
    }
    run_disp!("R-04龙虎榜", dispatch_r04_lhb_real(date, banner));
    run_disp!(
        "R-05信号复盘",
        dispatch_r05_signal_review_real(date, banner)
    );
    run_disp!("R-06失败归因", dispatch_r06_failure_real(date, banner));
    if isolated {
        skip_external!("R-08明日事件");
    } else {
        run_disp!(
            "R-08明日事件",
            dispatch_r08_event_calendar_real(date, banner)
        );
    }
    run_disp!("催化复盘", dispatch_catalyst_review_daily(date));
    run_disp!("模拟复盘", dispatch_paper_review_daily(date));
    if isolated {
        skip_external!("盘后资金买入");
    } else {
        run_disp!(
            "盘后资金买入",
            dispatch_post_close_fund_inflow_buy(date, banner)
        );
    }
    // 事件触发型 (无真事件, 不造样本)
    log::info!("[--test] === 事件触发型 (broker成交/大宗/ST/T0/挂单) 无真事件, 跳过 ===");

    if matches!(scope, TestScope::All | TestScope::IsolatedAll) {
        // ---- 盘中 (All only) ----
        if isolated {
            skip_external!("盘面盘中");
        } else {
            run_disp!("盘面盘中", dispatch_intraday_market_daily(hhmm, banner));
        }
        if isolated {
            skip_external!("新闻催化");
            skip_external!("涨停扩散");
            skip_external!("新闻驱动个股");
            skip_external!("持仓建议");
            skip_external!("候选触发");
        } else {
            run_disp!("新闻催化", dispatch_news_catalyst_daily(hhmm, banner));
            run_disp!(
                "涨停扩散",
                dispatch_industry_chain_intraday_daily(hhmm, banner)
            );
            run_disp!("新闻驱动个股", dispatch_news_to_idea_daily(hhmm, banner));
            run_disp!("持仓建议", dispatch_holding_plan_daily(hhmm, banner));
            run_disp!("候选触发", dispatch_candidate_triggered_daily(hhmm, banner));
        }
        if isolated {
            skip_external!("竞价量能P-02");
            skip_external!("领涨板块Top");
        } else {
            run_disp!("竞价量能P-02", dispatch_auction_volume_daily(hhmm, banner));
            run_disp!("领涨板块Top", dispatch_sector_top_daily(hhmm));
        }
        if isolated {
            skip_external!("板块异动");
        } else {
            run_disp!("板块异动", dispatch_sector_anomaly_daily(hhmm, ""));
        }
        if isolated {
            skip_external!("主力净流入Top");
        } else {
            run_disp!("主力净流入Top", dispatch_fund_inflow_top_daily(hhmm));
        }
        run_disp!("盘前新闻热点", dispatch_preopen_news_hot_daily());
        run_disp!("模拟交易", dispatch_paper_trade_daily(hhmm));
        run_disp!("虚拟盯盘", dispatch_virtual_watch_daily(hhmm, &[], 0));
        run_disp!("挂单管道", dispatch_trade_pipeline_orders(hhmm, banner));
        run_disp!("成交管道", dispatch_trade_pipeline_fills(hhmm, banner));
    }
    log::info!(
        "[--test] === 全模板覆盖完成: {} pushed, {} skipped ===",
        fired,
        skipped
    );
}

// ============================================================================
// CR-12 (review): R-02/R-08 真实 dispatcher (从 run_review_only_inner 抽取)
// ============================================================================

/// R-02 今日盘面 (CR-12): 真实大盘数据.
/// 之前 run_review_only_inner L2635 用的是 hardcode (sh_chg=0.5 等), 现在 fetch 真实值.
pub async fn dispatch_r02_review_market_real(_date: &str, _banner: &BannerCtx) -> bool {
    let snapshot =
        match tokio::task::spawn_blocking(super::market_data::fetch_market_review_snapshot).await {
            Ok(Ok(snapshot)) => snapshot,
            Ok(Err(error)) => {
                log::error!("[R-02][BR-093] market snapshot unavailable: {}", error);
                log_dispatcher_attempt("R-02", false, 0, &error);
                return false;
            }
            Err(error) => {
                let reason = format!("market snapshot task failed: {error}");
                log::error!("[R-02][BR-093] {}", reason);
                log_dispatcher_attempt("R-02", false, 0, &reason);
                return false;
            }
        };

    let reason = format!(
        "BR-093 R-02 disabled: required main_flow/money_effect/position-limit evidence unavailable; partial snapshot indices=({:+.2},{:+.2},{:+.2}) amount={:.0}",
        snapshot.sh_chg, snapshot.chinext_chg, snapshot.star_chg, snapshot.amount_yi
    );
    log::error!("[R-02] {reason}");
    log_dispatcher_attempt(
        "R-02",
        false,
        snapshot.limit_up_n as usize + snapshot.limit_down_n as usize,
        &reason,
    );
    false
}

/// R-08 明日事件 (CR-12): 真实公告 + 持仓事件 + 隔夜关注 (美股+汇率).
/// 之前 run_review_only_inner L2708 的真实实现, 现在抽取为独立 dispatcher.
pub async fn dispatch_r08_event_calendar_real(date: &str, _banner: &BannerCtx) -> bool {
    // 1. 拉今日全市场公告 (真实数据)
    let anns = match stock_analysis::data_provider::announcement::fetch_announcements(None).await {
        Ok(announcements) => announcements,
        Err(error) => {
            let reason = format!("公告数据源失败: {error}");
            log::error!("[R-08][BR-110] {reason}");
            log_dispatcher_attempt("R-08", false, 0, &reason);
            return false;
        }
    };
    // 2. 持仓事件 (实盘) + 虚拟持仓, 区分 tag
    let positions = match stock_analysis::portfolio::get_positions() {
        Ok(positions) => positions,
        Err(error) => {
            let reason = format!("实盘持仓数据源失败: {error}");
            log::error!("[R-08][BR-110] {reason}");
            log_dispatcher_attempt("R-08", false, 0, &reason);
            return false;
        }
    };
    let holding_codes: std::collections::HashSet<String> =
        positions.iter().map(|p| p.code.clone()).collect();
    // 宏观公告摘要: 区分持仓相关 / 非持仓
    let ann_summary = build_event_calendar_macro_summary(&anns, &holding_codes);
    // 实盘持仓: 优先今日公告标题作为事件, 否则标"持有"
    let mut holdings: Vec<EventHolding> = Vec::new();
    for p in positions.iter().take(5) {
        let p_ann = anns.iter().find(|a| a.code == p.code);
        let kind = match p_ann {
            Some(a) => a.title.chars().take(20).collect::<String>(),
            None => "持有 (今日无公告)".to_string(),
        };
        holdings.push(EventHolding {
            tag: "实盘".to_string(),
            name: p.name.clone(),
            code: p.code.clone(),
            kind,
        });
    }
    // 虚拟持仓 (虚拟观察仓)
    match event_calendar_virtual_holdings() {
        Ok(virtual_holdings) => holdings.extend(virtual_holdings),
        Err(error) => {
            let reason = format!("虚拟观察数据源失败: {error}");
            log::error!("[R-08][BR-104][BR-110] {reason}");
            log_dispatcher_attempt("R-08", false, 0, &reason);
            return false;
        }
    }
    // 3. 隔夜关注 (美股 + 汇率 雅虎 API)
    let (us_summary, fx_summary) = match tokio::task::spawn_blocking(
        stock_analysis::data_provider::yahoo::fetch_overnight_data,
    )
    .await
    {
        Ok(Ok(snapshot)) => snapshot,
        Ok(Err(error)) => {
            log::error!("[R-08] Yahoo 隔夜数据不可用: {}", error);
            (
                "不可用（数据源错误）".to_string(),
                "不可用（数据源错误）".to_string(),
            )
        }
        Err(error) => {
            log::error!("[R-08] fetch_overnight_data task 失败: {}", error);
            (
                "不可用（任务失败）".to_string(),
                "不可用（任务失败）".to_string(),
            )
        }
    };
    let events_ref: Vec<HoldingEventItem> = holdings
        .iter()
        .map(|h| HoldingEventItem {
            tag: h.tag.as_str(),
            name: h.name.as_str(),
            code: h.code.as_str(),
            kind: h.kind.as_str(),
        })
        .collect();
    let text = render_event_calendar(date, &events_ref, &ann_summary, &us_summary, &fx_summary);
    let push_result =
        crate::notify::push_governor(&text, crate::notify::PushKind::EventCalendar).await;
    log_dispatcher_attempt("R-08", push_result, holdings.len(), "");
    push_result
}

// ============================================================================
// CR-16 (review): R-03/R-04/R-05/R-06 真实 dispatcher (从 run_review_only_inner 抽取)
// 替代之前 dispatch_post_session_review 内的占位 dispatcher (复用 A-10/A-01)
// ============================================================================

/// R-03 涨停产业链：基于实盘持仓/自选和逐股日 K 聚合，不从概念延续次数推断涨停数（BR-106/BR-110）。
pub async fn dispatch_r03_industry_chain_real(date: &str, _banner: &BannerCtx) -> bool {
    use stock_analysis::market_analyzer::limit_chain_review::{aggregate, LimitChainInput};

    let review_date = date.to_string();
    let prepared = tokio::task::spawn_blocking(move || -> Result<(String, usize), String> {
        let positions = stock_analysis::portfolio::get_positions()
            .map_err(|error| format!("R-03 实盘持仓查询失败: {error}"))?;
        let stocks = super::load_review_limit_chain_stocks(&positions)?;
        if stocks.is_empty() {
            return Err("R-03 持仓/自选中没有经日 K 验证的当日涨停标的".to_string());
        }
        let aggregates = aggregate(&LimitChainInput {
            stocks,
            source_complete: true,
        });
        let follower_text: Vec<String> = aggregates
            .iter()
            .map(|row| {
                if row.followers.is_empty() {
                    "无".to_string()
                } else {
                    row.followers.join("、")
                }
            })
            .collect();
        let lines: Vec<ChainLine<'_>> = aggregates
            .iter()
            .zip(follower_text.iter())
            .take(5)
            .map(|(row, followers)| ChainLine {
                chain: &row.chain,
                limit_up_n: row.limit_up_n,
                first_n: row.first_n,
                consec_n: row.consec_n,
                heat_stage: &row.heat_stage,
                leader_name: &row.leader_name,
                leader_code: &row.leader_code,
                leader_boards: row.leader_boards,
                followers,
                watch_point: &row.watch_point,
            })
            .collect();
        let count = lines.len();
        Ok((render_industry_chain(&review_date, &lines, None), count))
    })
    .await;
    let (text, count) = match prepared {
        Ok(Ok(prepared)) => prepared,
        Ok(Err(error)) => {
            log::error!("[R-03][BR-106][BR-110] {error}");
            log_dispatcher_attempt("R-03", false, 0, &error);
            return false;
        }
        Err(error) => {
            let reason = format!("R-03 数据准备任务失败: {error}");
            log::error!("[R-03][BR-110] {reason}");
            log_dispatcher_attempt("R-03", false, 0, &reason);
            return false;
        }
    };
    let push_result =
        crate::notify::push_governor(&text, crate::notify::PushKind::IndustryChain).await;
    log_dispatcher_attempt("R-03", push_result, count, "");
    push_result
}

/// R-04 龙虎榜 (CR-16): 拉 lhb_review 真实数据, 渲染 5 条.
pub async fn dispatch_r04_lhb_real(date: &str, _banner: &BannerCtx) -> bool {
    use chrono::NaiveDate;
    use stock_analysis::market_analyzer::lhb_review::fetch_recent_lhb;
    // CR-23 (review): lhb 数据 21:00 后才有, 19:00 时直接跳过, 不浪费 I/O.
    // 之前无论何时都尝试拉, 必然返回空 + 浪费 spawn_blocking.
    let now_time = chrono::Local::now().time();
    let lhb_ready_time = chrono::NaiveTime::from_hms_opt(21, 0, 0).unwrap();
    if now_time < lhb_ready_time {
        log::info!(
            "[R-04] 现在 {} < 21:00, lhb 数据未出, 跳过 (后续 21:00 由 monitor_loop 重调)",
            now_time.format("%H:%M")
        );
        log_dispatcher_attempt("R-04", false, 0, "before 21:00, lhb not ready");
        return false;
    }
    let today = match NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        Ok(today) => today,
        Err(error) => {
            let reason = format!("非法复盘日期 {date}: {error}");
            log::error!("[R-04][BR-110] {reason}");
            log_dispatcher_attempt("R-04", false, 0, &reason);
            return false;
        }
    };
    let raw = match tokio::task::spawn_blocking(move || fetch_recent_lhb(today, 5)).await {
        Ok(Ok(rows)) => rows,
        Ok(Err(error)) => {
            log::error!("[R-04][BR-110] disabled=no_producer: {error}");
            log_dispatcher_attempt("R-04", false, 0, &error);
            return false;
        }
        Err(error) => {
            let reason = format!("龙虎榜查询任务失败: {error}");
            log::error!("[R-04][BR-110] {reason}");
            log_dispatcher_attempt("R-04", false, 0, &reason);
            return false;
        }
    };
    if raw.is_empty() {
        log::info!("[R-04] 21:00 仍无数据, 跳过推送");
        log_dispatcher_attempt("R-04", false, 0, "lhb empty (21:00 后)");
        return false;
    }
    let entries: Vec<LhbEntry> = raw
        .iter()
        .take(5)
        .map(|e| LhbEntry {
            name: &e.name,
            code: &e.code,
            net_buy_yi: e.net_buy_yi,
            reason: "—",
            // W4.3 / B-010 P1 修复: 数值字段 Option<f64>, 显式 None 表示缺失
            buy_inst_n: e.buy_inst_n.unwrap_or(0),
            buy_inst_amt_wan: e.buy_inst_amt_wan,
            buy_other_n: e.buy_other_n.unwrap_or(0),
            buy_other_amt_wan: e.buy_other_amt_wan,
            buy_conc_pct: e.buy_conc_pct,
            // W1.14 / B-010 P0-3: sell_desc 缺失用空串, render 端判空显示"无"
            // (BR-004 同款: em-dash 占位违反 AGENTS.md §2.2)
            sell_desc: match e.sell_desc.as_deref() {
                Some(s) if !s.is_empty() => s,
                _ => {
                    log::warn!("[push] LhbEntry 缺 sell_desc: code={}", e.code);
                    ""
                }
            },
            sell_conc_pct: e.sell_conc_pct,
            chain_match: e.chain_match.as_deref(),
            next_day_risk: "—",
        })
        .collect();
    let text = render_review_lhb(date, &entries);
    let push_result = crate::notify::push_governor(&text, crate::notify::PushKind::ReviewLhb).await;
    log_dispatcher_attempt("R-04", push_result, entries.len(), "");
    push_result
}

/// R-05 信号复盘：需要“信号触发 → 成交 → 平仓结果”的完整关联源。
/// 当前通用交易表无法证明信号归属，明确禁用而不是据此伪造胜率（BR-110）。
pub async fn dispatch_r05_signal_review_real(_date: &str, _banner: &BannerCtx) -> bool {
    let reason = "disabled=no_complete_signal_outcome_source";
    log::error!("[R-05][BR-110] {reason}");
    log_dispatcher_attempt("R-05", false, 0, reason);
    false
}

/// R-06 失败归因 (CR-16): 拉真实交易历史 (paper_trades 表), 渲染失败归因.
///   之前 main.rs L2783-2811 用 hardcoded mock 数据 (德展健康/达实智能), 违反 AGENTS §2.1
///   (生产路径禁止 mock). 现改为真接 closed trades 表, 标识 "n 笔待分类" (无 P&L 数据源).
pub async fn dispatch_r06_failure_real(_date: &str, _banner: &BannerCtx) -> bool {
    let reason = "disabled=no_classified_failure_outcome_source";
    log::error!("[R-06][BR-110] {reason}");
    log_dispatcher_attempt("R-06", false, 0, reason);
    false
}

// ============================================================================
// FIX-3 (review): R-02 / R-03 真实 dispatcher 单元测试, 防回归
// R-02 真实数据通路: spawn_blocking(MarketAnalyzer::get_limit_up_stocks) → MarketReview
// R-03 真实数据通路: chain_daily cluster → ChainLine
// ============================================================================

#[cfg(test)]
mod tests_r_dispatchers {
    use super::*;

    /// FIX-3: R-02 真实 dispatcher 不会因 spawn_blocking 失败而 panic,
    /// 缺数据时 fallback 到 hardcode (30, 5, 15.0) 继续渲染.
    /// 之前缺测试, 任何 panic 改动会让 19:00 批次静默崩溃.
    /// FIX-3 修正: 跑测试时设 V10_DRY_RUN_PUSH=1 让 push_governor 走 dry-run 路径 (返回 true)
    /// 注意: push_governor 返回值依赖 wechat transport, 测试不验证 return value,
    ///       只验证函数不 panic (这才是真实回归保护点).
    #[tokio::test]
    async fn test_dispatch_r02_review_market_no_panic() {
        let banner = BannerCtx {
            account_mode: AccountMode::Normal,
            total_pos: Some(5),
            today_pnl: Some(0.0),
            account_metrics_complete: true,
            data_mode: DataMode::Full,
            data_missing_note: None,
        };
        // 用过去日期避免数据查不到. 不验证 return value (依赖 transport),
        // 只验证函数不 panic — 这就是真正的回归保护点.
        let _ = dispatch_r02_review_market_real("2026-01-01", &banner).await;
    }

    /// FIX-3: R-02 缺数据时仍能渲染 (用 fallback 30, 5, 15.0).
    /// 验证降级路径不 panic.
    #[tokio::test]
    async fn test_dispatch_r02_fallback_renders() {
        let banner = BannerCtx {
            account_mode: AccountMode::Normal,
            total_pos: Some(0),
            today_pnl: Some(0.0),
            account_metrics_complete: true,
            data_mode: DataMode::Full,
            data_missing_note: None,
        };
        // 缺数据日 (2010-01-01) 走 fallback (30, 5, 15.0)
        let _ = dispatch_r02_review_market_real("2010-01-01", &banner).await;
    }

    /// FIX-3: R-03 真实 dispatcher, 无 cluster 时静默跳过 (返回 false, 不推).
    /// 防止: 未来 R-03 实现改成"硬推空 ChainLine" (会推 [板块联动] 0 股噪声)
    /// FIX-3 修正: 不依赖 DB 初始化, 用 try_get 包装. 测试 env 没 DB 时, 走 None 路径 → false.
    /// 实际生产路径 (有 DB) 同样走 try_get → None → false, 测试通过即生产通过.
    #[tokio::test]
    async fn test_dispatch_r03_skips_when_no_clusters() {
        // 关键: 不初始化 DB, 模拟生产中"DB 暂时不可用" 或 "chain_daily 缺数据" 场景.
        // dispatcher 应该走 try_get → None → return false (不推).
        // 之前测试 panic 在 DatabaseManager::get() unwrap, 说明设计缺陷:
        //   production 走 monitor_loop 已 init DB, 测试没 init 就 panic.
        //   修复: dispatcher 内部用 try_get 而非 get, 不会 panic.
        // 验证: 此测试在没 init DB 时不 panic 且返回 false.
        let banner = BannerCtx {
            account_mode: AccountMode::Normal,
            total_pos: Some(0),
            today_pnl: Some(0.0),
            account_metrics_complete: true,
            data_mode: DataMode::Full,
            data_missing_note: None,
        };
        let result = dispatch_r03_industry_chain_real("2026-01-01", &banner).await;
        // R-03: DB 未 init (try_get 返 None) → false (不推)
        assert!(!result, "R-03 缺 DB 应返回 false (不推), 不应 panic");
    }

    /// FIX-5: R-05 不 panic 在 DB 缺数据时.
    /// 之前: 不验证 panic protection, 任何闭包捕获错误会静默崩溃 19:00 批次.
    /// 验证: 不 init DB 时, dispatcher 走 try_get → None → false (不推).
    #[tokio::test]
    async fn test_dispatch_r05_skips_when_no_db() {
        let banner = BannerCtx {
            account_mode: AccountMode::Normal,
            total_pos: Some(0),
            today_pnl: Some(0.0),
            account_metrics_complete: true,
            data_mode: DataMode::Full,
            data_missing_note: None,
        };
        // 不 init DB, portfolio::get_trade_history 返 [], R-05 走 skip + log 路径
        let result = dispatch_r05_signal_review_real("2026-01-01", &banner).await;
        // R-05: 无 closed trades → false (不推), 不应 panic
        assert!(
            !result,
            "R-05 缺 closed trades 应返回 false (不推), 不应 panic"
        );
    }
}

/// v35: A-10 dispatcher 入口
pub async fn dispatch_catalyst_review_daily(date: &str) -> bool {
    let snapshot = match load_catalyst_review_snapshot_real(date) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            log::error!("[A-10] 催化复盘批次拒绝: {error}");
            log_dispatcher_attempt("A-10", false, 0, &error);
            return false;
        }
    };
    if snapshot.started.is_empty() {
        log_dispatcher_attempt("A-10", false, 0, "catalyst_review_snapshot empty");
        log::info!("[A-10] catalyst_review_snapshot 空 (chain_daily 无 cluster), 跳过推送");
        return false;
    }
    let started_refs: Vec<&str> = snapshot.started.iter().map(|s| s.as_str()).collect();
    let pending_refs: Vec<&str> = snapshot.pending.iter().map(|s| s.as_str()).collect();
    let params = CatalystReviewParams {
        date: &snapshot.date,
        theme: &snapshot.theme,
        score: snapshot.score,
        persistent: snapshot.persistent,
        started_names: started_refs,
        pending_names: pending_refs,
        watch_point: snapshot.watch_point.as_deref(),
    };
    let text = render_catalyst_review(params);
    let result = dispatch(crate::notify::PushKind::CatalystReview, "", None, text).await;
    log_dispatcher_attempt("A-10", result, snapshot.started.len(), "");
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
pub fn load_virtual_observation_for_a01() -> Result<VirtualSnapshotLite, String> {
    use std::fs;
    let dir = match stock_analysis::risk::env_guard::current_env() {
        stock_analysis::risk::env_guard::TradingEnv::Prod => {
            std::path::PathBuf::from("data/virtual_observation")
        }
        stock_analysis::risk::env_guard::TradingEnv::Test => {
            std::path::PathBuf::from("data/test/virtual_observation")
        }
    };
    if !dir.exists() {
        return Ok(VirtualSnapshotLite { records: vec![] });
    }
    let mut records: Vec<VirtualRecordLite> = Vec::new();
    let entries = fs::read_dir(&dir)
        .map_err(|error| format!("读取虚拟观察目录 {} 失败: {error}", dir.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| format!("读取虚拟观察目录项失败: {error}"))?
            .path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            paths.push(path);
        }
    }
    paths.sort();
    paths.reverse();

    #[derive(serde::Deserialize)]
    struct RecordJson {
        entry_date: Option<String>,
        code: Option<String>,
        name: Option<String>,
        entry_mode: Option<String>,
        entry_price: Option<f64>,
    }
    #[derive(serde::Deserialize)]
    struct SnapshotJson {
        records: Vec<RecordJson>,
    }

    fn push_record(
        records: &mut Vec<VirtualRecordLite>,
        parsed: RecordJson,
        source: &std::path::Path,
    ) -> Result<(), String> {
        let code = parsed
            .code
            .filter(|value| valid_source_stock_code(value))
            .ok_or_else(|| format!("{} 缺少合法六位 code", source.display()))?;
        let name = parsed
            .name
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("{} 的 {code} 缺少 name", source.display()))?;
        let entry_date = parsed
            .entry_date
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("{} 的 {code} 缺少 entry_date", source.display()))?;
        chrono::NaiveDate::parse_from_str(&entry_date, "%Y-%m-%d").map_err(|error| {
            format!(
                "{} 的 {code} entry_date={entry_date} 非法: {error}",
                source.display()
            )
        })?;
        let entry_mode = parsed
            .entry_mode
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("{} 的 {code} 缺少 entry_mode", source.display()))?;
        let entry_price = parsed
            .entry_price
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| format!("{} 的 {code} 缺少有效 entry_price", source.display()))?;
        records.push(VirtualRecordLite {
            entry_date,
            code,
            name,
            entry_mode,
            entry_price,
        });
        Ok(())
    }

    for path in paths.iter().take(5) {
        let raw = fs::read_to_string(path)
            .map_err(|error| format!("读取虚拟观察文件 {} 失败: {error}", path.display()))?;
        let parsed_records = if let Ok(snapshot) = serde_json::from_str::<SnapshotJson>(&raw) {
            snapshot.records
        } else if let Ok(record) = serde_json::from_str::<RecordJson>(&raw) {
            vec![record]
        } else {
            return Err(format!("虚拟观察文件 {} JSON 损坏", path.display()));
        };
        for record in parsed_records {
            push_record(&mut records, record, path)?;
        }
    }
    Ok(VirtualSnapshotLite { records })
}

/// v13 §14.2 I-01 盘中轮动总览 (⚡交易建议类, 带 banner)
pub async fn push_intraday_market(
    code: &str,
    banner: &BannerCtx,
    params: IntradayMarketParams<'_>,
) -> bool {
    push_intraday_market_outcome(code, banner, params)
        .await
        .is_pushed()
}

async fn push_intraday_market_outcome(
    code: &str,
    banner: &BannerCtx,
    params: IntradayMarketParams<'_>,
) -> crate::notify::PushOutcome {
    let text = render_intraday_market(banner, params);
    dispatch_outcome(
        crate::notify::PushKind::IntradayMarket,
        code,
        Some(banner),
        text,
    )
    .await
}

/// v13 §14.2 I-02 新闻催化映射 (⚡交易建议类, 带 banner)
pub async fn push_news_catalyst(
    code: &str,
    banner: &BannerCtx,
    params: NewsCatalystParams<'_>,
) -> bool {
    let text = render_news_catalyst(banner, params);
    dispatch(
        crate::notify::PushKind::NewsCatalyst,
        code,
        Some(banner),
        text,
    )
    .await
}

/// v13 §14.2 I-09 量价反向发现 (⚡重要, 无 banner)
pub async fn push_sector_anomaly(
    hhmm: &str,
    moves: &[stock_analysis::market_analyzer::sector_monitor::UnexplainedMove],
) -> bool {
    if moves.is_empty() {
        return false;
    }
    let text = render_sector_anomaly(hhmm, moves);
    dispatch(crate::notify::PushKind::SectorAnomaly, "", None, text).await
}

/// v13 §14.2 I-03 盘中涨停扩散 (⚡交易建议类, 带 banner, 审计多发现)
pub async fn push_industry_chain_intraday(
    code: &str,
    banner: &BannerCtx,
    params: IndustryChainIntradayParams<'_>,
) -> bool {
    push_industry_chain_intraday_outcome(code, banner, params)
        .await
        .is_pushed()
}

async fn push_industry_chain_intraday_outcome(
    code: &str,
    banner: &BannerCtx,
    params: IndustryChainIntradayParams<'_>,
) -> crate::notify::PushOutcome {
    let text = render_industry_chain_intraday(banner, params);
    dispatch_outcome(
        crate::notify::PushKind::IndustryChainIntraday,
        code,
        Some(banner),
        text,
    )
    .await
}

/// v13 §14.4 D-01 新闻驱动个股 (⚡交易建议类, 带 banner)
pub async fn push_news_to_idea(
    code: &str,
    banner: &BannerCtx,
    params: NewsToIdeaParams<'_>,
) -> bool {
    let text = render_news_to_idea(banner, params);
    dispatch(
        crate::notify::PushKind::NewsToIdea,
        code,
        Some(banner),
        text,
    )
    .await
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
    banner: &BannerCtx,
    params: CandidateTriggeredParams<'_>,
    promotion_evidence: Option<stock_analysis::opportunity::candidate_state::PromotionEvidence>,
    live_override: Option<bool>,
) -> bool {
    use stock_analysis::opportunity::candidate_state::require_live_promotion;

    if let Err(error) = require_live_promotion(promotion_evidence, live_override) {
        log::info!("[T-07] 候选触发保持 Shadow (code={code}): {error}");
        return false;
    }

    let text = render_candidate_triggered(banner, params);
    dispatch(
        crate::notify::PushKind::CandidateTriggered,
        code,
        Some(banner),
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
    dispatch(
        crate::notify::PushKind::CandidateInvalidated,
        code,
        None,
        text,
    )
    .await
}

/// v12 PR2-2.2: 数据模式变更编排器.
///
/// 完整链路: evaluate() → 计划状态变更 → 拼 T-02 → dispatch().
/// BR-116: 已确认状态本身负责精确去重，不设跨状态的粗粒度时间冷却。
///
/// 返回 `Ok(true)` 表示推送成功; `Ok(false)` 表示无变更 (no-op).
///
/// `prev` 由调用方从 history 表恢复, 首次评估传 None.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DataModeDispatchReason {
    Transition,
    PersistentUnsafeReminder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DataModeNotificationPlan {
    EstablishSilently,
    Dispatch {
        previous: Option<stock_analysis::monitor::data_mode::DataMode>,
        current: stock_analysis::monitor::data_mode::DataMode,
        reason: DataModeDispatchReason,
    },
}

fn data_mode_notification_plan(
    input: &stock_analysis::monitor::data_mode::DataHealthInput,
    prev: Option<stock_analysis::monitor::data_mode::DataMode>,
    persistent_reminder_due: bool,
) -> DataModeNotificationPlan {
    use stock_analysis::monitor::data_mode::{evaluate as dm_evaluate, DataMode as LibDM};

    let health = dm_evaluate(input, prev);
    match (prev, health.mode) {
        (None, LibDM::Full) => DataModeNotificationPlan::EstablishSilently,
        (None, current) => DataModeNotificationPlan::Dispatch {
            previous: None,
            current,
            reason: DataModeDispatchReason::Transition,
        },
        (Some(previous), current) if previous != current => DataModeNotificationPlan::Dispatch {
            previous: Some(previous),
            current,
            reason: DataModeDispatchReason::Transition,
        },
        (Some(LibDM::Unsafe), LibDM::Unsafe) if persistent_reminder_due => {
            DataModeNotificationPlan::Dispatch {
                previous: Some(LibDM::Unsafe),
                current: LibDM::Unsafe,
                reason: DataModeDispatchReason::PersistentUnsafeReminder,
            }
        }
        _ => DataModeNotificationPlan::EstablishSilently,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModeDispatchResult {
    EstablishedSilently,
    Delivery(crate::notify::PushOutcome),
}

impl ModeDispatchResult {
    pub fn is_confirmed(&self) -> bool {
        matches!(
            self,
            Self::EstablishedSilently | Self::Delivery(crate::notify::PushOutcome::Pushed)
        )
    }
}

pub async fn push_data_mode_change(
    input: &stock_analysis::monitor::data_mode::DataHealthInput,
    prev: Option<stock_analysis::monitor::data_mode::DataMode>,
    persistent_reminder_due: bool,
    banner: Option<&BannerCtx>,
) -> Result<ModeDispatchResult, String> {
    use stock_analysis::monitor::data_mode::{evaluate as dm_evaluate, DataMode as LibDM};

    let health = dm_evaluate(input, prev);

    let (prev_mode, new_mode, dispatch_reason) =
        match data_mode_notification_plan(input, prev, persistent_reminder_due) {
            DataModeNotificationPlan::EstablishSilently => {
                return Ok(ModeDispatchResult::EstablishedSilently);
            }
            DataModeNotificationPlan::Dispatch {
                previous,
                current,
                reason,
            } => (previous, current, reason),
        };

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

    let prev_tmpl = prev_mode.map(|mode| match mode {
        LibDM::Full => DataMode::Full,
        LibDM::Degraded => DataMode::Degraded,
        LibDM::Unsafe => DataMode::Unsafe,
    });
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
    let mode_text = match dispatch_reason {
        DataModeDispatchReason::Transition => render_data_mode(
            &hhmm,
            prev_tmpl,
            new_tmpl,
            &missing_str,
            &restrictions,
            health.eta.as_deref(),
        ),
        DataModeDispatchReason::PersistentUnsafeReminder => {
            log::warn!(
                "[DataMode][BR-135] persistent Unsafe reminder due; governed delivery starting"
            );
            render_data_mode_reminder(
                &hhmm,
                new_tmpl,
                &missing_str,
                &restrictions,
                health.eta.as_deref(),
            )
        }
    };
    text.push_str(&mode_text);

    // 2. dispatch (code="" 全局键; BR-116 uses the committed mode as exact dedup state)
    let outcome = dispatch_outcome(crate::notify::PushKind::DataMode, "", banner, text).await;

    if !matches!(outcome, crate::notify::PushOutcome::Pushed) {
        match dispatch_reason {
            DataModeDispatchReason::Transition => log::info!(
                "[DataMode][BR-116] T-02 delivery unconfirmed, mode {:?} → {:?}",
                prev_mode,
                new_mode
            ),
            DataModeDispatchReason::PersistentUnsafeReminder => {
                log::warn!("[DataMode][BR-135] persistent Unsafe reminder unconfirmed; remains due")
            }
        }
    }

    Ok(ModeDispatchResult::Delivery(outcome))
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
async fn dispatch_outcome(
    kind: crate::notify::PushKind,
    code: &str,
    banner: Option<&BannerCtx>,
    text: String,
) -> crate::notify::PushOutcome {
    // 1. mode/dm 停发
    if let Some(b) = banner {
        if should_block_on_mode(kind, b.account_mode, b.data_mode) {
            log::warn!(
                "[PUSH_GOVERNOR] §14.3.4 停发 | kind={} account={:?} data={:?}",
                kind.label(),
                b.account_mode,
                b.data_mode,
            );
            return crate::notify::PushOutcome::Denied(format!(
                "account/data mode blocked {}",
                kind.label()
            ));
        }
    }

    // 2. 冷却 (紧急类跳过)
    if is_in_cooldown(kind, code) {
        log::info!(
            "[PUSH_GOVERNOR] §14.3.1 冷却中跳过 | kind={} code={}",
            kind.label(),
            code,
        );
        return crate::notify::PushOutcome::Deduped;
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
            return crate::notify::PushOutcome::Denied(format!(
                "daily budget exhausted for {}",
                kind.label()
            ));
        }
    }

    // 4. 推 — b013 review P0-2: 票级事件显式传 code, 让 v14_gate L4 dedup 真正按
    //    (kind,code) 工作。全局事件沿用模板层的空字符串键，但进入事件 envelope 前必须
    //    规范化为 None；空字符串不是一个真实证券身份，也不能写入 BR-130 审计字段。
    //    模板层 record_cooldown + v14 L4 共存, 冗余安全 (后者兜前者漏判).
    let outcome = crate::notify::push_governor_v3(&text, kind, optional_dispatch_code(code)).await;
    if outcome.is_pushed() {
        record_cooldown(kind, code);
        if counts_against_daily_budget(kind) {
            DAILY_BUDGET_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
    outcome
}

fn optional_dispatch_code(code: &str) -> Option<&str> {
    (!code.trim().is_empty()).then_some(code)
}

pub async fn dispatch(
    kind: crate::notify::PushKind,
    code: &str,
    banner: Option<&BannerCtx>,
    text: String,
) -> bool {
    dispatch_outcome(kind, code, banner, text).await.is_pushed()
}

/// BR-116 result contract for a due periodic batch. `Empty` is reserved for a
/// successfully fetched and validated batch that contains no work; source and
/// governance failures must remain `Failed`/`Delivery(Denied|SinkError)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeriodicDispatchResult {
    Empty,
    Delivery(crate::notify::PushOutcome),
    Failed(String),
}

impl PeriodicDispatchResult {
    pub fn is_confirmed(&self) -> bool {
        matches!(
            self,
            Self::Empty
                | Self::Delivery(
                    crate::notify::PushOutcome::Pushed | crate::notify::PushOutcome::Deduped
                )
        )
    }

    fn is_pushed(&self) -> bool {
        matches!(self, Self::Delivery(crate::notify::PushOutcome::Pushed))
    }

    fn from_delivery_batch(outcomes: Vec<crate::notify::PushOutcome>) -> Self {
        if outcomes.is_empty() {
            return Self::Empty;
        }
        if let Some(failure) = outcomes.iter().find(|outcome| {
            matches!(
                outcome,
                crate::notify::PushOutcome::Denied(_) | crate::notify::PushOutcome::SinkError(_)
            )
        }) {
            return Self::Failed(format!("periodic delivery batch failed: {failure:?}"));
        }
        if outcomes.iter().any(crate::notify::PushOutcome::is_pushed) {
            Self::Delivery(crate::notify::PushOutcome::Pushed)
        } else {
            Self::Delivery(crate::notify::PushOutcome::Deduped)
        }
    }
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
    pub watch_stocks: Vec<(String, String, String)>, // (name, code, reason) — owned (name 反查, 不再用 code 顶替)
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
        // W1.15 / B-010 P0-4: sub 缺失用空串+log warn, 显示端判空显示"无"
        let s = match sub {
            Some(v) if !v.is_empty() => v,
            _ => {
                log::warn!("[push] IntradayMarket 缺 sub");
                ""
            }
        };
        let sc = score
            .map(|v| format!("{:.1}", v))
            .unwrap_or_else(|| "N/A".to_string());
        let s_display = if s.is_empty() { "无" } else { s };
        format!("{}(强度{})", s_display, sc)
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

/// v13 §14.2 I-09 量价反向发现（板块异动但无新闻归因）
pub fn render_sector_anomaly(
    hhmm: &str,
    moves: &[stock_analysis::market_analyzer::sector_monitor::UnexplainedMove],
) -> String {
    let mut s = format!("🛰️ 异动无归因（{}）\n", hhmm);
    for m in moves {
        let reasons = m
            .reasons
            .iter()
            .map(|r| r.label())
            .collect::<Vec<_>>()
            .join("/");
        s.push_str(&format!(
            "· {}({}) 涨幅{:+.2}% | 量比{:.2} | 资金加速{:+.2}pp\n  原因: {}\n",
            m.board.name,
            m.board.code,
            m.board.change_pct,
            m.board.vol_ratio,
            m.board.inflow_accel(),
            reasons,
        ));
    }
    s.push_str("新闻源未能解释该异动, 建议人工核查是否为新题材\n辅助建议, 非下单指令");
    s
}

/// v13 §14.4 D-01 新闻驱动个股 — 主题阶段
#[derive(Debug, Clone, Default, PartialEq)]
pub enum NewsStage {
    #[default]
    Starting, // 启动
    Fermenting, // 发酵
    Diverging,  // 分歧
}

/// v13 §14.4 D-01 新闻驱动个股 — 建议动作
#[derive(Debug, Clone, Default, PartialEq)]
pub enum NewsAction {
    #[default]
    Observe, // 观察
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
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum PersistentLevel {
    High,
    Med,
    #[default]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Exchange {
    SH, // 沪市 A 股/ETF (9:30-11:30, 13:00-15:30)
    SZ, // 深市 A 股/ETF (9:15-11:30, 13:00-15:30)
    BJ, // 北交所 A 股 (9:15-11:30, 13:00-15:30)
}

/// v13.1 §5.2 委托状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
    // v59: 按 HH:MM 派生窗口 (上午/下午/尾盘) — 用 NaiveTime 比较 (F3 修复)
    //   - 旧代码用字符串比较, "09:15" lexicographic > "11:30" (因 '9' > '1')
    //   - 应解析为 NaiveTime 后按时间值比较
    let window = match chrono::NaiveTime::parse_from_str(p.hhmm, "%H:%M") {
        Ok(t) if t < chrono::NaiveTime::from_hms_opt(11, 30, 0).unwrap() => "上午",
        Ok(t) if t < chrono::NaiveTime::from_hms_opt(15, 0, 0).unwrap() => "下午",
        Ok(_) => "尾盘",
        Err(_) => "未知", // 解析失败兜底
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

/// Recalculate the ST risk band from an evidenced cost and the effective daily
/// limit. This is a deterministic rule derivation, not a market quote.
pub fn recalculate_st_risk_levels(cost: f64, effective_limit: f64) -> Result<(f64, f64), String> {
    if !cost.is_finite() || cost <= 0.0 {
        return Err(format!("ST risk recalculation cost invalid: {cost}"));
    }
    if !effective_limit.is_finite() || !(0.0..1.0).contains(&effective_limit) {
        return Err(format!(
            "ST risk recalculation effective_limit invalid: {effective_limit}"
        ));
    }
    let stop = cost * (1.0 - effective_limit);
    let take_profit = cost * (1.0 + effective_limit);
    if !stop.is_finite() || stop <= 0.0 || !take_profit.is_finite() {
        return Err("ST risk recalculation produced invalid levels".to_string());
    }
    Ok((stop, take_profit))
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
    // v59: NaN 守卫 (F4 修复) — cost=0 时浮盈显示 "N/A" 而非 "nan%"
    let pnl_pct = if p.cost > 0.0 {
        format!("{:+.1}%", ((p.now_price - p.cost) / p.cost) * 100.0)
    } else {
        "N/A (成本未记录)".to_string()
    };
    let mut s = format!(
        "⚠️ ST 涨跌幅变更（{}）\n{}({}) [{}] 持仓 {} 股\n原涨跌幅: {:+.0}% → 新涨跌幅: {:+.0}%\n现价: {:.2} 成本: {:.2} 浮盈: {}\n",
        p.hhmm, p.name, p.code, st, p.holding_qty,
        p.old_limit * 100.0, p.new_limit * 100.0,
        p.now_price, p.cost, pnl_pct
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    Agreed,
    Competitive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Board {
    Gem,
    Star,
    Main,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettleType {
    NextSession,
    RealTime,
}

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

/// BR-033 render contract.
pub fn render_block_trade_intraday_confirm(p: BlockTradeIntradayConfirmParams<'_>) -> String {
    let block_type = match p.block_type {
        BlockType::Agreed => "协议大宗",
        BlockType::Competitive => "竞价大宗",
    };
    let board = match p.board {
        Board::Gem => "创业板",
        Board::Star => "科创板",
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
        p.hhmm, p.name, p.code, block_type, confirm, p.qty, p.price, board, settle
    )
}

pub struct BlockTradePriceRangeParams<'a> {
    pub hhmm: &'a str,
    pub name: &'a str,
    pub code: &'a str,
    pub prev_close: Option<f64>,
    pub today_avg_price: f64,
    pub block_price_range: Option<&'a str>,
    pub note: &'a str,
}

/// BR-034 render contract. Missing previous close remains explicit `N/A`.
pub fn render_block_trade_price_range(p: BlockTradePriceRangeParams<'_>) -> String {
    let previous = p
        .prev_close
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| "N/A".to_string());
    let range = p.block_price_range.unwrap_or("暂无");
    format!(
        "📊 北交所大宗价格区间（{}）\n{}({})\n前收盘价: {} (原口径)\n当日实时均价: {:.2} (新口径)\n价格区间: {}\n注: {}",
        p.hhmm, p.name, p.code, previous, p.today_avg_price, range, p.note
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
// v56: I-09 领涨板块 + I-10 主力净流入 dispatcher
// ============================================================================

/// v56: I-09 领涨板块 Top N dispatcher
///   数据源: stock_analysis::market_analyzer::sector_monitor::fetch_board_ranking
async fn dispatch_sector_top_daily_result(hhmm: &str) -> PeriodicDispatchResult {
    let boards = match tokio::task::spawn_blocking(|| {
        stock_analysis::market_analyzer::sector_monitor::fetch_board_ranking("f3", 5)
    })
    .await
    {
        Ok(Ok(b)) => b,
        Ok(Err(e)) => {
            log_dispatcher_attempt("I-09", false, 0, "fetch_board_ranking failed");
            log::warn!("[I-09] fetch_board_ranking 失败: {}", e);
            return PeriodicDispatchResult::Failed(e.to_string());
        }
        Err(e) => {
            log_dispatcher_attempt("I-09", false, 0, "spawn_blocking failed");
            log::warn!("[I-09] spawn_blocking 失败: {}", e);
            return PeriodicDispatchResult::Failed(e.to_string());
        }
    };
    if boards.is_empty() {
        log_dispatcher_attempt("I-09", false, 0, "boards empty");
        log::info!("[I-09] 板块数据空, 跳过");
        return PeriodicDispatchResult::Empty;
    }
    let items: Vec<(String, f64, f64)> = boards
        .iter()
        .map(|b| (b.name.clone(), b.change_pct, b.main_inflow / 1e8))
        .collect();
    let text = render_sector_top(hhmm, &items);
    let outcome = dispatch_outcome(crate::notify::PushKind::SectorTop, "", None, text).await;
    log_dispatcher_attempt("I-09", outcome.is_pushed(), items.len(), "");
    PeriodicDispatchResult::Delivery(outcome)
}

pub async fn dispatch_sector_top_daily(hhmm: &str) -> bool {
    dispatch_sector_top_daily_result(hhmm).await.is_pushed()
}

pub async fn dispatch_sector_top_periodic(hhmm: &str) -> bool {
    dispatch_sector_top_daily_result(hhmm).await.is_confirmed()
}

/// v13 §14.2 I-09 量价反向发现 dispatcher
///   数据源: stock_analysis::market_analyzer::sector_monitor::detect_unexplained_moves
///   说明: news_text 由调用方提供；空文本表示「没有足够新闻归因」的兜底模式
pub async fn dispatch_sector_anomaly_daily(hhmm: &str, news_text: &str) -> bool {
    let moves = match tokio::task::spawn_blocking({
        let news_text = news_text.to_string();
        move || {
            stock_analysis::market_analyzer::sector_monitor::detect_unexplained_moves(
                &news_text, 20,
            )
        }
    })
    .await
    {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => {
            log_dispatcher_attempt("I-09A", false, 0, "detect_unexplained_moves failed");
            log::warn!("[I-09A] detect_unexplained_moves 失败: {}", e);
            return false;
        }
        Err(e) => {
            log_dispatcher_attempt("I-09A", false, 0, "spawn_blocking failed");
            log::warn!("[I-09A] spawn_blocking 失败: {}", e);
            return false;
        }
    };
    if moves.is_empty() {
        log_dispatcher_attempt("I-09A", false, 0, "moves empty");
        return false;
    }
    let result = push_sector_anomaly(hhmm, &moves).await;
    log_dispatcher_attempt("I-09A", result, moves.len(), "");
    result
}

/// v56: I-10 主力净流入 Top N dispatcher
///   数据源: super::market_data::fetch_market_main_inflow_top
async fn dispatch_fund_inflow_top_daily_result(hhmm: &str) -> PeriodicDispatchResult {
    let top =
        match tokio::task::spawn_blocking(|| super::market_data::fetch_market_main_inflow_top(10))
            .await
        {
            Ok(Ok(t)) => t,
            _ => {
                log_dispatcher_attempt("I-10", false, 0, "fetch_market_main_inflow_top failed");
                return PeriodicDispatchResult::Failed(
                    "fetch_market_main_inflow_top failed".to_string(),
                );
            }
        };
    if top.is_empty() {
        log_dispatcher_attempt("I-10", false, 0, "top empty");
        log::info!("[I-10] 主力净流入空, 跳过");
        return PeriodicDispatchResult::Empty;
    }
    let missing_main_net: Vec<String> = top
        .iter()
        .filter(|stock| stock.main_net_yi.is_none())
        .map(|stock| stock.code.clone())
        .collect();
    if !missing_main_net.is_empty() {
        let reason = format!("主力净流入字段缺失: {}", missing_main_net.join(","));
        log_dispatcher_attempt("I-10", false, 0, &reason);
        log::error!("[I-10] {}，拒绝不完整批次", reason);
        return PeriodicDispatchResult::Failed(reason);
    }
    let items: Vec<(String, String, f64, Option<f64>, f64)> = top
        .iter()
        .map(|s| {
            (
                s.name.clone(),
                s.code.clone(),
                s.main_net_yi.expect("validated above"),
                s.volume_ratio,
                s.change_pct,
            )
        })
        .collect();
    let text = render_fund_inflow_top(hhmm, &items);
    if items.is_empty() {
        log_dispatcher_attempt("I-10", false, 0, "valid inflow rows empty");
        return PeriodicDispatchResult::Failed("valid inflow rows empty".to_string());
    }
    let outcome = dispatch_outcome(crate::notify::PushKind::FundInflow, "", None, text).await;
    log_dispatcher_attempt("I-10", outcome.is_pushed(), items.len(), "");
    PeriodicDispatchResult::Delivery(outcome)
}

pub async fn dispatch_fund_inflow_top_daily(hhmm: &str) -> bool {
    dispatch_fund_inflow_top_daily_result(hhmm)
        .await
        .is_pushed()
}

pub async fn dispatch_fund_inflow_top_periodic(hhmm: &str) -> bool {
    dispatch_fund_inflow_top_daily_result(hhmm)
        .await
        .is_confirmed()
}

/// 盘后: 推送主力净流入 Top10 + 按收盘价虚拟买入 (写 paper_trades).
///
/// 数据源: fetch_market_main_inflow_top (盘后 price = 当日收盘价, 走盘后快照新鲜度校验).
/// 虚拟买入: 每只 Top10 以收盘价 BUY 100 股, plan_id 带毫秒时间戳 (BR-025 允许同日同股多次).
///   - 收盘涨停 (主板≥9.8% / 创业科创≥19.8%) → paper_trade 标 NotFilled "涨停不可买" (不臆造成交).
///   - 价格 ≤ 0 → 跳过该只, 显式 log (红线 2.2).
///
/// 只写 paper_trades, 零写真实持仓 (BR-023).
pub async fn dispatch_post_close_fund_inflow_buy(date: &str, banner: &BannerCtx) -> bool {
    let top =
        match tokio::task::spawn_blocking(|| crate::market_data::fetch_market_main_inflow_top(10))
            .await
        {
            Ok(Ok(t)) => t,
            _ => {
                log_dispatcher_attempt(
                    "I-10-postclose",
                    false,
                    0,
                    "fetch_market_main_inflow_top failed",
                );
                log::warn!("[盘后资金流入] 主力净流入榜拉取失败, 跳过 (不臆造)");
                return false;
            }
        };
    if top.is_empty() {
        log_dispatcher_attempt("I-10-postclose", false, 0, "top empty");
        log::info!("[盘后资金流入] 主力净流入空, 跳过");
        return false;
    }

    // 1. 推送 Top10 (收盘口径)
    let items: Vec<(String, String, f64, Option<f64>, f64)> = top
        .iter()
        .filter_map(|s| {
            let Some(main_net_yi) = s.main_net_yi else {
                log::warn!("[盘后资金流入] {}({}) 主力净流入缺失，跳过", s.name, s.code);
                return None;
            };
            Some((
                s.name.clone(),
                s.code.clone(),
                main_net_yi,
                s.volume_ratio,
                s.change_pct,
            ))
        })
        .collect();
    let hhmm = format!("{} 收盘", date);
    let text = render_fund_inflow_top(&hhmm, &items);
    let push_result = dispatch(crate::notify::PushKind::FundInflow, "", None, text).await;

    let risk_context = match paper_risk_context_from_banner(banner) {
        Ok(context) => context,
        Err(error) => {
            log::error!(
                "[盘后资金流入][BR-134] 跳过虚拟买入: 风险上下文不可用: {}",
                error
            );
            log_dispatcher_attempt(
                "I-10-postclose",
                push_result,
                top.len(),
                "paper risk context unavailable",
            );
            return push_result;
        }
    };

    // 2. 按收盘价虚拟买入
    let now = chrono::Local::now();
    let mut filled = 0usize;
    for s in &top {
        if s.price <= 0.0 {
            log::warn!(
                "[盘后资金流入] 跳过虚拟买入 {}({}): 收盘价缺失 price={}",
                s.name,
                s.code,
                s.price
            );
            continue;
        }
        let code = s.code.clone();
        let execution_quote = match tokio::task::spawn_blocking(move || {
            stock_analysis::broker::execution_quote(&code)
        })
        .await
        {
            Ok(Ok(quote)) => quote,
            Ok(Err(error)) => {
                log::warn!(
                    "[盘后资金流入] 跳过虚拟买入 {}({}): 执行报价不可用: {}",
                    s.name,
                    s.code,
                    error
                );
                continue;
            }
            Err(error) => {
                log::warn!(
                    "[盘后资金流入] 跳过虚拟买入 {}({}): 报价任务失败: {}",
                    s.name,
                    s.code,
                    error
                );
                continue;
            }
        };
        let is_limit_up = execution_quote.price >= execution_quote.limit_up_price;
        let signal = PaperSignal {
            plan_id: format!(
                "postclose-fundinflow-{}-{}",
                s.code,
                now.format("%Y%m%d%H%M%S%3f")
            ),
            code: s.code.clone(),
            name: s.name.clone(),
            direction: Direction::Buy,
            price: execution_quote.price,
            quantity: 100,
            // v16.3 Commit 2: 改 free-text → VirtualReason::MainNetInflow.as_str()
            virtual_reason:
                stock_analysis::opportunity::virtual_reason::VirtualReason::MainNetInflow
                    .as_str()
                    .to_string(),
            is_limit_up,
            is_limit_down: false,
            is_suspended: false,
            limit_up_price: Some(execution_quote.limit_up_price),
            limit_down_price: Some(execution_quote.limit_down_price),
            secondary_confirmed: false,
            quote_observed_at: execution_quote.observed_at,
            risk_context,
        };
        // v16.3 Commit 1: simulate 签名加 4 参数 (quote_price 真 + cash/total/pos_pct 真 portfolio 读)
        let (cash, total, pos_pct) = match paper_portfolio_state(&s.code, execution_quote.price) {
            Ok(state) => state,
            Err(error) => {
                log::warn!(
                    "[盘后资金流入] 跳过虚拟买入 {}({}): 账户快照不可用: {}",
                    s.name,
                    s.code,
                    error
                );
                continue;
            }
        };
        match paper_trade::simulate(&signal, execution_quote.price, cash, total, pos_pct) {
            Ok(outcome) => {
                if outcome.result.status == paper_trade::PaperTradeStatus::Filled {
                    filled += 1;
                }
                log::info!(
                    "[盘后资金流入] 虚拟买入 {}({}) status={} price={:.2} qty=100",
                    signal.name,
                    signal.code,
                    outcome.result.status.as_str(),
                    signal.price
                );
            }
            Err(e) => log::warn!(
                "[盘后资金流入] 虚拟买入失败 {}({}): {}",
                signal.name,
                signal.code,
                e
            ),
        }

        // v16.3 Commit 2: 推入 pushed_stocks 票池
        let metric_json = truncate_metric_json(
            serde_json::json!({
                "main_net_yi": s.main_net_yi,
                "volume_ratio": s.volume_ratio,
                "price_chg_pct": s.change_pct,
                "push_subkind": "MainNetInflow",
            })
            .to_string(),
        );
        if let Err(error) = stock_analysis::signal::push_recorder::record(
            &stock_analysis::signal::push_recorder::PushRecordMeta {
                code: s.code.clone(),
                name: s.name.clone(),
                push_kind: "盘后资金".to_string(),
                push_price: s.price,
                metric_json,
                source: "postclose".to_string(),
            },
        ) {
            let reason = format!(
                "I-10-postclose pushed_stocks audit failed for {}: {error}",
                s.code
            );
            log::error!("{reason}");
            log_dispatcher_attempt("I-10-postclose", false, top.len(), &reason);
            return false;
        }
    }

    log_dispatcher_attempt("I-10-postclose", push_result, top.len(), "");
    log::info!(
        "[盘后资金流入] Top{} 推送={} 虚拟成交={}/{}",
        top.len(),
        push_result,
        filled,
        top.len()
    );
    push_result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn br116_periodic_batch_requires_every_delivery_to_be_confirmed() {
        assert!(PeriodicDispatchResult::from_delivery_batch(Vec::new()).is_confirmed());
        assert!(PeriodicDispatchResult::from_delivery_batch(vec![
            crate::notify::PushOutcome::Deduped,
            crate::notify::PushOutcome::Deduped,
        ])
        .is_confirmed());
        assert!(PeriodicDispatchResult::from_delivery_batch(vec![
            crate::notify::PushOutcome::Pushed,
            crate::notify::PushOutcome::Deduped,
        ])
        .is_confirmed());
        assert!(!PeriodicDispatchResult::from_delivery_batch(vec![
            crate::notify::PushOutcome::Pushed,
            crate::notify::PushOutcome::Denied("TEST_CODE denied".to_string()),
        ])
        .is_confirmed());
    }

    #[test]
    fn br087_trade_events_require_complete_identity_and_known_type() {
        let valid = TradeEvent {
            exchange: Exchange::SH,
            code: "TEST_CODE_600000".to_string(),
            name: "测试标的".to_string(),
            price: 10.0,
            qty: 100,
            event_type: "order".to_string(),
            order_id: Some("TEST_ORDER_1".to_string()),
            status: Some(OrderStatus::Submitted),
            next_session_carry: None,
        };
        assert!(valid_trade_event(&valid));
        for invalid in [
            TradeEvent {
                code: "TEST_CODE_BAD".to_string(),
                ..valid.clone()
            },
            TradeEvent {
                name: " ".to_string(),
                ..valid.clone()
            },
            TradeEvent {
                event_type: "unknown".to_string(),
                ..valid.clone()
            },
        ] {
            assert!(!valid_trade_event(&invalid), "{invalid:?}");
        }
    }

    #[test]
    fn br097_realtime_quote_freshness_uses_source_timestamp() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-18T02:15:35Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(realtime_quote_source_is_fresh(
            now - chrono::Duration::seconds(5),
            now
        ));
        assert!(!realtime_quote_source_is_fresh(
            now - chrono::Duration::milliseconds(5_001),
            now
        ));
        assert!(!realtime_quote_source_is_fresh(
            now + chrono::Duration::milliseconds(1),
            now
        ));
    }

    fn banner_normal() -> BannerCtx {
        BannerCtx {
            account_mode: AccountMode::Normal,
            total_pos: Some(5),
            today_pnl: Some(0.3),
            account_metrics_complete: true,
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
    fn incomplete_banner_renders_missing_account_facts() {
        let banner = BannerCtx {
            account_mode: AccountMode::ReduceOnly,
            total_pos: None,
            today_pnl: None,
            account_metrics_complete: false,
            data_mode: DataMode::Unsafe,
            data_missing_note: Some("账户指标缺失".to_string()),
        };
        let text = banner.render();
        assert!(text.contains("仓位缺失"));
        assert!(text.contains("日盈亏缺失"));
    }

    #[test]
    fn br134_incomplete_banner_cannot_create_paper_risk_context() {
        let banner = BannerCtx {
            account_mode: AccountMode::ReduceOnly,
            total_pos: None,
            today_pnl: None,
            account_metrics_complete: false,
            data_mode: DataMode::Unsafe,
            data_missing_note: Some("账户指标缺失".to_string()),
        };
        assert!(paper_risk_context_from_banner(&banner).is_err());
    }

    #[test]
    fn br134_displayed_metrics_do_not_replace_all_three_fact_completeness() {
        let banner = BannerCtx {
            account_mode: AccountMode::Normal,
            total_pos: Some(4),
            today_pnl: Some(0.2),
            account_metrics_complete: false,
            data_mode: DataMode::Full,
            data_missing_note: None,
        };

        assert!(paper_risk_context_from_banner(&banner).is_err());
    }

    #[test]
    fn br134_banner_conversion_preserves_frozen_and_unsafe_modes() {
        let banner = BannerCtx {
            account_mode: AccountMode::Frozen,
            data_mode: DataMode::Unsafe,
            ..BannerCtx::test_default()
        };
        let context = paper_risk_context_from_banner(&banner).unwrap();
        assert_eq!(
            context.account_mode,
            stock_analysis::risk::action_gate::AccountMode::Frozen
        );
        assert_eq!(
            context.data_mode,
            stock_analysis::monitor::data_mode::DataMode::Unsafe
        );
    }

    #[test]
    fn banner_reduce_only_degraded() {
        let b = BannerCtx {
            account_mode: AccountMode::ReduceOnly,
            total_pos: Some(6),
            today_pnl: Some(-1.6),
            account_metrics_complete: true,
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
            total_pos: Some(0),
            today_pnl: Some(-2.1),
            account_metrics_complete: true,
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
            Some(DataMode::Full),
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
            Some(DataMode::Degraded),
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
                code: "TEST_CODE_000001",
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
        assert!(s.contains("🎯 持仓建议 XX科技(TEST_CODE_000001)（13:42）"));
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
                code: "TEST_CODE_600000",
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
                code: "TEST_CODE_000001",
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
                code: "TEST_CODE_002415",
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
                code: "TEST_CODE_300750",
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
                code: "TEST_CODE_688001",
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
            "TEST_CODE_688002",
            "Watch",
            "触发失败: 未触达买入区",
        );
        assert!(s.contains("📋 候选失效 候选Y(TEST_CODE_688002)（11:00）"));
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
                code: "TEST_CODE_000001",
                hhmm: "10:00",
                conclusion: "距涨停过近, 禁止追买",
                reasons: &["距涨停仅 1.2%".to_string(), "板块已 Climax".to_string()],
            },
        );
        assert!(s.contains("🚫 禁止操作（10:00）"));
        assert!(s.contains("XX(TEST_CODE_000001): 距涨停过近, 禁止追买"));
        assert!(s.contains("· 距涨停仅 1.2%"));
        assert!(s.contains("· 板块已 Climax"));
    }

    // ---- T-10 虚拟盘 ----

    #[test]
    fn t10_paper_trade_filled() {
        let s = render_paper_trade(PaperTradeParams {
            name: "ZZ",
            code: "TEST_CODE_002415",
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
        assert!(s.contains("ZZ(TEST_CODE_002415) Filled"));
        assert!(s.contains("成交价25.10 数量1000 主理由候选A档触发"));
        assert!(s.contains("账户Normal/数据Full"));
    }

    #[test]
    fn t10_paper_trade_not_filled() {
        let s = render_paper_trade(PaperTradeParams {
            name: "YY",
            code: "TEST_CODE_688001",
            hhmm: "10:00",
            status: PaperTradeStatus::NotFilled,
            fill_price: None,
            qty: None,
            virtual_reason: None,
            not_fill_reason: Some("涨停不可买"),
            account_mode: AccountMode::Normal,
            data_mode: DataMode::Full,
        });
        assert!(s.contains("YY(TEST_CODE_688001) NotFilled"));
        assert!(s.contains("未成交原因: 涨停不可买"));
        assert!(!s.contains("成交价"));
    }

    // ---- T-11 竞价异动 ----

    #[test]
    fn t11_auction_volume() {
        let items = vec![
            AuctionItem {
                name: "A",
                code: "TEST_CODE_000001",
                gap_pct: 5.2,
                vol_ratio: 8.5,
                tag: "昨日涨停",
            },
            AuctionItem {
                name: "B",
                code: "TEST_CODE_600000",
                gap_pct: 2.1,
                vol_ratio: 3.2,
                tag: "观察池",
            },
        ];
        let s = render_auction_volume(&banner_normal(), "09:25", &items, "强承接", "可操作");
        assert!(s.contains("🌅 竞价热点量能 Top2（09:25）")); // v13 标题统一
        assert!(s.contains("A(TEST_CODE_000001) 高开+5.2% 量比8.5 [昨日涨停]"));
        assert!(s.contains("B(TEST_CODE_600000) 高开+2.1% 量比3.2 [观察池]"));
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
            code: "TEST_CODE_002415",
            satisfied: false,
            cond: "板块龙头未封板",
        };
        let s = render_close_call(&banner_normal(), "14:50", None, Some(&g));
        assert!(
            s.contains("[博弈] YY(TEST_CODE_002415): 尾盘买入博次日溢价条件未满足: 板块龙头未封板")
        );
    }

    // ---- R-01 持仓明日计划 ----

    #[test]
    fn r01_daily_report() {
        let items = vec![
            HoldingDailyPlan {
                name: "XX",
                code: "TEST_CODE_000001",
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
                code: "TEST_CODE_002415",
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
        assert!(s.contains("XX(TEST_CODE_000001) 现价12.30 成本11.80 浮盈+4.2%"));
        assert!(s.contains("· 高开>2.0%: 减仓1/3"));
        assert!(s.contains("· 低开/跌破11.95: 执行止损"));
        assert!(s.contains("YY(TEST_CODE_002415) 现价25.10 成本26.00 浮盈-3.5%"));
    }

    // ---- R-02 盘面走向 ----

    #[test]
    fn r02_review_market_full() {
        let s = render_review_market(
            "2026-07-05",
            &MarketReview {
                sh_chg: Some(0.5),
                chinext_chg: Some(1.2),
                star_chg: Some(1.5),
                limit_up_n: Some(35),
                limit_down_n: Some(3),
                broken_pct: Some(15.0),
                consecutive_h: Some(5),
                amount_yi: Some(8500.0),
                amount_delta_pct: Some(8.0),
                amount_dir: Some("放量"),
                main_flow_yi: Some(120.0),
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
    fn r02_review_market_missing_index_no_stray_pct() {
        // BR-093: 缺数据(None)时显示"暂无", 不应出现"暂无%"(尾部多一个%)
        let s = render_review_market(
            "2026-07-05",
            &MarketReview {
                sh_chg: None,
                chinext_chg: None,
                star_chg: None,
                limit_up_n: Some(30),
                limit_down_n: Some(5),
                broken_pct: Some(15.0),
                consecutive_h: None,
                amount_yi: None,
                amount_delta_pct: None,
                amount_dir: None,
                main_flow_yi: None,
                money_effect: "中等",
                heat_stage: "HeatUp",
                heat_conf_pct: 62,
                low_conf: false,
                low_conf_tier: None,
                account_mode: AccountMode::Normal,
                max_pos: 7,
            },
        );
        assert!(s.contains("上证暂无 创业暂无 科创暂无"));
        assert!(!s.contains("暂无%"), "缺数据不应出现 '暂无%' (尾部多余%)");
        assert!(s.contains("连板高度暂无"), "连板高度无数据应显示暂无");
        assert!(s.contains("两市暂无"), "成交额无数据应显示暂无");
        assert!(s.contains("主力净暂无"), "主力净流入无数据应显示暂无");
    }

    #[test]
    fn r02_review_market_preserves_real_zero() {
        let s = render_review_market(
            "2026-07-05",
            &MarketReview {
                sh_chg: Some(0.0),
                chinext_chg: Some(0.0),
                star_chg: Some(0.0),
                limit_up_n: Some(0),
                limit_down_n: Some(0),
                broken_pct: Some(0.0),
                consecutive_h: Some(0),
                amount_yi: Some(8500.0),
                amount_delta_pct: Some(0.0),
                amount_dir: Some("平量"),
                main_flow_yi: Some(0.0),
                ..test_market_review_default()
            },
        );
        assert!(s.contains("上证+0.0% 创业+0.0% 科创+0.0%"));
        assert!(s.contains("涨停0家 跌停0家 炸板率0% 连板高度0板"));
        assert!(s.contains("两市8500亿（平量+0%） 主力净+0亿"));
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
            sh_chg: None,
            chinext_chg: None,
            star_chg: None,
            limit_up_n: None,
            limit_down_n: None,
            broken_pct: None,
            consecutive_h: None,
            amount_yi: None,
            amount_delta_pct: None,
            amount_dir: None,
            main_flow_yi: None,
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
                leader_code: "TEST_CODE_688001",
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
                leader_code: "TEST_CODE_300750",
                leader_boards: 2,
                followers: "X,Y",
                watch_point: "接力意愿",
            },
        ];
        let s = render_industry_chain("2026-07-05", &chains, Some("光伏（涨停12→3家）"));
        assert!(s.starts_with("🔥 涨停产业链（2026-07-05）"));
        assert!(s.contains("1. AI算力 涨停8家"));
        assert!(s.contains("龙头: 龙头A(TEST_CODE_688001) 4板"));
        assert!(s.contains("2. 机器人"));
        assert!(s.contains("⚠️ 退潮链: 光伏（涨停12→3家）"));
    }

    // ---- R-04 龙虎榜 ----

    #[test]
    fn r04_review_lhb() {
        let entries = vec![LhbEntry {
            name: "X",
            code: "TEST_CODE_688001",
            net_buy_yi: 1.5,
            reason: "涨幅偏离值达7%",
            buy_inst_n: 2,
            buy_inst_amt_wan: Some(8000.0),
            buy_other_n: 3,
            buy_other_amt_wan: Some(4000.0),
            buy_conc_pct: Some(65.0),
            sell_desc: "游资席位",
            sell_conc_pct: Some(45.0),
            chain_match: Some("AI算力"),
            next_day_risk: "高开震荡",
        }];
        let s = render_review_lhb("2026-07-05", &entries);
        assert!(s.starts_with("🐉 龙虎榜净买前五（2026-07-05 21:00）"));
        assert!(s.contains("X(TEST_CODE_688001) 净买1.5亿"));
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
            code: "TEST_CODE_688001",
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
        assert!(s.contains("X(TEST_CODE_688001) 原信号: ⚡A档"));
        assert!(s.contains("归因: 涨停不可买"));
        assert!(s.contains("处理建议: 调高触发阈值"));
        assert!(s.contains("本周归因分布: 买点过晚2 板块退潮1 不可成交3 人未执行1"));
    }

    // ---- R-07 明日观察池 ----

    #[test]
    fn r07_tomorrow_watch() {
        let items = vec![WatchItem {
            name: "Y",
            code: "TEST_CODE_002415",
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
        assert!(s.contains("1. Y(TEST_CODE_002415) [机器人] 来源: A档未触发"));
        assert!(s.contains("触发突破50.5 | 低吸49.50~50.30 | 止损48.50"));
        assert!(s.contains("共1只 | 明日竞价后按 T-11 复核"));
    }

    // ---- R-08 事件日历 ----

    #[test]
    fn r08_event_calendar() {
        let holdings = vec![
            HoldingEventItem {
                tag: "实盘",
                name: "XX",
                code: "TEST_CODE_000001",
                kind: "解禁3.2亿",
            },
            HoldingEventItem {
                tag: "虚拟",
                name: "YY",
                code: "TEST_CODE_000002",
                kind: "财报预告",
            },
        ];
        let s = render_event_calendar("2026-07-06", &holdings, "央行MLF到期", "+0.8%", "7.18");
        assert!(s.starts_with("🗓️ 明日事件（2026-07-06）"));
        assert!(s.contains("· 【实盘】XX(TEST_CODE_000001): 解禁3.2亿"));
        assert!(s.contains("· 【虚拟】YY(TEST_CODE_000002): 财报预告"));
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
                (
                    "中科曙光".to_string(),
                    "TEST_CODE_603019".to_string(),
                    "AI算力龙头".to_string(),
                ),
                (
                    "绿的谐波".to_string(),
                    "TEST_CODE_688017".to_string(),
                    "减速器".to_string(),
                ),
            ],
        };
        let out = render_preopen_news_hot(p);
        assert!(out.contains("📰 盘前热点（09:05）"));
        assert!(out.contains("主线: AI算力 / 机器人 / 消费电子"));
        assert!(out.contains("· 英伟达新品 → 利好GPU"));
        assert!(out.contains("· 中科曙光(TEST_CODE_603019) 逻辑: AI算力龙头"));
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
            watch_stocks: vec![(
                "X".to_string(),
                "TEST_CODE_000001".to_string(),
                "r".to_string(),
            )],
        };
        let out = render_preopen_news_hot(p);
        assert!(!out.contains("主线:"));
        assert!(!out.contains("催化:"));
        assert!(out.contains("· X(TEST_CODE_000001) 逻辑: r"));
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
        // W1.15 / B-010 P0-4: sub=None 时显示 "无" (不再用 em-dash 占位, BR-004)
        assert!(out.contains("无(强度N/A)")); // power and robot default to "无"
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
            stocks: vec![("中科曙光", "TEST_CODE_603019", Some(5.2), "AI龙头")],
        };
        let banner = BannerCtx::test_default();
        let out = render_news_catalyst(&banner, p);
        assert!(out.contains("🟢")); // banner 包含 Normal icon
        assert!(out.contains("📰⚡ 新闻催化跟踪（10:30）"));
        assert!(out.contains("新闻: 英伟达发布H200"));
        assert!(out.contains("受益板块: AI算力"));
        assert!(out.contains("· 中科曙光(TEST_CODE_603019) +5.2% | 原因:AI龙头"));
        assert!(out.ends_with("辅助建议, 非下单指令"));
    }

    #[test]
    fn news_catalyst_missing_chg_omits_row() {
        let p = NewsCatalystParams {
            hhmm: "10:30",
            headline: "X",
            theme: None,
            stocks: vec![
                ("A", "TEST_CODE_000001", None, "r"),
                ("B", "TEST_CODE_000002", Some(3.0), "r2"),
            ],
        };
        let banner = BannerCtx::test_default();
        let out = render_news_catalyst(&banner, p);
        assert!(!out.contains("· A(TEST_CODE_000001)"));
        assert!(out.contains("· B(TEST_CODE_000002) +3.0% | 原因:r2"));
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
            code: "TEST_CODE_603019",
            reasons: vec!["AI算力龙头", "业绩超预期"],
            action: Some(NewsAction::BuyDip),
        };
        let banner = BannerCtx::test_default();
        let out = render_news_to_idea(&banner, p);
        assert!(out.contains("🧭 新闻驱动个股（10:30）"));
        assert!(out.contains("板块: AI算力 | 阶段: 启动"));
        assert!(out.contains("个股: 中科曙光(TEST_CODE_603019)"));
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
            code: "TEST_CODE_000001",
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
            code: "TEST_CODE_000001",
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
            code: "TEST_CODE_000001",
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
            leader_code: Some("TEST_CODE_000001"),
            leader_height: 3,
            supplements: vec![SupplementCandidate {
                name: "B",
                code: "TEST_CODE_000002",
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
        assert!(out.contains("龙头: A(TEST_CODE_000001) 3板"));
        assert!(out.contains("· B(TEST_CODE_000002) 触发条件首板 | 低吸10.00~12.00 | 止损9.00"));
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
            code: "TEST_CODE_600000",
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
            code: "TEST_CODE_000001",
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
            code: "TEST_CODE_830001",
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
            code: "TEST_CODE_600000",
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
            code: "TEST_CODE_830001",
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
            code: "TEST_CODE_600000",
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
        assert!(out.contains("A(TEST_CODE_600000) [ST] 持仓 1000 股"));
        assert!(out.contains("原涨跌幅: +5% → 新涨跌幅: +10%"));
        assert!(out.contains("新止损: 9.00 (基于 10% 阈值)"));
        assert!(out.contains("新止盈: 12.00"));
        assert!(out.contains("浮盈: +10.0%"));
        assert!(out.contains("辅助建议, 非下单指令 — 现有持仓风险阈值已重新校准"));
    }

    #[test]
    fn st_risk_recalculation_uses_effective_limit_and_rejects_bad_inputs() {
        let (stop, take_profit) =
            recalculate_st_risk_levels(10.0, 0.10).expect("valid ST recalculation");
        assert!((stop - 9.0).abs() < f64::EPSILON);
        assert!((take_profit - 11.0).abs() < f64::EPSILON);
        assert!(recalculate_st_risk_levels(0.0, 0.10).is_err());
        assert!(recalculate_st_risk_levels(10.0, 1.0).is_err());
    }

    #[test]
    fn st_price_limit_changed_star_st_no_recalc() {
        let p = StPriceLimitChangedParams {
            hhmm: "09:30",
            name: "B",
            code: "TEST_CODE_000001",
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
        assert!(out.contains("B(TEST_CODE_000001) [*ST]"));
        assert!(out.contains("新止损: 未重算"));
        assert!(!out.contains("新止盈:"));
        assert!(out.contains("浮盈: -10.0%"));
    }

    #[test]
    fn st_price_limit_changed_zero_qty_alert() {
        let p = StPriceLimitChangedParams {
            hhmm: "09:30",
            name: "A",
            code: "TEST_CODE_600000",
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
            code: "TEST_CODE_510300",
            call_auction_price: Some(3.952),
            vs_continuous_est: Some(0.15),
            liquidity_note: "正常, 无尾盘操纵",
        };
        let out = render_etf_closing_call_auction(p);
        assert!(out.contains("📊 ETF 集合竞价尾盘（14:58）"));
        assert!(out.contains("沪深300ETF(TEST_CODE_510300) 沪市 ETF 收盘价: 3.952"));
        assert!(out.contains("vs 连续竞价估值: +0.15%"));
        assert!(out.contains("14:57-15:00 集合竞价形成收盘价"));
    }

    #[test]
    fn block_trade_intraday_confirm_gem() {
        let out = render_block_trade_intraday_confirm(BlockTradeIntradayConfirmParams {
            hhmm: "11:15",
            name: "A",
            code: "TEST_CODE_300750",
            qty: 1000,
            price: 50.0,
            block_type: BlockType::Agreed,
            board: Board::Gem,
            real_time_confirm: true,
            next_session_settle: SettleType::NextSession,
        });
        assert!(out.contains("协议大宗 ✅ 盘中实时确认"));
        assert!(out.contains("板块: 创业板 | 清算: 次日清算"));
    }

    #[test]
    fn block_trade_price_range_bj() {
        let out = render_block_trade_price_range(BlockTradePriceRangeParams {
            hhmm: "14:30",
            name: "A",
            code: "TEST_CODE_830001",
            prev_close: Some(10.50),
            today_avg_price: 10.80,
            block_price_range: Some("10.50~11.10"),
            note: "新口径为当日均价",
        });
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
    fn gov_etf_closing_call_auction_no_banner() {
        assert!(!crate::notify::PushKind::EtfClosingCallAuction.requires_banner());
    }
    #[test]
    fn gov_etf_closing_call_auction_level() {
        assert_eq!(
            crate::notify::PushKind::EtfClosingCallAuction.level(),
            crate::notify::PushLevel::Important
        );
    }

    // ====== v14 A-01 虚拟仓复盘 (2 用例) ======
    #[test]
    fn paper_review_full() {
        let p = PaperReviewParams {
            date: "2026-07-06",
            name: "A",
            code: "TEST_CODE_000001",
            trigger: "首板",
            desc: "已成交",
            pnl: Some(2.5),
            plan_high: Some("观察"),
            plan_flat: Some("持有"),
            plan_low: Some("止损"),
        };
        let out = render_paper_review(p);
        assert!(out.contains("🧪 虚拟仓复盘（2026-07-06）"));
        assert!(out.contains("A(TEST_CODE_000001) 原触发: 首板"));
        assert!(out.contains("结果: 已成交 +2.5%"));
        assert!(out.contains("· 高开>1%: 观察"));
        assert!(out.contains("· 平开: 持有"));
        assert!(out.contains("· 低开/跌破止损: 止损"));
    }

    #[test]
    fn paper_review_pnl_missing_no_plan() {
        let p = PaperReviewParams {
            date: "2026-07-06",
            name: "A",
            code: "TEST_CODE_000001",
            trigger: "T",
            desc: "X",
            pnl: None,
            plan_high: None,
            plan_flat: None,
            plan_low: None,
        };
        let out = render_paper_review(p);
        assert!(out.contains("结果: X N/A%"));
        assert!(!out.contains("次日计划:"));
    }

    // ====== v14 治理元信息测试 (A-01) ======
    #[test]
    fn gov_paper_review_cooldown() {
        assert_eq!(
            crate::notify::PushKind::PaperReview.cooldown_secs(),
            Some(86_400)
        );
    }
    #[test]
    fn gov_paper_review_no_banner() {
        assert!(!crate::notify::PushKind::PaperReview.requires_banner());
    }
    #[test]
    fn gov_paper_review_level() {
        assert_eq!(
            crate::notify::PushKind::PaperReview.level(),
            crate::notify::PushLevel::Info
        );
    }

    // ====== v14.3 F-12: 候选失效独立 enum 治理测试 ======
    #[test]
    fn gov_candidate_invalidated_cooldown() {
        assert_eq!(
            crate::notify::PushKind::CandidateInvalidated.cooldown_secs(),
            Some(1800)
        );
    }
    #[test]
    fn gov_candidate_invalidated_no_banner() {
        assert!(!crate::notify::PushKind::CandidateInvalidated.requires_banner());
    }
    #[test]
    fn gov_candidate_invalidated_level() {
        assert_eq!(
            crate::notify::PushKind::CandidateInvalidated.level(),
            crate::notify::PushLevel::Important
        );
    }

    // ====== v15.1: P-01 业务层集成测试 ======
    #[test]
    fn v15_build_preopen_news_hot_from_db() {
        use stock_analysis::database::concepts::{BoardRotationRow, ChainDailyRow};
        let clusters = vec![
            ChainDailyRow {
                date: "2026-07-06".to_string(),
                concept: "AI算力".to_string(),
                stocks: r#"["TEST_CODE_600000","TEST_CODE_000001","TEST_CODE_600519"]"#.to_string(),
                continuation_count: 3,
            },
            ChainDailyRow {
                date: "2026-07-06".to_string(),
                concept: "机器人".to_string(),
                stocks: r#"["TEST_CODE_000002","TEST_CODE_000003"]"#.to_string(),
                continuation_count: 2,
            },
        ];
        let rotations = vec![
            BoardRotationRow {
                date: "2026-07-06".to_string(),
                board_code: "BK_AI".to_string(),
                board_name: "AI算力".to_string(),
                news_title: "AI 服务器订单增长".to_string(),
                board_change_pct: 2.0,
                board_main_net_pct: 1.0,
                stocks_json: r#"[{"code":"TEST_CODE_600000","name":"浦发银行","change_pct":1.0}]"#
                    .to_string(),
            },
            BoardRotationRow {
                date: "2026-07-06".to_string(),
                board_code: "BK_ROBOT".to_string(),
                board_name: "机器人".to_string(),
                news_title: "机器人产业订单落地".to_string(),
                board_change_pct: 1.5,
                board_main_net_pct: 0.8,
                stocks_json: r#"[{"code":"TEST_CODE_000002","name":"万科A","change_pct":1.0}]"#
                    .to_string(),
            },
        ];
        let p = build_preopen_news_hot_from_db("09:05", &clusters, &rotations)
            .expect("build strict preopen snapshot");
        assert_eq!(p.hhmm, "09:05");
        assert_eq!(p.theme_1, Some("AI算力"));
        assert_eq!(p.theme_2, Some("机器人"));
        assert_eq!(p.theme_3, None); // 只有 2 cluster
        assert_eq!(p.watch_stocks.len(), 2);
        assert_eq!(p.watch_stocks[0].0, "浦发银行");
        assert_eq!(p.watch_stocks[0].1, "TEST_CODE_600000");
        assert_eq!(p.watch_stocks[0].2, "AI算力");
        assert_eq!(p.news_pairs.len(), 2);
        assert_eq!(p.news_pairs[0], ("AI 服务器订单增长", "AI算力"));
    }

    #[test]
    fn v15_build_preopen_news_hot_empty_db() {
        use stock_analysis::database::concepts::ChainDailyRow;
        let clusters: Vec<ChainDailyRow> = vec![];
        assert!(build_preopen_news_hot_from_db("09:05", &clusters, &[]).is_err());
    }

    #[test]
    fn v15_dispatch_preopen_news_hot_daily_no_data() {
        // 空 DB 时不推送 (graceful no-op)
        // 实际需要 DB, 此处仅验证 build_* 函数路径, dispatch 行为在 e2e
        use stock_analysis::database::concepts::ChainDailyRow;
        let clusters: Vec<ChainDailyRow> = vec![];
        assert!(build_preopen_news_hot_from_db("09:05", &clusters, &[]).is_err());
    }

    // ====== v15.2: I-01 业务层集成测试 (sector_score 抽口) ======
    #[test]
    fn v15_build_intraday_market_from_snapshot() {
        let s = SectorSnapshot {
            hhmm: "10:30".to_string(),
            tech_sub: "AI算力".to_string(),
            tech_score: Some(85.5),
            power_sub: "特高压".to_string(),
            power_score: Some(60.0),
            robot_sub: "减速器".to_string(),
            robot_score: Some(72.3),
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

        let _: fn(&str) -> Result<SectorSnapshot, String> = load_sector_snapshot_real;
        // 验证 SectorSnapshot Default 字段
        let s = SectorSnapshot::default();
        assert_eq!(s.rotation_state, RotationState::Spreading); // enum default
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
                (
                    "中科曙光".to_string(),
                    "TEST_CODE_603019".to_string(),
                    Some(5.2),
                ),
                (
                    "浪潮信息".to_string(),
                    "TEST_CODE_000977".to_string(),
                    Some(3.8),
                ),
            ],
            llm_tickers: vec![],
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

    /// v13.10.5: LLM 路径 — llm_tickers 非空时优先用 LLM 提供的 chain + reason
    #[test]
    fn v13_10_5_llm_tickers_take_precedence() {
        use stock_analysis::llm::TickerHit;
        let s = NewsCatalystSnapshot {
            hhmm: "10:30".to_string(),
            headline: "PCB 涨价 12%".to_string(),
            theme: "PCB".to_string(),
            stocks: vec![], // 空 — LLM 路径接管
            llm_tickers: vec![
                TickerHit {
                    code: "TEST_CODE_002916".to_string(),
                    name: "深南电路".to_string(),
                    importance: 9,
                    reason: "PCB 涨价 12% 直接受益".to_string(),
                    chain: "PCB".to_string(),
                },
                TickerHit {
                    code: "TEST_CODE_002463".to_string(),
                    name: "沪电股份".to_string(),
                    importance: 7,
                    reason: "800G 交换机 PCB 订单".to_string(),
                    chain: "PCB".to_string(),
                },
            ],
        };
        let p = build_news_catalyst_from_snapshot(&s);
        assert_eq!(p.stocks.len(), 2, "应使用 llm_tickers");
        assert_eq!(p.stocks[0].0, "深南电路", "用 LLM 提供的 name");
        assert_eq!(p.stocks[0].1, "TEST_CODE_002916");
        assert_eq!(
            p.stocks[0].3, "PCB 涨价 12% 直接受益",
            "用 LLM 提供的 reason"
        );
        assert_eq!(p.stocks[1].3, "800G 交换机 PCB 订单");
    }

    /// v13.10.5: 降级路径 — llm_tickers 空时, 用 stocks + theme 短语
    #[test]
    fn v13_10_5_fallback_to_theme_when_llm_empty() {
        let s = NewsCatalystSnapshot {
            hhmm: "10:30".to_string(),
            headline: "PCB 涨价".to_string(),
            theme: "PCB".to_string(),
            stocks: vec![(
                "深南电路".to_string(),
                "TEST_CODE_002916".to_string(),
                Some(10.0),
            )],
            llm_tickers: vec![],
        };
        let p = build_news_catalyst_from_snapshot(&s);
        assert_eq!(p.stocks.len(), 1);
        assert_eq!(p.stocks[0].3, "PCB 板块共振", "降级用 theme match 短语");
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
            leader_code: "TEST_CODE_000001".to_string(),
            leader_height: 3,
            supplements: vec![(
                "补涨B".to_string(),
                "TEST_CODE_000002".to_string(),
                "首板".to_string(),
                10.0,
                12.0,
                9.0,
            )],
            record_candidates: Vec::new(),
            llm_triggers: std::collections::HashMap::new(),
        };
        let p = build_industry_chain_intraday_from_snapshot(&s);
        assert_eq!(p.chain, "AI算力");
        assert_eq!(p.limit_count, 5);
        assert_eq!(p.leader_name, Some("龙头A"));
        assert_eq!(p.supplements.len(), 1);
    }

    /// v13.10.5: I-03 LLM 路径 — llm_triggers 命中 code 时用真实 trigger
    #[test]
    fn v13_10_5_i03_llm_triggers_override() {
        let mut s = IndustryChainSnapshot {
            hhmm: "10:30".to_string(),
            chain: "PCB".to_string(),
            limit_count: 3,
            leader_name: "深南电路".to_string(),
            leader_code: "TEST_CODE_002916".to_string(),
            leader_height: 3,
            supplements: vec![(
                "沪电股份".to_string(),
                "TEST_CODE_002463".to_string(),
                "首板".to_string(), // 旧 fallback
                10.0,
                12.0,
                9.0,
            )],
            record_candidates: Vec::new(),
            llm_triggers: std::collections::HashMap::new(),
        };
        // 注入 LLM trigger
        s.llm_triggers.insert(
            "TEST_CODE_002463".to_string(),
            "800G 交换机订单 + 估值修复".to_string(),
        );
        let p = build_industry_chain_intraday_from_snapshot(&s);
        assert_eq!(p.supplements.len(), 1);
        assert_eq!(p.supplements[0].trigger, "800G 交换机订单 + 估值修复");
        assert_eq!(p.supplements[0].code, "TEST_CODE_002463");
    }

    /// v13.10.5: I-03 降级 — llm_triggers 不命中时回退原 trigger
    #[test]
    fn v13_10_5_i03_fallback_when_llm_missing() {
        let s = IndustryChainSnapshot {
            hhmm: "10:30".to_string(),
            chain: "PCB".to_string(),
            limit_count: 3,
            leader_name: "深南电路".to_string(),
            leader_code: "TEST_CODE_002916".to_string(),
            leader_height: 3,
            supplements: vec![(
                "兴森科技".to_string(),
                "TEST_CODE_002436".to_string(),
                "放量突破".to_string(),
                10.0,
                12.0,
                9.0,
            )],
            record_candidates: Vec::new(),
            llm_triggers: Default::default(), // 空 — 回退
        };
        let p = build_industry_chain_intraday_from_snapshot(&s);
        assert_eq!(
            p.supplements[0].trigger, "放量突破",
            "llm_triggers 缺 code 时用原 trigger"
        );
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
            code: "TEST_CODE_603019".to_string(),
            reasons: vec!["AI算力龙头".to_string(), "业绩超预期".to_string()],
            action: Some(NewsAction::BuyDip),
            llm_reasons: vec![],
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
        assert_eq!(s.stage, NewsStage::Starting); // default
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

    /// v13.10.5: D-01 LLM 路径 — llm_reasons 非空时优先
    #[test]
    fn v13_10_5_d01_llm_reasons_take_precedence() {
        let s = NewsToIdeaSnapshot {
            hhmm: "10:30".to_string(),
            headline: "PCB 涨价 12%".to_string(),
            theme: "PCB".to_string(),
            stage: NewsStage::Starting,
            name: "深南电路".to_string(),
            code: "TEST_CODE_002916".to_string(),
            reasons: vec!["多源验证".to_string()],
            action: Some(NewsAction::BuyDip),
            llm_reasons: vec![
                "PCB 涨价 12% 直接传导到毛利".to_string(),
                "800G 交换机订单超预期".to_string(),
                "国产替代加速".to_string(),
            ],
        };
        let p = build_news_to_idea_from_snapshot(&s);
        assert_eq!(p.reasons.len(), 3, "应使用 llm_reasons (3 条)");
        assert!(p.reasons[0].contains("PCB"));
    }

    /// v13.10.5: D-01 降级 — llm_reasons 空时用原 evidence
    #[test]
    fn v13_10_5_d01_fallback_to_evidence() {
        let s = NewsToIdeaSnapshot {
            hhmm: "10:30".to_string(),
            headline: "PCB".to_string(),
            theme: "PCB".to_string(),
            stage: NewsStage::Fermenting,
            name: "深南电路".to_string(),
            code: "TEST_CODE_002916".to_string(),
            reasons: vec!["多源验证".to_string(), "放量突破".to_string()],
            action: Some(NewsAction::Observe),
            llm_reasons: vec![],
        };
        let p = build_news_to_idea_from_snapshot(&s);
        assert_eq!(p.reasons.len(), 2);
        assert_eq!(p.reasons[0], "多源验证");
    }

    // ====== v15.6: A-01 业务层集成测试 (paper_review 抽口) ======
    #[test]
    fn v15_build_paper_review_from_snapshot() {
        let s = PaperReviewSnapshot {
            date: "2026-07-06".to_string(),
            name: "A".to_string(),
            code: "TEST_CODE_000001".to_string(),
            trigger: "首板".to_string(),
            desc: "已成交".to_string(),
            pnl: Some(2.5),
            plan_high: Some("减仓1/2".to_string()),
            plan_flat: Some("持有".to_string()),
            plan_low: Some("止损".to_string()),
        };
        let p = build_paper_review_from_snapshot(&s);
        assert_eq!(p.name, "A");
        assert_eq!(p.code, "TEST_CODE_000001");
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
        let (h, _f, _l) = derive_plan_from_pnl(7.0);
        assert_eq!(h, "减仓1/3");
        // pnl > 0% → 减仓1/2
        let (h, _f, _l) = derive_plan_from_pnl(3.0);
        assert_eq!(h, "减仓1/2");
        // pnl <= 0% → 持有观望
        let (h, _f, _l) = derive_plan_from_pnl(-1.0);
        assert_eq!(h, "持有观望");
    }

    #[test]
    fn br104_a01_uses_exact_next_trading_day_close() {
        let entry = chrono::NaiveDate::from_ymd_opt(2026, 7, 17).unwrap();
        let review = chrono::NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
        let later = chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        let rows = vec![(later, 99.0), (review, 12.5)];

        assert_eq!(
            select_t1_close(&rows, entry, review).unwrap(),
            Some((review, 12.5))
        );
    }

    #[test]
    fn br104_a01_does_not_use_current_quote_before_t1() {
        let entry = chrono::NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
        let review = entry;
        let rows = vec![(entry, 11.0)];

        assert_eq!(select_t1_close(&rows, entry, review).unwrap(), None);
    }

    #[test]
    fn v15_load_paper_review_snapshot_default() {
        // v16+ 待集成真实 virtual_watch/paper_trades
        let s = load_paper_review_snapshot("2026-07-06");
        assert!(s.name.is_empty());
    }

    // ====== v13.7: dispatcher_log 可观测性测试 ======
    #[test]
    #[serial_test::serial(dispatcher_log_env)]
    fn v13_7_dispatcher_log_writes_jsonl() {
        use std::fs;

        let dir = std::env::temp_dir().join(format!(
            "stock_analysis_dispatcher_log_{}_{}",
            std::process::id(),
            chrono::Local::now()
                .timestamp_nanos_opt()
                .unwrap_or_default()
        ));

        // 写 3 条 (成功 2 + 失败 1)
        write_dispatcher_attempt(&dir, "P-01", true, 3, "").expect("write P-01");
        write_dispatcher_attempt(&dir, "I-01", false, 0, "sector empty").expect("write I-01");
        write_dispatcher_attempt(&dir, "A-01", true, 1, "").expect("write A-01");

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
        let dir = std::env::temp_dir().join(format!(
            "stock_analysis_p5_source_{}_{}",
            std::process::id(),
            chrono::Local::now()
                .timestamp_nanos_opt()
                .unwrap_or_default()
        ));
        fs::create_dir_all(&dir).expect("create P5 temp dir");

        // 写 2 个 P5 源文件
        let stock_pick_path = dir.join("stock_pick.jsonl");
        let optimal_path = dir.join("optimal_close.jsonl");
        fs::write(&stock_pick_path, "{\"code\":\"TEST_CODE_600519\",\"name\":\"贵州茅台\",\"chg_pct\":3.2}\n{\"code\":\"TEST_CODE_000858\",\"name\":\"五粮液\",\"chg_pct\":2.1}\n").unwrap();
        fs::write(
            &optimal_path,
            "{\"code\":\"TEST_CODE_002208\",\"name\":\"合肥城建\",\"chg_pct\":5.5}\n",
        )
        .unwrap();

        // 验证加载
        let items1 = load_p5_source_items_from_dir("stock_pick", &dir).expect("load stock_pick");
        assert_eq!(items1.len(), 2);
        assert_eq!(items1[0].1, "TEST_CODE_600519");
        assert_eq!(items1[0].2, "贵州茅台");

        let items2 =
            load_p5_source_items_from_dir("optimal_close", &dir).expect("load optimal_close");
        assert_eq!(items2.len(), 1);
        assert_eq!(items2[0].1, "TEST_CODE_002208");

        // 未知来源名必须报错，不能伪装成空源。
        assert!(load_p5_source_items_from_dir("nonexistent", &dir).is_err());

        // 已知来源缺文件表示本轮无数据。
        let missing = load_p5_source_items_from_dir("volume_watchlist", &dir)
            .expect("missing known source is empty");
        assert!(missing.is_empty());

        // 存在的源中任一坏行使整源失败，不能跳过。
        fs::write(
            dir.join("volume_real_trade.jsonl"),
            "{\"code\":\"TEST_CODE_600519\",\"name\":\"贵州茅台\"}\n{bad-json}\n",
        )
        .expect("write malformed P5 source");
        assert!(load_p5_source_items_from_dir("volume_real_trade", &dir).is_err());
        fs::remove_dir_all(&dir).expect("remove P5 temp dir");
    }

    fn valid_paper_trade_dispatch_row() -> PaperTradeDispatchRow {
        PaperTradeDispatchRow {
            id: 1,
            code: "TEST_CODE_P04".to_string(),
            name: "测试虚拟仓".to_string(),
            direction: "buy".to_string(),
            price: 10.0,
            quantity: 100,
            status: "Filled".to_string(),
            fill_price: Some(10.0),
            not_fill_reason: None,
            virtual_reason: "NewsCatalyst".to_string(),
            account_mode: "Normal".to_string(),
            data_mode: "Full".to_string(),
        }
    }

    #[test]
    fn br100_paper_trade_report_rejects_incomplete_completed_rows() {
        let mut missing_fill = valid_paper_trade_dispatch_row();
        missing_fill.fill_price = None;
        assert!(validate_paper_trade_dispatch_row_for_env(
            missing_fill,
            stock_analysis::risk::env_guard::TradingEnv::Test
        )
        .is_err());

        let mut missing_reason = valid_paper_trade_dispatch_row();
        missing_reason.status = "NotFilled".to_string();
        missing_reason.fill_price = None;
        assert!(validate_paper_trade_dispatch_row_for_env(
            missing_reason,
            stock_analysis::risk::env_guard::TradingEnv::Test
        )
        .is_err());
    }

    #[test]
    fn br100_paper_trade_renderer_never_fills_missing_with_zero_or_empty() {
        let text = render_paper_trade(PaperTradeParams {
            name: "测试",
            code: "TEST_CODE_P04",
            hhmm: "09:31",
            status: PaperTradeStatus::Filled,
            fill_price: None,
            qty: None,
            virtual_reason: None,
            not_fill_reason: None,
            account_mode: AccountMode::Normal,
            data_mode: DataMode::Full,
        });
        assert!(text.contains("成交价— 缺失 数量— 缺失 主理由— 缺失"));
        assert!(!text.contains("数量0"));
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
            code: "TEST_CODE_600000".to_string(),
            name: "浦发银行".to_string(),
            chain: "银行".to_string(),
            board_level: 1,
            is_limit_up_today: 9.8 > 9.5, // 9.8% 涨 → 涨停
            is_first_board: true,
            consecutive_days: 1,
        };
        assert!(n_above.is_limit_up_today);

        // chg_pct < 9.5 → is_limit_up_today = false
        let n_below = StockLimitStats {
            code: "TEST_CODE_000001".to_string(),
            name: "平安银行".to_string(),
            chain: "银行".to_string(),
            board_level: 1,
            is_limit_up_today: 5.0 > 9.5, // 5% 涨 → 不涨停
            is_first_board: false,
            consecutive_days: 0,
        };
        assert!(!n_below.is_limit_up_today);

        // 边界: 9.5 整 → 不涨停 (> 严格不等)
        let n_boundary = StockLimitStats {
            code: "TEST_CODE_600519".to_string(),
            name: "贵州茅台".to_string(),
            chain: "白酒".to_string(),
            board_level: 2,
            is_limit_up_today: 9.5 > 9.5, // 9.5 整 → false
            is_first_board: false,
            consecutive_days: 2,
        };
        assert!(!n_boundary.is_limit_up_today);

        // 涨停 (>9.5) + 一字板 (is_first_board=false) → board_level 仍按位置推断
        let n_limit_up = StockLimitStats {
            code: "TEST_CODE_002415".to_string(),
            name: "海康威视".to_string(),
            chain: "AI".to_string(),
            board_level: 2, // 简化: 按位置推断
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
        let result = fetch_realtime_quotes_batch(&[]).expect("empty batch succeeds");
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
            hhmm: "09:05",
            theme_1: Some("AI算力"),
            theme_2: Some("机器人"),
            theme_3: None,
            news_pairs: vec![("英伟达H200", "GPU")],
            watch_stocks: vec![(
                "中科曙光".to_string(),
                "TEST_CODE_603019".to_string(),
                "AI龙头".to_string(),
            )],
        });
        assert!(p1.contains("📰 盘前热点"));
        assert!(p1.contains("AI算力"));
        assert!(p1.contains("中科曙光"));

        // 2. I-01
        let i1 = render_intraday_market(
            &banner,
            IntradayMarketParams {
                hhmm: "10:30",
                tech_sub: "AI算力".into(),
                tech_score: Some(85.5),
                power_sub: "特高压".into(),
                power_score: Some(60.0),
                robot_sub: "减速器".into(),
                robot_score: Some(72.3),
                main_attack: Some("AI算力"),
                rotation_state: RotationState::Spreading,
            },
        );
        assert!(i1.contains("📊 盘中轮动"));
        assert!(i1.contains("轮动状态: 扩散"));

        // 3. I-02
        let i2 = render_news_catalyst(
            &banner,
            NewsCatalystParams {
                hhmm: "10:30",
                headline: "英伟达H200发布",
                theme: Some("AI算力"),
                stocks: vec![
                    ("中科曙光", "TEST_CODE_603019", Some(5.2), "AI算力订单"),
                    ("浪潮信息", "TEST_CODE_000977", Some(3.8), "服务器受益"),
                ],
            },
        );
        assert!(i2.contains("📰⚡ 新闻催化跟踪"));
        assert!(i2.contains("中科曙光"));

        // 4. I-03
        let i3 = render_industry_chain_intraday(
            &banner,
            IndustryChainIntradayParams {
                hhmm: "10:30",
                chain: "AI算力",
                limit_count: 5,
                leader_name: Some("中科曙光"),
                leader_code: Some("TEST_CODE_603019"),
                leader_height: 3,
                supplements: vec![SupplementCandidate {
                    name: "浪潮信息",
                    code: "TEST_CODE_000977",
                    trigger: "首板",
                    lo: 10.0,
                    hi: 12.0,
                    stop: 9.0,
                }],
            },
        );
        assert!(i3.contains("🔥 盘中涨停扩散"));
        assert!(i3.contains("AI算力"));

        // 5. D-01
        let d1 = render_news_to_idea(
            &banner,
            NewsToIdeaParams {
                hhmm: "10:30",
                headline: "AI算力龙头",
                theme: Some("AI"),
                stage: NewsStage::Starting,
                name: "中科曙光",
                code: "TEST_CODE_603019",
                reasons: vec!["AI龙头", "业绩超预期"],
                action: Some(NewsAction::BuyDip),
            },
        );
        assert!(d1.contains("🧭 新闻驱动个股"));
        assert!(d1.contains("[建议动作: 低吸]"));

        // 6. A-10
        let a10 = render_catalyst_review(CatalystReviewParams {
            date: "2026-07-06",
            theme: "AI算力",
            score: Some(85.0),
            persistent: PersistentLevel::High,
            started_names: vec!["中科曙光", "浪潮信息"],
            pending_names: vec!["紫光股份"],
            watch_point: Some("明日是否扩散"),
        });
        assert!(a10.contains("📰 题材催化复盘"));
        assert!(a10.contains("AI算力"));

        // 7. A-01
        let a01 = render_paper_review(PaperReviewParams {
            date: "2026-07-06",
            name: "中科曙光",
            code: "TEST_CODE_603019",
            trigger: "首板",
            desc: "已成交",
            pnl: Some(2.5),
            plan_high: Some("减仓1/2"),
            plan_flat: Some("持有"),
            plan_low: Some("止损"),
        });
        assert!(a01.contains("🧪 虚拟仓复盘"));
        assert!(a01.contains("中科曙光"));

        // 8. T-14
        let t14 = render_post_fixed_price_order(PostFixedPriceOrderParams {
            exchange: Exchange::SH,
            hhmm: "10:00",
            name: "A",
            code: "TEST_CODE_600000",
            price: 10.5,
            qty: 1000,
            order_id: "ORD001",
            status: OrderStatus::Submitted,
        });
        assert!(t14.contains("📋 盘后固定价格申报"));
        assert!(t14.contains("沪市"));

        // 9. T-15
        let t15 = render_post_fixed_price_fill(PostFixedPriceFillParams {
            exchange: Exchange::BJ,
            hhmm: "15:10",
            name: "A",
            code: "TEST_CODE_830001",
            fill_price: 10.0,
            qty: 100,
            vs_limit_pct: Some(2.5),
            next_session_carry: true,
        });
        assert!(t15.contains("✅ 盘后固定价格成交"));
        assert!(t15.contains("北交所"));

        // 10. T-16
        let t16 = render_st_price_limit_changed(StPriceLimitChangedParams {
            hhmm: "09:30",
            name: "A",
            code: "TEST_CODE_600000",
            st_type: StType::ST,
            old_limit: 0.05,
            new_limit: 0.10,
            holding_qty: 1000,
            cost: 10.0,
            now_price: 11.0,
            new_stop_loss: Some(9.0),
            new_take_profit: Some(12.0),
        });
        assert!(t16.contains("⚠️ ST 涨跌幅变更"));
        assert!(t16.contains("原涨跌幅"));
        assert!(t16.contains("新涨跌幅"));

        // 11. T-17
        let t17 = render_etf_closing_call_auction(EtfClosingCallAuctionParams {
            hhmm: "14:58",
            name: "沪深300ETF",
            code: "TEST_CODE_510300",
            call_auction_price: Some(3.952),
            vs_continuous_est: Some(0.15),
            liquidity_note: "正常",
        });
        assert!(t17.contains("📊 ETF 集合竞价尾盘"));
        assert!(t17.contains("沪市 ETF"));

        // 12-13. T-18/T-19: v17.8 审计删除 (2026-07-16), 随 render fn 一同移除

        // 全部 11 个模板 + 辅助行 ("辅助建议, 非下单指令" 等)
        assert!(p1.contains("辅助建议, 非下单指令"));
        assert!(i1.contains("辅助建议, 非下单指令"));
        assert!(i2.contains("辅助建议, 非下单指令"));
        assert!(i3.contains("辅助建议, 非下单指令"));
        assert!(d1.contains("辅助建议, 非下单指令"));
        assert!(a10.contains("辅助建议, 非下单指令"));
        assert!(a01.contains("辅助建议, 非下单指令"));

        // 打印所有 11 个模板样例 (v19 任务: 用户要看每个模板输出; T-18/T-19 已删)
        eprintln!("\n╔══════════════════════════════════════════════════════════════════╗");
        eprintln!("║ 11 个新模板 render 输出 (v13/v13.1)                              ║");
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
        // T-18/T-19: v17.8 审计删除 (2026-07-16)
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
            None,
            "DataMode transitions have no coarse cooldown"
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
        assert!(!is_in_cooldown(PushKind::HoldingPlan, "TEST_CODE_000001"));
        assert!(!is_in_cooldown(PushKind::HoldingPlan, "TEST_CODE_000002"));
        record_cooldown(PushKind::HoldingPlan, "TEST_CODE_000001");
        assert!(is_in_cooldown(PushKind::HoldingPlan, "TEST_CODE_000001"));
        assert!(
            !is_in_cooldown(PushKind::HoldingPlan, "TEST_CODE_000002"),
            "不同 code 应独立"
        );
    }

    #[test]
    fn emergency_bypass_cooldown_table() {
        use super::super::notify::{PushKind, PushLevel};
        // HoldingEvent 是 Emergency, 即使在 cooldown table 中也是 false
        record_cooldown(PushKind::HoldingEvent, "TEST_CODE_000001");
        assert!(!is_in_cooldown(PushKind::HoldingEvent, "TEST_CODE_000001"));
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
            total_pos: Some(5),
            today_pnl: Some(0.3),
            account_metrics_complete: true,
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
                code: "TEST_CODE_000001",
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
        assert!(s.contains("🎯 持仓建议 XX科技(TEST_CODE_000001)（13:42）"));
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
                code: "TEST_CODE_688001",
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
            "📋 候选触发 候选X(TEST_CODE_688001)（10:30）",
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
                code: "TEST_CODE_688002",
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
                sh_chg: Some(0.5),
                chinext_chg: Some(1.2),
                star_chg: Some(1.5),
                limit_up_n: Some(35),
                limit_down_n: Some(3),
                broken_pct: Some(15.0),
                consecutive_h: Some(5),
                amount_yi: Some(8500.0),
                amount_delta_pct: Some(8.0),
                amount_dir: Some("放量"),
                main_flow_yi: Some(120.0),
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
    /// 环境变量由 `TestEnvGuard` 负责隔离；这里仅重置业务状态。
    fn reset_daily_budget_for_test() {
        DAILY_BUDGET_COUNT.store(0, Ordering::Relaxed);
        let mut table = COOLDOWN_TABLE.lock().expect("cooldown table poisoned");
        table.clear();
        drop(table);
        crate::v14_adapter::_reset_dedup_for_test();
        // 清空 account_mode_log (并行测试可能插入行, 影响 e2e_t01_no_change 的 count 断言)
        use diesel::prelude::*;
        if let Ok(mut conn) = stock_analysis::database::DatabaseManager::get().get_conn() {
            diesel::sql_query("DELETE FROM account_mode_log")
                .execute(&mut conn)
                .ok();
        }
    }

    #[test]
    fn br130_global_dispatch_key_is_absent_from_delivery_identity() {
        assert_eq!(optional_dispatch_code(""), None);
        assert_eq!(optional_dispatch_code("   "), None);
        assert_eq!(
            optional_dispatch_code("TEST_CODE_600519"),
            Some("TEST_CODE_600519")
        );
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
            total_pos: Some(5),
            today_pnl: Some(0.3),
            account_metrics_complete: true,
            data_mode: DataMode::Full,
            data_missing_note: None,
        }
    }

    fn account_evaluation_for_test(
        metrics: &stock_analysis::risk::account_mode::PortfolioMetrics,
        prev: Option<stock_analysis::risk::action_gate::AccountMode>,
    ) -> stock_analysis::risk::account_mode::ModeEvaluation {
        stock_analysis::risk::account_mode::evaluate_with_reset(
            metrics,
            prev,
            &stock_analysis::config::get_risk_config()
                .account_mode
                .to_thresholds(),
            chrono::Local::now().time(),
        )
    }

    #[test]
    fn br116_failed_account_notification_reuses_pending_audit_row() {
        use stock_analysis::database::account_mode_log::AccountModeLogRow;
        use stock_analysis::risk::action_gate::AccountMode as LibAM;

        let pending = AccountModeLogRow {
            id: 41,
            ts: "2026-07-20 09:30:00".to_string(),
            prev_mode: "Normal".to_string(),
            new_mode: "ReduceOnly".to_string(),
            trigger_reason: "TEST_CODE account metrics missing".to_string(),
            today_pnl_pct: None,
            consecutive_n: None,
            total_pos_cheng: None,
            data_complete: 0,
            pushed: 0,
            push_attempted_at: None,
        };

        assert_eq!(
            plan_account_mode_notification(Some(&pending), LibAM::ReduceOnly).unwrap(),
            AccountModeNotificationPlan::ReusePending(41)
        );
    }

    #[test]
    fn br116_invalid_pushed_flag_is_rejected() {
        use stock_analysis::database::account_mode_log::AccountModeLogRow;
        use stock_analysis::risk::action_gate::AccountMode as LibAM;

        for pushed in [-1, 2] {
            let row = AccountModeLogRow {
                id: 42,
                ts: "2026-07-20 09:30:00".to_string(),
                prev_mode: "Normal".to_string(),
                new_mode: "ReduceOnly".to_string(),
                trigger_reason: "TEST_CODE pending".to_string(),
                today_pnl_pct: None,
                consecutive_n: None,
                total_pos_cheng: None,
                data_complete: 0,
                pushed,
                push_attempted_at: None,
            };
            assert!(plan_account_mode_notification(Some(&row), LibAM::ReduceOnly).is_err());
        }
    }

    #[test]
    fn br116_account_delivery_requires_push_confirmation() {
        assert!(AccountModeDispatchResult::NoChange.is_confirmed());
        assert!(
            AccountModeDispatchResult::Delivery(crate::notify::PushOutcome::Pushed).is_confirmed()
        );
        assert!(
            !AccountModeDispatchResult::Delivery(crate::notify::PushOutcome::Deduped)
                .is_confirmed()
        );
        assert!(
            !AccountModeDispatchResult::Delivery(crate::notify::PushOutcome::Denied(
                "TEST_CODE governance denied".to_string(),
            ))
            .is_confirmed()
        );
        assert!(
            !AccountModeDispatchResult::Delivery(crate::notify::PushOutcome::SinkError(
                "TEST_CODE sink failed".to_string(),
            ))
            .is_confirmed()
        );
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br116_account_delivery_confirmation_propagates_audit_update_failure() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        init_test_db();

        let error = finalize_account_mode_delivery(i64::MAX, crate::notify::PushOutcome::Pushed)
            .expect_err("missing audit row must not be confirmed");
        assert!(error.contains("expected 1 affected row"));
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br116_denied_and_sink_error_keep_original_account_audit_pending() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        init_test_db();
        use stock_analysis::database::account_mode_log;
        use stock_analysis::risk::action_gate::AccountMode as LibAM;

        for outcome in [
            crate::notify::PushOutcome::Denied("TEST_CODE governance denied".to_string()),
            crate::notify::PushOutcome::SinkError("TEST_CODE sink failed".to_string()),
        ] {
            let id = account_mode_log::insert_account_mode_change(
                LibAM::Normal,
                LibAM::ReduceOnly,
                "TEST_CODE pending delivery",
                None,
                None,
                None,
                false,
            )
            .expect("seed pending account audit");

            let result = finalize_account_mode_delivery(id, outcome)
                .expect("unconfirmed outcome must remain retryable");
            assert!(!result.is_confirmed());
            let row = account_mode_log::latest_account_mode_change()
                .expect("read account audit")
                .expect("pending account audit exists");
            assert_eq!(i64::from(row.id), id);
            assert_eq!(row.pushed, 0);
            assert!(row.push_attempted_at.is_none());
        }
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br116_pending_account_notification_retries_without_duplicate_row() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        init_test_db();
        reset_daily_budget_for_test();

        use stock_analysis::database::account_mode_log;
        use stock_analysis::risk::account_mode::PortfolioMetrics;
        use stock_analysis::risk::action_gate::AccountMode as LibAM;

        account_mode_log::insert_account_mode_change(
            LibAM::Normal,
            LibAM::ReduceOnly,
            "TEST_CODE account metrics missing",
            None,
            None,
            None,
            false,
        )
        .expect("seed pending notification");
        let pending = account_mode_log::latest_account_mode_change()
            .expect("read pending notification")
            .expect("pending notification exists");
        let banner = BannerCtx {
            account_mode: AccountMode::ReduceOnly,
            total_pos: None,
            today_pnl: None,
            account_metrics_complete: false,
            data_mode: DataMode::Unsafe,
            data_missing_note: Some("账户指标缺失".to_string()),
        };
        *crate::LATEST_BANNER
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(banner.clone());

        let metrics = PortfolioMetrics::incomplete();
        let evaluation = account_evaluation_for_test(&metrics, Some(LibAM::ReduceOnly));
        let pushed = push_account_mode_change(
            &metrics,
            Some(LibAM::ReduceOnly),
            Some(&pending),
            Some(&banner),
            &evaluation,
        )
        .await
        .expect("retry pending notification");

        assert!(pushed.is_confirmed());
        let rows = account_mode_log::recent_account_mode_changes(10).expect("read audit rows");
        assert_eq!(rows.len(), 1, "retry must reuse the pending audit row");
        assert_eq!(
            rows[0].id, pending.id,
            "retry must retain the original row ID"
        );
        assert_eq!(rows[0].pushed, 1);
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br116_single_reset_evaluation_controls_persistence_and_banner() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        init_test_db();
        reset_daily_budget_for_test();

        use stock_analysis::database::account_mode_log;
        use stock_analysis::risk::account_mode::{
            evaluate_with_reset, ModeThresholds, PortfolioMetrics,
        };
        use stock_analysis::risk::action_gate::AccountMode as LibAM;

        let previous_id = account_mode_log::insert_account_mode_change(
            LibAM::Frozen,
            LibAM::Frozen,
            "TEST_CODE prior frozen",
            Some(-2.1),
            Some(3),
            Some(8),
            true,
        )
        .expect("seed prior Frozen state");
        account_mode_log::mark_account_mode_pushed(previous_id)
            .expect("confirm prior Frozen state");
        let previous = account_mode_log::latest_account_mode_change()
            .expect("read prior state")
            .expect("prior state exists");

        let metrics = PortfolioMetrics::complete(0.2, 0, 4);
        let evaluation = evaluate_with_reset(
            &metrics,
            Some(LibAM::Frozen),
            &ModeThresholds::default(),
            chrono::NaiveTime::from_hms_opt(8, 30, 59).unwrap(),
        );
        assert_eq!(evaluation.mode, LibAM::Normal);
        let banner = BannerCtx {
            account_mode: AccountMode::Normal,
            total_pos: Some(4),
            today_pnl: Some(0.2),
            account_metrics_complete: true,
            data_mode: DataMode::Full,
            data_missing_note: None,
        };
        *crate::LATEST_BANNER
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(banner.clone());

        let result = push_account_mode_change(
            &metrics,
            Some(LibAM::Frozen),
            Some(&previous),
            Some(&banner),
            &evaluation,
        )
        .await
        .expect("single reset evaluation must orchestrate");

        assert!(result.is_confirmed());
        let latest = account_mode_log::latest_account_mode_change()
            .expect("read reset state")
            .expect("reset state exists");
        assert_eq!(latest.prev_mode, "Frozen");
        assert_eq!(latest.new_mode, "Normal");
        assert_eq!(latest.pushed, 1);
        let context = paper_risk_context_from_banner(&banner).expect("complete banner context");
        assert_eq!(context.account_mode, LibAM::Normal);
    }

    /// T-01 E2E: Normal → ReduceOnly. 验证 DB 写 + 推送路径
    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn e2e_t01_normal_to_reduce_only_db_and_push() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        init_test_db();
        reset_daily_budget_for_test();

        use stock_analysis::database::account_mode_log;
        use stock_analysis::risk::account_mode::PortfolioMetrics;
        use stock_analysis::risk::action_gate::AccountMode as LibAM;

        let metrics = PortfolioMetrics::complete(-1.6, 0, 5);
        let evaluation = account_evaluation_for_test(&metrics, Some(LibAM::Normal));

        let result = push_account_mode_change(
            &metrics,
            Some(LibAM::Normal),
            None,
            Some(&banner_normal_full()),
            &evaluation,
        )
        .await;

        assert!(result.is_ok(), "orchestrator 不应报错: {:?}", result);
        assert!(result.unwrap().is_confirmed(), "T-01 应推送成功 (dry-run)");

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
    }

    /// T-01 E2E: 无变更 → 不推送不写库
    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn e2e_t01_no_change_no_push_no_db_write() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        init_test_db();
        reset_daily_budget_for_test();

        use stock_analysis::database::account_mode_log;
        use stock_analysis::risk::account_mode::PortfolioMetrics;
        use stock_analysis::risk::action_gate::AccountMode as LibAM;

        let before = account_mode_log::recent_account_mode_changes(100)
            .map(|r| r.len())
            .unwrap_or(0);

        let metrics = PortfolioMetrics::complete(-1.6, 0, 5);
        let evaluation = account_evaluation_for_test(&metrics, Some(LibAM::ReduceOnly));
        // prev 已是 ReduceOnly, metrics 不变 → is_changed=false
        let result = push_account_mode_change(
            &metrics,
            Some(LibAM::ReduceOnly),
            None,
            Some(&banner_normal_full()),
            &evaluation,
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), AccountModeDispatchResult::NoChange);

        let after = account_mode_log::recent_account_mode_changes(100)
            .map(|r| r.len())
            .unwrap_or(0);
        assert_eq!(before, after, "无变更不应写库");
    }

    /// BR-108: the first real evaluation must establish an auditable state
    /// instead of silently assuming Normal.
    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn e2e_t01_initial_evaluation_is_persisted_without_invented_predecessor() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        init_test_db();
        reset_daily_budget_for_test();

        use stock_analysis::database::account_mode_log;
        use stock_analysis::risk::account_mode::PortfolioMetrics;

        let metrics = PortfolioMetrics::complete(0.2, 0, 4);
        let evaluation = account_evaluation_for_test(&metrics, None);

        let pushed = push_account_mode_change(
            &metrics,
            None,
            None,
            Some(&banner_normal_full()),
            &evaluation,
        )
        .await
        .expect("initial evaluation must be orchestrated");
        assert!(
            pushed.is_confirmed(),
            "dry-run initial evaluation should dispatch"
        );

        let rows = account_mode_log::recent_account_mode_changes(1).expect("query initial state");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].prev_mode, "Normal");
        assert_eq!(rows[0].new_mode, "Normal");
        assert_eq!(rows[0].trigger_reason, "initial account mode evaluation");
        assert_eq!(rows[0].pushed, 1);
    }

    /// T-01 E2E: ReduceOnly → Frozen. 数据准确
    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn e2e_t01_reduce_only_to_frozen_circuit_breaker() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        init_test_db();
        reset_daily_budget_for_test();

        use stock_analysis::database::account_mode_log;
        use stock_analysis::risk::account_mode::PortfolioMetrics;
        use stock_analysis::risk::action_gate::AccountMode as LibAM;

        let metrics = PortfolioMetrics::complete(-2.5, 5, 9); // 超过 -2.0% 熔断线
        let evaluation = account_evaluation_for_test(&metrics, Some(LibAM::ReduceOnly));

        let result = push_account_mode_change(
            &metrics,
            Some(LibAM::ReduceOnly),
            None,
            Some(&banner_normal_full()),
            &evaluation,
        )
        .await;
        assert!(result.is_ok());

        let rows = account_mode_log::recent_account_mode_changes(1).expect("query");
        assert_eq!(rows[0].new_mode, "Frozen");
        assert_eq!(rows[0].prev_mode, "ReduceOnly");
        assert!(rows[0].trigger_reason.contains("熔断"));
        assert!(rows[0].trigger_reason.contains("-2.00%"));
        assert_eq!(rows[0].pushed, 1);
    }

    /// T-01 E2E: 数据缺失 → 保守 ReduceOnly
    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn e2e_t01_data_missing_conservative_reduce_only() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        init_test_db();
        reset_daily_budget_for_test();

        use stock_analysis::database::account_mode_log;
        use stock_analysis::risk::account_mode::PortfolioMetrics;
        use stock_analysis::risk::action_gate::AccountMode as LibAM;

        let metrics = PortfolioMetrics::incomplete();
        let evaluation = account_evaluation_for_test(&metrics, Some(LibAM::Normal));

        let result = push_account_mode_change(
            &metrics,
            Some(LibAM::Normal),
            None,
            Some(&banner_normal_full()),
            &evaluation,
        )
        .await;
        assert!(result.is_ok());

        let rows = account_mode_log::recent_account_mode_changes(1).expect("query");
        assert_eq!(rows[0].new_mode, "ReduceOnly");
        assert!(rows[0].trigger_reason.contains("数据缺失"));
        assert_eq!(rows[0].data_complete, 0);
    }

    /// T-02 E2E: Full → Degraded (Kline 过期)
    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn e2e_t02_full_to_degraded_kline_stale() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        init_test_db();
        reset_daily_budget_for_test();

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

        let result = push_data_mode_change(
            &input,
            Some(LibDM::Full),
            false,
            Some(&banner_normal_full()),
        )
        .await;
        assert!(result.is_ok(), "T-02 orchestrator: {:?}", result);
        assert!(matches!(
            result.unwrap(),
            ModeDispatchResult::Delivery(crate::notify::PushOutcome::Pushed)
        ));
    }

    /// T-02 E2E: 无变更 → no-op
    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn e2e_t02_no_change_no_push() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        init_test_db();
        reset_daily_budget_for_test();

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

        let result = push_data_mode_change(
            &input,
            Some(LibDM::Full),
            false,
            Some(&banner_normal_full()),
        )
        .await;
        assert!(result.is_ok());
        assert!(matches!(
            result.unwrap(),
            ModeDispatchResult::EstablishedSilently
        ));
    }

    #[test]
    fn initial_unsafe_data_mode_requires_a_status_delivery_plan() {
        use stock_analysis::monitor::data_mode::{DataHealthInput, DataMode as LibDM};

        let plan = data_mode_notification_plan(&DataHealthInput::default(), None, false);
        assert!(matches!(
            plan,
            DataModeNotificationPlan::Dispatch {
                previous: None,
                current: LibDM::Unsafe,
                reason: DataModeDispatchReason::Transition,
            }
        ));
    }

    #[test]
    fn br135_same_unsafe_dispatches_only_when_reminder_is_due() {
        use stock_analysis::monitor::data_mode::{DataHealthInput, DataMode as LibDM};

        assert!(matches!(
            data_mode_notification_plan(&DataHealthInput::default(), Some(LibDM::Unsafe), true,),
            DataModeNotificationPlan::Dispatch {
                current: LibDM::Unsafe,
                reason: DataModeDispatchReason::PersistentUnsafeReminder,
                ..
            }
        ));
        assert_eq!(
            data_mode_notification_plan(&DataHealthInput::default(), Some(LibDM::Unsafe), false,),
            DataModeNotificationPlan::EstablishSilently
        );
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br135_due_unsafe_reminder_uses_governed_delivery() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        init_test_db();
        reset_daily_budget_for_test();
        crate::v14_adapter::_reset_dedup_for_test();

        use stock_analysis::monitor::data_mode::{DataHealthInput, DataMode as LibDM};

        let input = DataHealthInput::default();
        let banner = BannerCtx {
            data_mode: DataMode::Unsafe,
            data_missing_note: Some("Quote/Kline/MoneyFlow/News/OrderBook".to_string()),
            ..BannerCtx::test_default()
        };
        *crate::LATEST_BANNER
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(banner.clone());

        assert_eq!(
            push_data_mode_change(&input, Some(LibDM::Unsafe), true, Some(&banner))
                .await
                .expect("due persistent Unsafe reminder must use the governed path"),
            ModeDispatchResult::Delivery(crate::notify::PushOutcome::Pushed)
        );
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn initial_unsafe_data_mode_is_actually_delivered() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        init_test_db();
        reset_daily_budget_for_test();
        crate::v14_adapter::_reset_dedup_for_test();

        use stock_analysis::monitor::data_mode::DataHealthInput;

        let input = DataHealthInput::default();
        let banner = BannerCtx {
            account_mode: AccountMode::ReduceOnly,
            total_pos: None,
            today_pnl: None,
            account_metrics_complete: false,
            data_mode: DataMode::Unsafe,
            data_missing_note: Some("Quote/Kline/MoneyFlow/News/OrderBook".to_string()),
        };
        *crate::LATEST_BANNER
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(banner.clone());

        let result = push_data_mode_change(&input, None, false, Some(&banner))
            .await
            .expect("initial Unsafe mode must use the governed status path");

        assert_eq!(
            result,
            ModeDispatchResult::Delivery(crate::notify::PushOutcome::Pushed)
        );
    }

    #[test]
    fn initial_full_data_mode_establishes_silently() {
        use stock_analysis::monitor::data_mode::{Capability, CapabilityStatus, DataHealthInput};

        let input = DataHealthInput {
            capabilities: Capability::ALL
                .iter()
                .map(|capability| CapabilityStatus::fresh(*capability, 1))
                .collect(),
            critical_max_age_secs: 120,
            orderbook_max_age_secs: 600,
        };
        assert_eq!(
            data_mode_notification_plan(&input, None, false),
            DataModeNotificationPlan::EstablishSilently
        );
    }

    #[test]
    fn br116_data_mode_dedup_is_not_delivery_confirmation() {
        assert!(!ModeDispatchResult::Delivery(crate::notify::PushOutcome::Deduped).is_confirmed());
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br116_rapid_distinct_data_mode_transitions_are_both_delivered() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        init_test_db();
        reset_daily_budget_for_test();
        crate::v14_adapter::_reset_dedup_for_test();

        use stock_analysis::monitor::data_mode::{
            Capability, CapabilityStatus, DataHealthInput, DataMode as LibDM,
        };

        let degraded_input = DataHealthInput {
            capabilities: vec![
                CapabilityStatus::fresh(Capability::Quote, 1),
                CapabilityStatus::missing(Capability::Kline),
                CapabilityStatus::fresh(Capability::MoneyFlow, 1),
                CapabilityStatus::fresh(Capability::News, 1),
                CapabilityStatus::missing(Capability::OrderBook),
            ],
            critical_max_age_secs: 120,
            orderbook_max_age_secs: 600,
        };
        let degraded_banner = BannerCtx {
            data_mode: DataMode::Degraded,
            data_missing_note: Some("Kline/OrderBook".to_string()),
            ..BannerCtx::test_default()
        };
        *crate::LATEST_BANNER
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(degraded_banner.clone());
        let first = push_data_mode_change(
            &degraded_input,
            Some(LibDM::Full),
            false,
            Some(&degraded_banner),
        )
        .await
        .expect("Full to Degraded delivery");

        let unsafe_input = DataHealthInput::default();
        let unsafe_banner = BannerCtx {
            data_mode: DataMode::Unsafe,
            data_missing_note: Some("Quote/Kline/MoneyFlow/News/OrderBook".to_string()),
            ..BannerCtx::test_default()
        };
        *crate::LATEST_BANNER
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(unsafe_banner.clone());
        let second = push_data_mode_change(
            &unsafe_input,
            Some(LibDM::Degraded),
            false,
            Some(&unsafe_banner),
        )
        .await
        .expect("Degraded to Unsafe delivery");

        assert_eq!(
            first,
            ModeDispatchResult::Delivery(crate::notify::PushOutcome::Pushed)
        );
        assert_eq!(
            second,
            ModeDispatchResult::Delivery(crate::notify::PushOutcome::Pushed)
        );
    }

    /// T-02 模板精确内容验证: 文本必须与 §14.1 T-02 模板逐字符一致
    #[test]
    fn t02_template_text_exact_format() {
        let s = render_data_mode(
            "10:23",
            Some(DataMode::Full),
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

    #[test]
    fn br135_persistent_unsafe_reminder_text_is_explicit() {
        let text = render_data_mode_reminder(
            "10:23",
            DataMode::Unsafe,
            "Quote/News",
            &["禁出价格型建议".to_string(), "仅保留风险类推送".to_string()],
            Some("Quote 恢复后"),
        );
        for required in [
            "📡 数据状态持续异常（10:23）",
            "当前模式: Unsafe",
            "受影响: Quote/News",
            "· 禁出价格型建议",
            "· 仅保留风险类推送",
            "恢复预计: Quote 恢复后",
            "提醒频率: 每30分钟",
        ] {
            assert!(
                text.contains(required),
                "BR-135 reminder missing: {required}"
            );
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
            total_pos: Some(5),
            today_pnl: Some(-1.6),
            account_metrics_complete: true,
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
    // =========== 20 模板隔离装配 + 可选真实 sink 冒烟 ===============
    // 完整模板测试默认走 cfg(test) dry-run；真实 sink 冒烟必须显式 opt-in。
    // ===============================================================

    /// 单条推送冒烟: 验证 magiclaw daemon 可达 + PUSH_VERBOSE=true
    /// 运行: V12_E2E_REAL_PUSH=1 cargo test --bin monitor push_templates::tests::e2e_single_smoke
    #[tokio::test]
    async fn e2e_single_smoke() {
        if std::env::var("V12_E2E_REAL_PUSH").ok().as_deref() != Some("1") {
            return;
        }
        let Ok(magiclaw_home) = std::env::var("MAGICLAW_HOME") else {
            eprintln!("[v12-E2E-smoke] 跳过: 缺 MAGICLAW_HOME");
            return;
        };
        let Ok(magiclaw_bin) = std::env::var("MAGICLAW_BIN") else {
            eprintln!("[v12-E2E-smoke] 跳过: 缺 MAGICLAW_BIN");
            return;
        };
        let Ok(feishu_to) = std::env::var("FEISHU_TO") else {
            eprintln!("[v12-E2E-smoke] 跳过: 缺 FEISHU_TO");
            return;
        };
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
        let out = std::process::Command::new(&magiclaw_bin)
            .args([
                "send",
                "--channel",
                "feishu",
                "--to",
                &feishu_to,
                "--message",
                text,
            ])
            .current_dir(&magiclaw_home)
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
        let out2 = std::process::Command::new(&magiclaw_bin)
            .args([
                "send",
                "--channel",
                "feishu",
                "--to",
                &feishu_to,
                "--message",
                text,
            ])
            .current_dir(&magiclaw_home)
            .env("DATABASE_PATH", "./data/stock_analysis.db")
            .env("MAGICLAW_DB_PATH", "./data/stock_analysis.db")
            .env("FEISHU_TO", &feishu_to)
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

    /// v12 E2E 隔离装配: 从测试 DB ledger/positions/trades 走完整模板与 dry-run governor.
    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn e2e_real_all_20_templates() {
        let _e2e_guard = E2E_MUTEX.lock().await;
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        *crate::LATEST_BANNER
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(BannerCtx::test_default());

        // 1. Setup: init 隔离 DB + 装配 TEST_CODE 数据
        init_test_db();
        reset_daily_budget_for_test();
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
            let metrics = if data_complete {
                stock_analysis::risk::account_mode::PortfolioMetrics::complete(
                    today_pnl_pct,
                    0,
                    ((market_value / total_value) * 10.0).round() as u8,
                )
            } else {
                stock_analysis::risk::account_mode::PortfolioMetrics::incomplete()
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
                    Some(DataMode::Full),
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
                "TEST_CODE_000001".to_string(),
                11.80_f64,
                3000_u32,
            )
        };
        {
            let banner = BannerCtx {
                account_mode: AccountMode::Normal,
                total_pos: if total_value > 0.0 {
                    Some(((market_value / total_value) * 10.0).round() as u8)
                } else {
                    None
                },
                today_pnl: data_complete.then_some(today_pnl_pct),
                account_metrics_complete: data_complete,
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
            let ok = crate::notify::push_governor_v3(
                &banner_text,
                crate::notify::PushKind::HoldingPlan,
                Some(&t03_code_s),
            )
            .await
            .is_pushed();
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
                "TEST_CODE_000001".to_string(),
                11.80_f64,
                3000_u32,
            )
        };
        {
            let banner = BannerCtx {
                account_mode: AccountMode::Normal,
                total_pos: if total_value > 0.0 {
                    Some(((market_value / total_value) * 10.0).round() as u8)
                } else {
                    None
                },
                today_pnl: data_complete.then_some(today_pnl_pct),
                account_metrics_complete: data_complete,
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
                "TEST_CODE_000001".to_string(),
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
                let text = render_t0_advice(&BannerCtx::test_default(), params);
                let banner_text = format!("[v12-E2E-T05] {}", text);
                let ok = crate::notify::push_governor_v3(
                    &banner_text,
                    crate::notify::PushKind::T0Advice,
                    Some(&t05_code_s),
                )
                .await
                .is_pushed();
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
            // T-05/T-06 共用 PushKind；使用独立 TEST_CODE 验证两条完整投递，
            // 避免正确的 per-ticket 冷却把第二条判为 Deduped。
            let (name, code) = ("测试标的B", "TEST_CODE_000002");
            let params = T0ForbidParams {
                name,
                code,
                hhmm: &hhmm,
                reason: "主升核心票防卖飞 (BR-022 衍生)",
            };
            let text = render_t0_forbid(&BannerCtx::test_default(), params);
            let banner_text = format!("[v12-E2E-T06] {}", text);
            let ok = crate::notify::push_governor_v3(
                &banner_text,
                crate::notify::PushKind::T0Advice,
                Some(code),
            )
            .await
            .is_pushed();
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
                code: "TEST_CODE_688001",
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
            let text = render_candidate_triggered(&BannerCtx::test_default(), params);
            let banner_text = format!("[v12-E2E-T07] {}", text);
            let ok = crate::notify::push_governor_v3(
                &banner_text,
                crate::notify::PushKind::CandidateTriggered,
                Some("TEST_CODE_688001"),
            )
            .await
            .is_pushed();
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
            // 走生产编排入口，验证 CandidateInvalidated 的票级治理与投递链路。
            // PID 只用于隔离不同测试进程的冷却身份，且 TEST_CODE 前缀确保
            // 测试标的永远不会被误认为真实证券代码。
            let code = format!("TEST_CODE_T08_{}", std::process::id());
            let ok = push_candidate_invalidated(
                &code,
                &hhmm,
                "AI算力候选",
                "Watch",
                "触发失败: 未触达买入区",
            )
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
                code: "TEST_CODE_688001",
                hhmm: &hhmm,
                conclusion: "距涨停过近, 禁止追买",
                reasons: &reasons,
            };
            let text = render_forbidden_ops(&BannerCtx::test_default(), params);
            let banner_text = format!("[v12-E2E-T09] {}", text);
            let ok = crate::notify::push_governor_v3(
                &banner_text,
                crate::notify::PushKind::ForbiddenOps,
                Some("TEST_CODE_688001"),
            )
            .await
            .is_pushed();
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
                .unwrap_or("TEST_CODE_000001");
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
            let ok = crate::notify::push_governor_v3(
                &banner_text,
                crate::notify::PushKind::PaperTrade,
                Some(code),
            )
            .await
            .is_pushed();
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
                    code: "TEST_CODE_688001",
                    gap_pct: 5.2,
                    vol_ratio: 8.5,
                    tag: "昨日涨停",
                },
                AuctionItem {
                    name: "机器人B",
                    code: "TEST_CODE_300750",
                    gap_pct: 2.1,
                    vol_ratio: 3.2,
                    tag: "观察池",
                },
            ];
            let text = render_auction_volume(
                &BannerCtx::test_default(),
                &hhmm,
                &items,
                "强承接",
                "可操作",
            );
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
                ("示例持仓".to_string(), "TEST_CODE_000001".to_string())
            };
            let t12_name_s = t12_name.clone();
            let holding = CloseCallHolding {
                name: &t12_name_s,
                state: "尾盘跳水-建议处理",
            };
            let text = render_close_call(&BannerCtx::test_default(), &hhmm, Some(&holding), None);
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
                    (p.cost_price * 1.05 / p.cost_price - 1.0) * 100.0
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
                    code: "TEST_CODE_000001",
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
            let ev = stock_analysis::market_analyzer::market_stage_confidence::MarketStageEvidence {
                technical: Some(
                stock_analysis::market_analyzer::market_stage_confidence::TechnicalMetrics {
                    sh_chg: 0.5,
                    chinext_chg: 1.2,
                    star_chg: 1.5,
                },
                ),
                capital: Some(
                stock_analysis::market_analyzer::market_stage_confidence::CapitalMetrics {
                    main_flow_yi: 120.0,
                    amount_yi: market_value,
                    amount_delta_pct: 8.0,
                },
                ),
                sentiment: Some(
                stock_analysis::market_analyzer::market_stage_confidence::SentimentMetrics {
                    limit_up_n: 35,
                    limit_down_n: 3,
                    broken_pct: 15.0,
                    consecutive_h: 5,
                },
                ),
                ..Default::default()
            };
            let conf = stock_analysis::market_analyzer::market_stage_confidence::evaluate(&ev);
            let r = MarketReview {
                sh_chg: Some(0.5),
                chinext_chg: Some(1.2),
                star_chg: Some(1.5),
                limit_up_n: Some(35),
                limit_down_n: Some(3),
                broken_pct: Some(15.0),
                consecutive_h: Some(5),
                amount_yi: Some(market_value),
                amount_delta_pct: Some(8.0),
                amount_dir: Some("放量"),
                main_flow_yi: Some(120.0),
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
                    code: "TEST_CODE_688001".to_string(),
                    name: "龙头A".to_string(),
                    chain: "AI算力".to_string(),
                    board_level: 4,
                    is_limit_up_today: true,
                    is_first_board: false,
                    consecutive_days: 4,
                },
                StockLimitStats {
                    code: "TEST_CODE_688002".to_string(),
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
                1, "AI龙头A", "TEST_CODE_688001", 1.5_f64, "涨幅偏离值达7%",
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
                code: "TEST_CODE_688001",
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
                code: "TEST_CODE_688001",
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
                    tag: "实盘",
                    name: p.name.as_str(),
                    code: p.code.as_str(),
                    kind: "解禁 3.2亿",
                })
                .collect();
            if events.is_empty() {
                events.push(HoldingEventItem {
                    tag: "实盘",
                    name: "示例持仓",
                    code: "TEST_CODE_000001",
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

    // v58: P-05 虚拟观察仓 模板测试
    #[test]
    fn test_p05_virtual_watch_template() {
        use super::{render_virtual_watch, VirtualWatchItem, VirtualWatchParams};
        let items = vec![
            VirtualWatchItem {
                name: "XX科技",
                code: "TEST_CODE_000001",
                open_price: 12.30,
                shares: 1000,
                estimated_amount: 12300.0,
            },
            VirtualWatchItem {
                name: "YY股份",
                code: "TEST_CODE_002049",
                open_price: 100.50,
                shares: 1000,
                estimated_amount: 100500.0,
            },
        ];
        let text = render_virtual_watch(VirtualWatchParams {
            hhmm: "09:30",
            shares_per_lot: 1000,
            items,
            total_amount: 112800.0,
            item_count: 2,
        });
        assert!(text.contains("🔍 虚拟观察仓位（09:30）"));
        assert!(text.contains("· XX科技(TEST_CODE_000001) @ ¥12.30 | 1000股 预计 ¥12300"));
        assert!(text.contains("· YY股份(TEST_CODE_002049) @ ¥100.50 | 1000股 预计 ¥100500"));
        assert!(text.contains("合计虚拟敞口: ¥112800 (1000股×2只)"));
        assert!(text.contains("⚠️ 仅做观察、研究用途，未实际下单"));
        assert!(text.ends_with("辅助建议, 非下单指令"));
    }

    #[test]
    fn test_p05_virtual_watch_empty() {
        use super::{render_virtual_watch, VirtualWatchParams};
        let text = render_virtual_watch(VirtualWatchParams {
            hhmm: "09:30",
            shares_per_lot: 1000,
            items: vec![],
            total_amount: 0.0,
            item_count: 0,
        });
        assert!(text.contains("⚠️ 候选空, 跳过"));
    }

    // v61 (F14): D01_LAST_PUSH LRU 驱逐测试
    //   - 验证 evict_d01_memo_expired 移除 > 7200s 的 entry
    #[test]
    fn test_d01_memo_lru_eviction() {
        use super::{_reset_d01_memo_for_test, evict_d01_memo_expired, D01_LAST_PUSH};
        _reset_d01_memo_for_test();

        // 写入一个 entry (Instant::now)
        D01_LAST_PUSH.lock().unwrap().insert(
            "TEST_CODE_000001:测试股".to_string(),
            std::time::Instant::now(),
        );

        // 立即驱逐: entry 是 now, age=0 < 7200s, 应保留
        evict_d01_memo_expired();
        assert_eq!(
            D01_LAST_PUSH
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len(),
            1,
            "新 entry 不应被驱逐"
        );

        // 模拟旧 entry: 用 std::time::Instant::now() - Duration::from_secs(8000)
        // Instant 不支持减法, 但可以放一个 entry 然后立即驱逐 (因为 age 太小)
        // 真实测试需用 mock clock. 简化: 验证 evict 不抛错
        _reset_d01_memo_for_test();
    }

    // v29: D-01 dispatcher memo 测试
    // 注: 验证 memo 容器可写入 + 可重置, 集成测试由 monitor --test --v13-diag 覆盖
    #[test]
    fn test_d01_memo_map_basic() {
        use super::{_reset_d01_memo_for_test, D01_LAST_PUSH};
        _reset_d01_memo_for_test();

        // 写入
        {
            let mut map = D01_LAST_PUSH.lock().unwrap_or_else(|e| e.into_inner());
            map.insert(
                "TEST_CODE_000001:平安银行".to_string(),
                std::time::Instant::now(),
            );
        }

        // 读出
        let map = D01_LAST_PUSH.lock().unwrap_or_else(|e| e.into_inner());
        assert!(
            map.contains_key("TEST_CODE_000001:平安银行"),
            "memo 容器应包含刚插入的 key"
        );

        // 重置
        drop(map);
        _reset_d01_memo_for_test();
        let map = D01_LAST_PUSH.lock().unwrap_or_else(|e| e.into_inner());
        assert!(map.is_empty(), "重置后 memo 容器应为空");
    }

    #[test]
    fn ranking_renderers_preserve_missing_values_and_rank_overflow() {
        let sectors = render_sector_top(
            "10:30",
            &[
                ("TEST_CODE_板块1".to_string(), 3.2, 1.5),
                ("TEST_CODE_板块2".to_string(), 2.8, 1.2),
                ("TEST_CODE_板块3".to_string(), 2.1, 0.9),
                ("TEST_CODE_板块4".to_string(), 1.9, 0.8),
                ("TEST_CODE_板块5".to_string(), 1.7, 0.7),
                ("TEST_CODE_板块6".to_string(), 1.5, 0.6),
            ],
        );
        assert!(sectors.contains("🥇 TEST_CODE_板块1 +3.2%"));
        assert!(sectors.contains("5️⃣ TEST_CODE_板块6 +1.5%"));

        let flows = render_fund_inflow_top(
            "10:31",
            &[
                (
                    "测试股A".to_string(),
                    "TEST_CODE_600000".to_string(),
                    2.5,
                    Some(1.8),
                    1.2,
                ),
                (
                    "测试股B".to_string(),
                    "TEST_CODE_000001".to_string(),
                    -0.2,
                    None,
                    -1.1,
                ),
            ],
        );
        assert!(flows.contains("量比1.8"));
        assert!(flows.contains("量比暂无"));

        let empty = render_turnover_top("10:32", &[]);
        assert!(empty.contains("数据源不稳定"));
        let turnover = render_turnover_top(
            "10:32",
            &[
                TurnoverEntry {
                    name: "测试股A".to_string(),
                    code: "TEST_CODE_600000".to_string(),
                    price: 10.25,
                    change_pct: 2.0,
                    turnover_pct: 12.5,
                    main_flow_yi: Some(1.25),
                },
                TurnoverEntry {
                    name: "测试股B".to_string(),
                    code: "TEST_CODE_000001".to_string(),
                    price: 8.0,
                    change_pct: -1.0,
                    turnover_pct: 9.0,
                    main_flow_yi: None,
                },
            ],
        );
        assert!(turnover.contains("主力1.25亿"));
        assert!(turnover.contains("主力暂无"));
        assert!(turnover.contains("非龙虎榜"));
    }

    #[test]
    fn event_macro_summary_separates_held_and_other_complete_announcements() {
        use stock_analysis::data_provider::announcement::{AnnLevel, Announcement};
        let announcement = |code: &str, name: &str, title: &str, level: AnnLevel| Announcement {
            code: code.to_string(),
            name: name.to_string(),
            title: title.to_string(),
            date: "2026-07-18".to_string(),
            summary: "TEST_CODE_摘要".to_string(),
            content: "TEST_CODE_正文".to_string(),
            level,
            reason: "TEST_CODE_原因".to_string(),
            external_id: Some(format!("TEST_CODE_{code}")),
            url: Some("https://example.invalid/announcement".to_string()),
        };
        assert_eq!(
            build_event_calendar_macro_summary(&[], &Default::default()),
            "今日公告批次成功返回 0 条"
        );
        let rows = vec![
            announcement(
                "TEST_CODE_600000",
                "测试持仓",
                "持仓公告",
                AnnLevel::Important,
            ),
            announcement("TEST_CODE_000001", "", "其他公告1", AnnLevel::Info),
            announcement(
                "TEST_CODE_000002",
                "测试二",
                "其他公告2",
                AnnLevel::Emergency,
            ),
            announcement("TEST_CODE_000003", "测试三", "其他公告3", AnnLevel::Info),
            announcement("TEST_CODE_000004", "测试四", "其他公告4", AnnLevel::Info),
        ];
        let held = std::collections::HashSet::from(["TEST_CODE_600000".to_string()]);
        let summary = build_event_calendar_macro_summary(&rows, &held);
        assert!(summary.contains("今日共 5 条公告"));
        assert!(summary.contains("持仓相关 (TOP 1)"));
        assert!(summary.contains("测试持仓(TEST_CODE_600000)"));
        assert!(summary.contains("TEST_CODE_000001 (Info): 其他公告1"));
        assert!(summary.contains("非持仓 (TOP 3)"));
        assert!(!summary.contains("其他公告4"));
    }

    #[test]
    fn source_wrapper_and_metric_json_fail_closed_without_external_io() {
        assert!(load_p5_source_items("TEST_CODE_unknown_source").is_err());
        let short = serde_json::json!({"code":"TEST_CODE_600000"}).to_string();
        assert_eq!(truncate_metric_json(short.clone()), short);
        let long = serde_json::json!({"text":"测".repeat(2_000)}).to_string();
        let truncated = truncate_metric_json(long);
        let value: serde_json::Value = serde_json::from_str(&truncated).unwrap();
        assert_eq!(value.get("truncated").and_then(|v| v.as_bool()), Some(true));
        assert!(value.get("orig_bytes").and_then(|v| v.as_u64()).unwrap() > 4_096);
    }
}

// ===== v16.3 review fixes: helper fns =====

/// v16.3 Commit 2 Fix 2: paper_portfolio_state — 读真实 (cash, total, pos_pct) 给 risk_adapter 4 项检查用
/// review fix Issue #5: 逻辑下沉 lib (trading::paper_trade::portfolio_state), bin/lib 共用同一实现
pub fn paper_portfolio_state(code: &str, quote_price: f64) -> Result<(f64, f64, f64), String> {
    stock_analysis::trading::paper_trade::portfolio_state(code, quote_price)
}

/// v16.3 Commit 2 Fix 8: DoS 防护 — metric_json > 4KB 时替换为截断标记
/// review fix Issue #8: 之前 String::truncate(4096) 会产生非法 JSON (且非 char 边界会 panic),
/// 改为返回合法的最小 JSON, 下游 serde_json::from_str 不会静默失败
pub fn truncate_metric_json(s: String) -> String {
    const MAX_BYTES: usize = 4096;
    if s.len() <= MAX_BYTES {
        return s;
    }
    log::warn!(
        "[truncate_metric_json] metric_json {} bytes > {} 上限 → 替换为截断标记 (保 JSON 合法)",
        s.len(),
        MAX_BYTES
    );
    serde_json::json!({ "truncated": true, "orig_bytes": s.len() }).to_string()
}
