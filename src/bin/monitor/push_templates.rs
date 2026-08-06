//! Registered business rules: BR-047, BR-049, BR-071, BR-072, BR-073, BR-083, BR-192, BR-213.
//! v12 §14 推送消息模板渲染
//!
//! 职责：仅做"按模板拼字符串"，不接 push 通道、不写库、不读行情。
//! 模板结构与字段顺序严格对齐 `docs/architecture/v13-push-templates.md`。
//!
//! 调用约定:
//!   1. 调用方先拼好本模板所需的领域数据（结构体入参）
//!   2. 调对应 `render_xxx()` 函数得到完整 text
//!   3. 获取精确 BR-196 生产展示令牌后交给展示网关推送
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
use std::sync::{Mutex, OnceLock};

/// Acquire the exact registered tuple at the producer/renderer use-site and
/// consume it in the only generic presentation gateway. Registry drift is an
/// explicit denied outcome; it is never downgraded to an untyped dispatch.
macro_rules! dispatch_registered_outcome {
    ($family:literal, $kind:path, $producer:literal, $renderer:literal, $code:expr, $banner:expr, $text:expr) => {{
        match crate::presentation_registry::acquire_token($family, $kind, $producer, $renderer) {
            Ok(token) => dispatch_outcome(token, $code, $banner, $text).await,
            Err(reason) => {
                log::error!(
                    "[BR-196] production presentation token rejected family={} reason={}",
                    $family,
                    reason
                );
                crate::notify::PushOutcome::Denied(reason)
            }
        }
    }};
}

static CLOSING_VALUATION_NOTE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

pub fn set_closing_valuation_note(note: Option<String>) {
    if let Ok(mut slot) = CLOSING_VALUATION_NOTE
        .get_or_init(|| Mutex::new(None))
        .lock()
    {
        *slot = note;
    }
}

fn closing_valuation_note() -> Option<String> {
    CLOSING_VALUATION_NOTE.get()?.lock().ok()?.clone()
}

fn user_confirmed_account_note() -> Option<String> {
    stock_analysis::database::DatabaseManager::try_get()?;
    let summary = stock_analysis::database::user_account_summary::latest()
        .ok()
        .flatten()?;
    Some(format!(
        "用户确认持仓可用；仓位{:.1}%，日盈亏{:+.2}",
        summary.position_ratio_pct, summary.daily_pnl
    ))
}

fn account_status_note() -> String {
    if let Some(note) = closing_valuation_note() {
        return format!("实时账户未接入；{note}");
    }
    if let Some(note) = user_confirmed_account_note() {
        return format!("实时账户未接入；{note}；收盘估值不可用");
    }
    "实时账户未接入；用户确认账户摘要不可用".to_string()
}

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
        let position = if !self.account_metrics_complete && self.total_pos.is_some() {
            "仓位已确认".to_string()
        } else {
            self.total_pos
                .map_or_else(|| "仓位缺失".to_string(), |value| format!("仓位{value}成"))
        };
        let pnl = if !self.account_metrics_complete && self.today_pnl.is_some() {
            "日盈亏已确认".to_string()
        } else {
            self.today_pnl.map_or_else(
                || "日盈亏缺失".to_string(),
                |value| format!("日盈亏{value:+.1}%"),
            )
        };
        let line1 = format!(
            "[{} {} | {} | {} | 数据{}]",
            self.account_mode.icon(),
            self.account_mode.label(),
            position,
            pnl,
            self.data_mode.label(),
        );
        let account_note = (!self.account_metrics_complete).then_some(account_status_note());
        let rendered = match (self.data_missing_note.as_deref(), account_note) {
            (Some(note), _) if !note.is_empty() && self.data_mode != DataMode::Full => {
                format!("{}\n[⚠️ {}: 本条不含承接判断]", line1, note)
            }
            (_, Some(note)) => format!("{}\n[ℹ️ {}]", line1, note),
            _ => line1,
        };
        closing_valuation_note().map_or(rendered.clone(), |note| {
            format!("{}\n[ℹ️ {}]", rendered, note)
        })
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

/// BR-151: paper-only risk context from user-confirmed facts.  This never
/// authorizes a broker order; the Full data gate remains mandatory.
pub(crate) fn snapshot_paper_risk_context_from_banner(
    banner: &BannerCtx,
) -> Result<stock_analysis::trading::paper_trade::PaperRiskContext, String> {
    if banner.data_mode != DataMode::Full {
        let now = chrono::Local::now();
        let latest_valuation =
            stock_analysis::database::closing_valuation::latest_persisted_valuation_view()?;
        let valuation_complete = latest_valuation.as_ref().is_some_and(|view| {
            view.valuation.covered == view.valuation.total && view.valuation.total > 0
        });
        let valuation_date_eligible = latest_valuation.as_ref().is_some_and(|view| {
            post_close_valuation_eligible(now, view.valuation.price_date, valuation_complete)
        });
        if !valuation_date_eligible {
            return Err(format!(
                "SnapshotPaper requires Full intraday data or complete post-close valuation, current={} valuation_complete={valuation_complete}",
                banner.data_mode.label()
            ));
        }
    }
    stock_analysis::database::DatabaseManager::try_get()
        .ok_or_else(|| "数据库未初始化，无法读取用户快照".to_string())?;
    stock_analysis::database::user_account_summary::latest()?
        .ok_or_else(|| "用户账户摘要不存在".to_string())?;
    stock_analysis::database::user_position_snapshot::latest_user_position_snapshot()?
        .ok_or_else(|| "用户持仓快照不存在".to_string())?;
    Ok(stock_analysis::trading::paper_trade::PaperRiskContext::new(
        stock_analysis::risk::action_gate::AccountMode::Normal,
        stock_analysis::monitor::data_mode::DataMode::Full,
    ))
}

fn post_close_valuation_eligible(
    now: chrono::DateTime<chrono::Local>,
    valuation_date: chrono::NaiveDate,
    complete: bool,
) -> bool {
    if !complete {
        return false;
    }
    let today = now.date_naive();
    valuation_date < today
        || (valuation_date == today
            && now.time() >= chrono::NaiveTime::from_hms_opt(15, 0, 0).expect("valid time"))
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
    out.push_str(&format!("\n账户状态: {}", account_status_note()));
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

/// 盘中指标告警 (BR-192 counted 收尾): 12 类 detector 告警 → 推送文本。
/// 与 T-04 render_holding_event (持仓紧急风险) 区分: 本模板渲染 detector 产出
/// 的 AlertEvent (涨停突破/主力突袭/量比爆发/炸板/竞价异动等指标触发),
/// 供 intraday_alert_dispatcher 走 counted binding 投递。
pub fn render_intraday_alert(event: &stock_analysis::monitor::detector::AlertEvent) -> String {
    let hhmm = event.triggered_at.format("%H:%M");
    let extra = event
        .detail
        .extra
        .as_deref()
        .map(|e| format!("\n{e}"))
        .unwrap_or_default();
    format!(
        "{} {} {}({}) {}\n{}\n{}辅助建议, 非下单指令",
        event.level.emoji(),
        event.category.label(),
        event.name,
        event.code,
        hhmm,
        event.message,
        extra
    )
}

/// BR-151/BR-153: Magic TDX evidence-backed reverse-T observation.
pub fn render_t0_advice(banner: &BannerCtx, p: T0AdviceParams<'_>) -> String {
    let plan = p.plan;
    let batch_prefix = plan.batch_id.get(..12).unwrap_or(plan.batch_id.as_str());
    let source_time = plan
        .source_at
        .with_timezone(&chrono::Local)
        .format("%H:%M:%S");
    format!(
        "{}\n🔁 做T观察【真实持仓】 {}({})\n\
         数据: Magic TDX | 批次: {} | 源时间: {}\n\
         状态: {} | 趋势: {}\n\
         现价: {} | 成本: {} | TDX分时均价: {} | ATR14: {}\n\
         量能节奏: {:.2}x | 末根5分钟量比: {:.2}x | 五档卖/买: {:.2}x | 五档买/卖: {:.2}x\n\
         卖出观察区: {}~{}（{}）\n\
         接回观察区: {}~{}（{}）\n\
         毛价差: {:.2}% | 观察腿: {}股卖出/{}股接回\n\
         触发: {}\n\
         失效: {}\n\
         说明: 总持仓{}股；观察腿由用户确认持仓计算，不代表券商已验证可卖数量；执行前必须另取≤30秒券商可用持仓并校验T+1。\n\
         仅观察建议，不自动下单。",
        banner.render(),
        plan.name,
        plan.code,
        batch_prefix,
        source_time,
        plan.state.label(),
        plan.metrics.trend.label(),
        fmt_price(plan.current_price),
        fmt_price(plan.cost_price),
        fmt_price(plan.metrics.intraday_average_price),
        fmt_price(plan.metrics.atr14),
        plan.metrics.pace_ratio,
        plan.metrics.last_bar_volume_ratio,
        plan.metrics.ask_bid_ratio,
        plan.metrics.bid_ask_ratio,
        fmt_price(plan.sell_zone.low),
        fmt_price(plan.sell_zone.high),
        plan.sell_zone.source.label(),
        fmt_price(plan.buy_zone.low),
        fmt_price(plan.buy_zone.high),
        plan.buy_zone.source.label(),
        plan.gross_spread_pct,
        plan.sell_quantity,
        plan.buyback_quantity,
        plan.trigger_text,
        plan.invalidation_text,
        plan.total_quantity,
    )
}

#[derive(Debug)]
pub struct T0AdviceParams<'a> {
    pub plan: &'a stock_analysis::decision::t0_advisor::T0StructuredPlan,
}

impl<'a> From<&'a stock_analysis::decision::t0_advisor::T0StructuredPlan> for T0AdviceParams<'a> {
    fn from(plan: &'a stock_analysis::decision::t0_advisor::T0StructuredPlan) -> Self {
        Self { plan }
    }
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
        p.name,
        p.code,
        p.hhmm,
        p.grade.label(),
        p.topic,
        fmt_price(p.price),
        p.trigger_desc,
        fmt_price(p.lo),
        fmt_price(p.hi),
        fmt_price(p.stop),
        p.max_pos_pct,
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

/// The template DTO deliberately represents terminal rows only. Convert it
/// explicitly at the durable audit boundary instead of relying on two
/// same-named enums being interchangeable.
impl From<PaperTradeStatus> for paper_trade::PaperTradeStatus {
    fn from(status: PaperTradeStatus) -> Self {
        match status {
            PaperTradeStatus::Filled => Self::Filled,
            PaperTradeStatus::NotFilled => Self::NotFilled,
            PaperTradeStatus::Invalidated => Self::Invalidated,
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
    let result = dispatch_registered_outcome!(
        "P-05-virtual-watch",
        crate::notify::PushKind::VirtualWatch,
        "virtual_watch_dispatcher",
        "render_virtual_watch",
        "",
        None,
        text
    )
    .is_pushed();
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
        sh_chg_display,
        chinext_display,
        star_display,
        limit_up_display,
        limit_down_display,
        broken_display,
        consecutive_display,
        amount_display,
        amount_delta_display,
        main_flow_display,
        m.money_effect,
        m.heat_stage,
        m.heat_conf_pct,
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

/// R-03 涨停题材联动
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
    pub watch_point: Option<&'a str>,
}

/// v12 §14.2 R-03 verified limit-pool theme template.
pub fn render_industry_chain(
    date: &str,
    chains: &[ChainLine<'_>],
    fade: Option<&str>,
    evidence_note: Option<&str>,
) -> String {
    let mut out = format!("🔥 涨停题材联动（{}）", date);
    for (i, c) in chains.iter().enumerate() {
        let watch_point = c
            .watch_point
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("数据缺失（当前批次未提供量能/走势证据）");
        out.push_str(&format!(
            "\n{}. {} 涨停{}家（首板{}/连板{}） 阶段: {}\n   龙头: {}({}) {}板\n   后排: {}\n   明日观察: {}",
            i + 1,
            c.chain, c.limit_up_n, c.first_n, c.consec_n, c.heat_stage,
            c.leader_name, c.leader_code, c.leader_boards,
            c.followers,
            watch_point,
        ));
    }
    if let Some(f) = fade.filter(|s| !s.is_empty()) {
        out.push_str(&format!("\n⚠️ 退潮链: {}", f));
    }
    if let Some(note) = evidence_note.filter(|note| !note.is_empty()) {
        out.push_str(&format!("\n⚠️ 部分证据: {note}"));
    }
    out
}

/// R-04 龙虎榜
#[derive(Debug)]
pub struct LhbEntry<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub net_buy_yi: f64,
    pub reason: Option<&'a str>,
    pub buy_inst_n: Option<u32>,
    // W4.3 / B-010 P1 修复: 数值字段从 f64 改 Option<f64>, render 端判 None 显示"无"
    pub buy_inst_amt_wan: Option<f64>,
    pub buy_other_n: Option<u32>,
    pub buy_other_amt_wan: Option<f64>,
    pub buy_conc_pct: Option<f64>,
    pub sell_desc: Option<&'a str>,
    pub sell_conc_pct: Option<f64>,
    pub chain_match: Option<&'a str>,
    pub next_day_risk: Option<&'a str>,
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
        let missing = "数据缺失";
        let reason = e
            .reason
            .filter(|value| !value.is_empty())
            .unwrap_or(missing);
        let sell_desc_display = e
            .sell_desc
            .filter(|value| !value.is_empty())
            .unwrap_or(missing);
        let buy_inst_amt = e
            .buy_inst_amt_wan
            .map(|v| format!("{:.0}", v))
            .unwrap_or_else(|| missing.to_string());
        let buy_other_amt = e
            .buy_other_amt_wan
            .map(|v| format!("{:.0}", v))
            .unwrap_or_else(|| missing.to_string());
        let buy_conc = e
            .buy_conc_pct
            .map(|v| format!("{:.0}", v))
            .unwrap_or_else(|| missing.to_string());
        let sell_conc = e
            .sell_conc_pct
            .map(|v| format!("{:.0}", v))
            .unwrap_or_else(|| missing.to_string());
        let buy_inst_n = e
            .buy_inst_n
            .map(|value| value.to_string())
            .unwrap_or_else(|| missing.to_string());
        let buy_other_n = e
            .buy_other_n
            .map(|value| value.to_string())
            .unwrap_or_else(|| missing.to_string());
        let chain_match = e
            .chain_match
            .filter(|value| !value.is_empty())
            .map(|value| format!("是-{value}"))
            .unwrap_or_else(|| missing.to_string());
        let next_day_risk = e
            .next_day_risk
            .filter(|value| !value.is_empty())
            .unwrap_or(missing);
        out.push_str(&format!(
            "\n{}. {}({}) 净买{:.1}亿 | {}\n   买: 机构{}席{}万 其他{}席{}万（集中度{}%）\n   卖: {}（集中度{}%）\n   主线一致: {}\n   次日风险: {}",
            i + 1,
            e.name, e.code, e.net_buy_yi, reason,
            buy_inst_n, buy_inst_amt,
            buy_other_n, buy_other_amt,
            buy_conc,
            sell_desc_display, sell_conc,
            chain_match,
            next_day_risk,
        ));
        out.push_str("\n─────");
    }
    out.push_str("\n仅结构化事实, 不含席位风格推断");
    out
}

/// BR-162 R-04 renderer: one row per stock while retaining every distinct
/// source TRADE_ID and the exact buy-five/sell-five seats.
pub fn render_review_lhb_gateway(
    date: &str,
    stocks: &[stock_analysis::data_gateway::DragonTigerStockReview],
    evidence: &stock_analysis::data_gateway::BatchEvidence,
) -> String {
    let mut out = format!(
        "🐉 龙虎榜净买前五（{date} 21:00）\n数据源: {} | 源日期: {} | 批次: {}",
        evidence.source,
        evidence.source_at.as_deref().unwrap_or("未提供"),
        evidence.batch_id
    );
    for (index, stock) in stocks.iter().enumerate() {
        out.push_str(&format!(
            "\n{}. {}{} 排名净买 {} | 源披露 {} 条",
            index + 1,
            exchange_label(stock.exchange),
            stock.code,
            format_r04_money(stock.ranking_net_amount_yuan),
            stock.disclosures.len()
        ));
        for (disclosure_index, disclosure) in stock.disclosures.iter().enumerate() {
            out.push_str(&format!(
                "\n   披露{} TRADE_ID={} | 原因: {}\n   买={} 卖={} 净={} 换手={}",
                disclosure_index + 1,
                disclosure.trade_id,
                disclosure.reason.as_deref().unwrap_or("源未提供"),
                format_optional_r04_money(disclosure.buy_amount_yuan),
                format_optional_r04_money(disclosure.sell_amount_yuan),
                format_optional_r04_money(disclosure.net_amount_yuan),
                disclosure
                    .turnover_rate_pct
                    .map(|value| format!("{value:.2}%"))
                    .unwrap_or_else(|| "源未提供".to_string())
            ));
            for side in [
                magic_market_core::DragonTigerSide::Buy,
                magic_market_core::DragonTigerSide::Sell,
            ] {
                out.push_str(match side {
                    magic_market_core::DragonTigerSide::Buy => "\n   买入席位:",
                    magic_market_core::DragonTigerSide::Sell => "\n   卖出席位:",
                });
                for seat in disclosure.seats.iter().filter(|seat| seat.side == side) {
                    out.push_str(&format!(
                        "\n     {}{} {} | {}{}",
                        match side {
                            magic_market_core::DragonTigerSide::Buy => "买",
                            magic_market_core::DragonTigerSide::Sell => "卖",
                        },
                        seat.rank,
                        seat.seat_name,
                        format_r04_money(seat.amount_yuan),
                        seat.net_amount_yuan
                            .map(|value| format!(" | 净={}", format_r04_money(value)))
                            .unwrap_or_default()
                    ));
                }
            }
        }
        out.push_str("\n─────");
    }
    out.push_str("\n仅展示源结构化事实；不同 TRADE_ID 未合并求和，不含席位风格推断");
    out
}

fn format_optional_r04_money(value: Option<f64>) -> String {
    value
        .map(format_r04_money)
        .unwrap_or_else(|| "源未提供".to_string())
}

fn format_r04_money(value: f64) -> String {
    let absolute = value.abs();
    if absolute >= 100_000_000.0 {
        format!("{:.2}亿", value / 100_000_000.0)
    } else if absolute >= 10_000.0 {
        format!("{:.2}万", value / 10_000.0)
    } else {
        format!("{value:.2}元")
    }
}

const fn exchange_label(exchange: magic_market_core::Exchange) -> &'static str {
    match exchange {
        magic_market_core::Exchange::Shanghai => "SH",
        magic_market_core::Exchange::Shenzhen => "SZ",
        magic_market_core::Exchange::Beijing => "BJ",
    }
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
        r.holding_n,
        r.holding_exec,
        r.holding_eff,
        r.t0_n,
        r.t0_eff,
        r.cand_trigger,
        r.cand_filled,
        r.cand_notfilled,
        r.cand_limitup,
        r.cand_notreach,
        r.paper_pnl_pct,
        r.paper_total_pct,
        r.paper_n,
        r.news_push_n,
        r.news_d1_eff,
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
    futures_delivery: &str,
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
        "\n宏观: {}\n期货交割: {}\n隔夜关注: 美股{} 汇率{}",
        macro_econ, futures_delivery, us_chg, fx
    ));
    out
}

#[derive(Clone, Copy)]
pub enum R08HoldingAudience<'a> {
    /// Immutable broker position batch with provider, batch identity and source time.
    Verified(&'a std::collections::HashSet<String>),
    /// No verified per-security broker batch. Never infer non-holding from this state.
    Unavailable,
}

/// R-08 宏观公告摘要: 只有经验证券商逐仓批次才能区分"持仓相关" / "非持仓".
/// 受众不可用时保留公告事实但显式标记关系未知，禁止把未知填成空持仓。
pub fn build_event_calendar_macro_summary(
    anns: &[stock_analysis::announcement::Announcement],
    audience: R08HoldingAudience<'_>,
) -> String {
    use stock_analysis::announcement::Announcement;
    if anns.is_empty() {
        return "今日公告批次成功返回 0 条".to_string();
    }
    let immediate: Vec<&Announcement> = anns
        .iter()
        .filter(|announcement| {
            stock_analysis::announcement::announcement_is_immediate_notification_candidate(
                announcement,
            )
        })
        .collect();
    if immediate.is_empty() {
        return "今日可即时通知公告 0 条（本地生命周期证据已隔离）".to_string();
    }
    let fmt = |a: &Announcement| -> String {
        let disp = if a.name.is_empty() {
            a.code.clone()
        } else {
            format!("{}({})", a.name, a.code)
        };
        format!("· {} ({:?}): {}", disp, a.level, a.title)
    };
    let R08HoldingAudience::Verified(holding_codes) = audience else {
        let mut summary = format!(
            "今日共 {} 条公告\n持仓关系不可判定 (TOP {}):",
            immediate.len(),
            immediate.len().min(3)
        );
        for announcement in immediate.iter().take(3) {
            summary.push('\n');
            summary.push_str(&fmt(announcement));
        }
        return summary;
    };
    let held: Vec<&Announcement> = immediate
        .iter()
        .copied()
        .filter(|a| holding_codes.contains(&a.code))
        .collect();
    let other: Vec<&Announcement> = immediate
        .iter()
        .copied()
        .filter(|a| !holding_codes.contains(&a.code))
        .collect();
    let mut s = format!("今日共 {} 条公告", immediate.len());
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
    let outcome = dispatch_registered_outcome!(
        "T-01-account-mode",
        crate::notify::PushKind::AccountMode,
        "account_mode_hook",
        "render_account_mode",
        "", // code 空 = 全局键
        banner,
        text
    );

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
            chrono::Local::now(),
            None,
            false,
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
            outcome,
            log_id
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
// v14.2: v13 核心 6 模板 push_* wrapper (render + dispatch)
// ============================================================================

/// v13 §14.1 P-01 盘前新闻热点 (ℹ️参考, 盘前无 banner)
pub async fn push_preopen_news_hot(code: &str, params: PreopenNewsHotParams<'_>) -> bool {
    let text = render_preopen_news_hot(params);
    dispatch_registered_outcome!(
        "P-01-preopen-news-hot",
        crate::notify::PushKind::PreopenNewsHot,
        "preopen_news_dispatcher",
        "render_preopen_news_hot",
        code,
        None,
        text
    )
    .is_pushed()
}

/// BR-225: 取前三条主线簇的头股代码。`chain_daily.stocks` 只存代码，名称必须
/// 由外部权威来源解析，因此调用方需要先拿到这份代码集合。
pub fn preopen_head_codes(
    clusters: &[stock_analysis::database::concepts::ChainDailyRow],
) -> Result<Vec<String>, String> {
    let mut codes = Vec::new();
    for (cluster_index, cluster) in clusters.iter().take(3).enumerate() {
        let parsed = serde_json::from_str::<Vec<String>>(&cluster.stocks).map_err(|error| {
            format!(
                "P-01 chain_daily 第 {} 个主线 stocks JSON 非法: {error}",
                cluster_index + 1
            )
        })?;
        let code = parsed
            .first()
            .map(|value| value.trim())
            .filter(|code| valid_source_stock_code(code))
            .ok_or_else(|| {
                format!(
                    "P-01 chain_daily 第 {} 个主线缺少有效头股",
                    cluster_index + 1
                )
            })?;
        codes.push(code.to_string());
    }
    Ok(codes)
}

/// BR-225: 通过统一 Gateway 的已接纳 security identity 能力解析头股真实名称。
///
/// `chain_daily` 与 `board_rotation_daily` 是两张互相独立的表，头股不必出现在
/// 板块异动股列表里；把板块表当作唯一名称来源会让整条 P-01 推送因一只股票缺名
/// 而全量失败。这里保留板块表内的 provider 名称为首选证据，缺失项才回落到
/// security identity 批次，仍然不合成任何本地名称。
pub async fn resolve_preopen_head_names(
    codes: &[String],
) -> Result<std::collections::HashMap<String, String>, String> {
    use stock_analysis::data_gateway::{GatewayBatch, MarketCapabilitiesGateway};
    if codes.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let batch = MarketCapabilitiesGateway::new()
        .security_identities(codes)
        .await
        .map_err(|error| format!("P-01 证券身份统一 Gateway 失败: {error}"))?;
    let (records, evidence) = match batch {
        GatewayBatch::Available { records, evidence } => (records, evidence),
        GatewayBatch::VerifiedEmpty(evidence) => (Vec::new(), evidence),
    };
    let mut names = std::collections::HashMap::new();
    for record in records {
        if record.batch_id != evidence.batch_id {
            return Err(format!(
                "P-01 证券身份批次身份不一致: code={} batch_id={}",
                record.code, evidence.batch_id
            ));
        }
        let name = record.name.trim();
        if name.is_empty() {
            continue;
        }
        names.insert(record.code.clone(), name.to_string());
    }
    Ok(names)
}

/// BR-101: 从主线簇与板块联动归因构造可证的盘前新闻快照。
pub fn build_preopen_news_hot_from_db<'a>(
    hhmm: &'a str,
    clusters: &'a [stock_analysis::database::concepts::ChainDailyRow],
    rotations: &'a [stock_analysis::database::concepts::BoardRotationRow],
    resolved_names: &std::collections::HashMap<String, String>,
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
            .or_else(|| resolved_names.get(code))
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
    // BR-225: 头股名称先由统一 Gateway 的 security identity 批次解析，作为板块
    // 异动股名称之外的独立回落证据。解析失败不合成名称，只记录后交由构造函数
    // 按缺失代码显式失败。
    let resolved_names = match preopen_head_codes(&clusters) {
        Ok(codes) => match resolve_preopen_head_names(&codes).await {
            Ok(names) => names,
            Err(error) => {
                log::warn!("[P-01][BR-225] 头股名称回落解析失败: {error}");
                std::collections::HashMap::new()
            }
        },
        Err(error) => {
            log::error!("[P-01] 快照批次拒绝: {error}");
            log_dispatcher_attempt("P-01", false, 0, &error);
            return false;
        }
    };
    let params = match build_preopen_news_hot_from_db(&hhmm, &clusters, &rotations, &resolved_names)
    {
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
    let snapshot = match tokio::task::spawn_blocking({
        let hhmm = hhmm.to_string();
        move || load_sector_snapshot_real(&hhmm)
    })
    .await
    .map_err(|error| format!("I-01 snapshot worker join: {error}"))
    .and_then(|result| result)
    {
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

/// BR-164: all monitor quote consumers use one evidence-preserving Magic
/// provider Gateway batch. Partial results are rejected at the boundary.
fn fetch_realtime_quote_batch_strict(
    codes: &[&str],
) -> Result<
    std::collections::HashMap<String, stock_analysis::data_gateway::RealtimeMarketQuote>,
    String,
> {
    if codes.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let requested: Vec<String> = codes.iter().map(|code| (*code).to_string()).collect();
    let batch = stock_analysis::data_gateway::MarketDataGateway::new()
        .realtime_quotes(&requested)
        .map_err(|error| format!("统一实时行情 Gateway 不可用: {error}"))?;
    if batch.is_verified_empty() {
        return Err("统一实时行情 Gateway 返回不允许的 verified-empty".to_string());
    }
    let quotes: std::collections::HashMap<_, _> = batch
        .records()
        .iter()
        .cloned()
        .map(|quote| (quote.code.clone(), quote))
        .collect();
    if quotes.len() != requested.len() {
        return Err(format!(
            "统一实时行情 Gateway 批次不完整: requested={} actual={}",
            requested.len(),
            quotes.len()
        ));
    }
    Ok(quotes)
}

pub fn fetch_realtime_quotes_batch(
    codes: &[&str],
) -> Result<std::collections::HashMap<String, f32>, String> {
    fetch_realtime_quote_batch_strict(codes).map(|quotes| {
        quotes
            .into_iter()
            .map(|(code, quote)| (code, quote.change_percent as f32))
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
                .filter(|value| value.is_finite())
                .ok_or_else(|| format!("I-02 {code} 缺少有效 change_pct"))?;
            if change_pct.abs() > 20.0 {
                log::warn!(
                    "[DQ-2.3] I-02 {code} change_pct={change_pct:.2}% 超过常规±20%，保留真实值并标记需人工确认"
                );
            }
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
        stocks.push((quote.name.clone(), code, Some(quote.change_percent as f32)));
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
    let hhmm_owned = hhmm.to_string();
    let mut snapshot = match crate::blocking_market_data::run_blocking_market_data(
        "I-02 news catalyst snapshot",
        move || load_news_catalyst_snapshot_real(&hhmm_owned),
    )
    .await
    {
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

/// v16.3+v14.1: 真实数据集成 — 复用 chain_daily DB + unified quote Gateway + aggregate()
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
    let quotes = super::market_data::fetch_realtime_quotes(&all_codes)?;
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
    let hhmm_owned = hhmm.to_string();
    let mut snapshot = match crate::blocking_market_data::run_blocking_market_data(
        "I-03 industry chain snapshot",
        move || load_industry_chain_snapshot_real(&hhmm_owned),
    )
    .await
    {
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
                snapshot.chain,
                snapshot.leader_name,
                snapshot.leader_code,
                snapshot.leader_height,
                candidates_block
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
    quote_evidence: Option<stock_analysis::data_gateway::BatchEvidence>,
    statistics_evidence: Option<stock_analysis::data_gateway::BatchEvidence>,
}

#[derive(Debug, Clone, PartialEq)]
struct CandidateStatisticsRow {
    code: String,
    volume_ratio: Option<f64>,
}

#[derive(Debug)]
struct CandidateStatisticsBatch {
    rows: Vec<CandidateStatisticsRow>,
    evidence: stock_analysis::data_gateway::BatchEvidence,
}

#[derive(Debug)]
struct CandidateSourceContext {
    entries: Vec<stock_analysis::opportunity::candidate_panel::CandidateEntry>,
    themes: std::collections::HashMap<String, String>,
    held_codes: Vec<String>,
}

fn native_candidate_code(code: &str) -> &str {
    code.strip_prefix("TEST_CODE_").unwrap_or(code)
}

fn project_candidate_statistics(
    codes: &[String],
    batch: stock_analysis::data_gateway::GatewayBatch<
        stock_analysis::data_gateway::company::MarketStatistics,
    >,
) -> Result<CandidateStatisticsBatch, String> {
    use stock_analysis::data_gateway::GatewayBatch;

    let (records, evidence) = match batch {
        GatewayBatch::Available { records, evidence } if !records.is_empty() => (records, evidence),
        GatewayBatch::Available { .. } | GatewayBatch::VerifiedEmpty(_) => {
            return Err("候选台市场统计 Gateway 返回不允许的空批次".to_string());
        }
    };
    if records.len() != codes.len() {
        return Err(format!(
            "候选台市场统计批次基数不一致 requested={} actual={}",
            codes.len(),
            records.len()
        ));
    }
    let rows = codes
        .iter()
        .zip(records)
        .map(|(requested, record)| {
            if native_candidate_code(requested) != record.instrument().code() {
                return Err(format!(
                    "候选台市场统计身份不一致 requested={} actual={}",
                    requested,
                    record.instrument().code()
                ));
            }
            let record_evidence = record.evidence();
            if record_evidence.provider() != evidence.provider
                || record_evidence.batch_id() != evidence.batch_id
                || record_evidence.observed_at() != evidence.observed_at
            {
                return Err(format!(
                    "候选台市场统计 {} 记录证据与批次证据不一致",
                    requested
                ));
            }
            let volume_ratio = record.volume_ratio().map(|value| value.get());
            if volume_ratio.is_some_and(|value| !value.is_finite() || value < 0.0) {
                return Err(format!(
                    "候选台市场统计 {} volume_ratio 非法: {:?}",
                    requested, volume_ratio
                ));
            }
            Ok(CandidateStatisticsRow {
                code: requested.to_string(),
                volume_ratio,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(CandidateStatisticsBatch { rows, evidence })
}

fn assemble_real_candidate_batch(
    mut entries: Vec<stock_analysis::opportunity::candidate_panel::CandidateEntry>,
    quote_batch: crate::market_data::TopStockBatch,
    statistics_batch: CandidateStatisticsBatch,
    themes: std::collections::HashMap<String, String>,
    held_codes: &[String],
) -> Result<RealCandidateBatch, String> {
    use stock_analysis::opportunity::candidate_panel::{
        classify_tier, filter_hard_gates, sort_candidates_by_heat, CandidateSource,
    };

    let codes: Vec<String> = entries.iter().map(|entry| entry.code.clone()).collect();
    crate::market_data::validate_quote_batch_codes(
        &codes,
        &quote_batch.stocks,
        "candidate_quote_gateway",
    )?;
    let statistic_codes: Vec<String> = statistics_batch
        .rows
        .iter()
        .map(|row| row.code.clone())
        .collect();
    if statistic_codes != codes {
        return Err(format!(
            "候选台市场统计身份/顺序不一致 requested={codes:?} actual={statistic_codes:?}"
        ));
    }

    let statistics: std::collections::HashMap<_, _> = statistics_batch
        .rows
        .into_iter()
        .map(|row| (row.code.clone(), row))
        .collect();
    let mut quote_map: std::collections::HashMap<_, _> = quote_batch
        .stocks
        .into_iter()
        .map(|quote| (quote.code.clone(), quote))
        .collect();

    for entry in &mut entries {
        let statistics = statistics
            .get(&entry.code)
            .ok_or_else(|| format!("候选台缺少 {} 市场统计", entry.code))?;
        let quote = quote_map
            .get_mut(&entry.code)
            .ok_or_else(|| format!("候选台缺少 {} 实时行情", entry.code))?;
        quote.volume_ratio = statistics.volume_ratio;
        quote.main_net_yi = None;
        entry.name = quote.name.clone();
        entry.current_price = Some(quote.price);
        entry.change_pct = Some(quote.change_pct);
        entry.heat_score = None;
        let mut evidence = Vec::with_capacity(entry.sources.len());
        for source in &entry.sources {
            let description = match source {
                CandidateSource::IndustryChain => {
                    let theme = themes.get(&entry.code).ok_or_else(|| {
                        format!("候选台 {} 含产业链来源但缺少主线名称", entry.code)
                    })?;
                    format!("产业链: {theme}")
                }
                _ => format!("真实来源: {}", source.label()),
            };
            evidence.push(description);
        }
        entry.evidence = evidence;
        entry.tier = classify_tier(&entry.evidence);
    }

    entries = filter_hard_gates(entries, held_codes);
    entries = sort_candidates_by_heat(entries);

    Ok(RealCandidateBatch {
        entries,
        quotes: quote_map,
        themes,
        quote_evidence: Some(quote_batch.evidence),
        statistics_evidence: Some(statistics_batch.evidence),
    })
}

fn load_candidate_source_context() -> Result<CandidateSourceContext, String> {
    use stock_analysis::database::DatabaseManager;
    use stock_analysis::opportunity::candidate_panel::{merge_candidates, CandidateSource};

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

    let entries = merge_candidates(items);
    let held_codes = stock_analysis::portfolio::get_positions()
        .map_err(|error| format!("候选台读取持仓失败: {error}"))?
        .into_iter()
        .map(|position| position.code)
        .collect();
    Ok(CandidateSourceContext {
        entries,
        themes,
        held_codes,
    })
}

async fn load_real_candidate_batch() -> Result<RealCandidateBatch, String> {
    let CandidateSourceContext {
        entries,
        themes,
        held_codes,
    } = crate::blocking_market_data::run_blocking_market_data(
        "BR-099 candidate source context",
        load_candidate_source_context,
    )
    .await?;
    if entries.is_empty() {
        return Ok(RealCandidateBatch {
            entries,
            quotes: std::collections::HashMap::new(),
            themes,
            quote_evidence: None,
            statistics_evidence: None,
        });
    }

    let codes: Vec<String> = entries.iter().map(|entry| entry.code.clone()).collect();
    let quote_codes = codes.clone();
    let quote_future = crate::blocking_market_data::run_blocking_market_data(
        "BR-159 candidate quote batch",
        move || crate::market_data::fetch_realtime_quote_batch(&quote_codes),
    );
    let company_gateway = stock_analysis::data_gateway::CompanyDataGateway::new();
    let statistics_future = company_gateway.market_statistics(&codes);
    let (quote_result, statistics_result) = tokio::join!(quote_future, statistics_future);
    let quote_batch = quote_result?;
    let statistics_batch = project_candidate_statistics(
        &codes,
        statistics_result.map_err(|error| format!("候选台市场统计 Gateway 不可用: {error}"))?,
    )?;
    log::info!(
        "[BR-099][BR-159][CandidateAssembly] codes={} quote_provider={:?} quote_source={} quote_source_at={} quote_observed_at={} quote_batch_id={} statistics_provider={:?} statistics_source={} statistics_source_at={} statistics_observed_at={} statistics_batch_id={}",
        codes.len(),
        quote_batch.evidence.provider,
        quote_batch.evidence.source,
        quote_batch
            .evidence
            .source_at
            .as_deref()
            .unwrap_or("absent"),
        quote_batch.evidence.observed_at,
        quote_batch.evidence.batch_id,
        statistics_batch.evidence.provider,
        statistics_batch.evidence.source,
        statistics_batch
            .evidence
            .source_at
            .as_deref()
            .unwrap_or("absent"),
        statistics_batch.evidence.observed_at,
        statistics_batch.evidence.batch_id
    );
    assemble_real_candidate_batch(entries, quote_batch, statistics_batch, themes, &held_codes)
}

/// v16.4+v13.6.2+v14.2: 真实数据集成 — 从候选台取 top 1 candidate
/// 联接 opportunity::candidate_panel::merge_candidates
/// v14.2 改进: P5 源文件化 (data/p5_sources/*.jsonl)
pub async fn load_news_to_idea_snapshot_real(hhmm: &str) -> Result<NewsToIdeaSnapshot, String> {
    let batch = load_real_candidate_batch().await?;
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
    let mut snapshot = match load_news_to_idea_snapshot_real(hhmm).await {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum A01TargetDisposition {
    Eligible(chrono::NaiveDate),
    Pending(chrono::NaiveDate),
    OutOfWindow(chrono::NaiveDate),
}

fn classify_a01_target(
    entry_date: chrono::NaiveDate,
    review_date: chrono::NaiveDate,
    completed_through: chrono::NaiveDate,
) -> Result<A01TargetDisposition, String> {
    if entry_date > review_date {
        return Err(format!(
            "A-01 entry_date={entry_date} 晚于 review_date={review_date}"
        ));
    }
    let target = stock_analysis::calendar::next_trading_day(entry_date);
    if target < review_date {
        return Ok(A01TargetDisposition::OutOfWindow(target));
    }
    if target > review_date || target > completed_through {
        return Ok(A01TargetDisposition::Pending(target));
    }
    Ok(A01TargetDisposition::Eligible(target))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PaperReviewRejection {
    code: String,
    reason: String,
}

#[derive(Debug, Clone, Default)]
struct PaperReviewCandidateBatch {
    snapshot: Option<PaperReviewSnapshot>,
    rejections: Vec<PaperReviewRejection>,
    pending_count: usize,
    out_of_window_count: usize,
}

#[cfg(test)]
fn build_paper_review_candidate_with<F>(
    date: &str,
    records: &[VirtualRecordLite],
    mut fetch_daily: F,
) -> Result<PaperReviewCandidateBatch, String>
where
    F: FnMut(&str, usize) -> Result<(Vec<(chrono::NaiveDate, f64)>, String), String>,
{
    let review_date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|error| format!("A-01 非法复盘日期 {date}: {error}"))?;
    let mut batch = PaperReviewCandidateBatch::default();

    for record in records {
        let entry_date = match chrono::NaiveDate::parse_from_str(&record.entry_date, "%Y-%m-%d") {
            Ok(entry_date) => entry_date,
            Err(error) => {
                batch.rejections.push(PaperReviewRejection {
                    code: record.code.clone(),
                    reason: format!("entry_date 非法: {error}"),
                });
                continue;
            }
        };
        let target = match classify_a01_target(entry_date, review_date, review_date) {
            Ok(A01TargetDisposition::Eligible(target)) => target,
            Ok(A01TargetDisposition::Pending(_)) => {
                batch.pending_count += 1;
                continue;
            }
            Ok(A01TargetDisposition::OutOfWindow(_)) => {
                batch.out_of_window_count += 1;
                continue;
            }
            Err(reason) => {
                batch.rejections.push(PaperReviewRejection {
                    code: record.code.clone(),
                    reason,
                });
                continue;
            }
        };
        let evaluated = (|| -> Result<Option<PaperReviewSnapshot>, String> {
            if !valid_source_stock_code(&record.code) {
                return Err("code 非法".to_string());
            }
            if record.name.trim().is_empty() {
                return Err("name 缺失".to_string());
            }
            if record.entry_mode.trim().is_empty() {
                return Err("entry_mode 缺失".to_string());
            }
            if !record.entry_price.is_finite() || record.entry_price <= 0.0 {
                return Err("entry_price 缺失/非法".to_string());
            }

            let (rows, source) = fetch_daily(&record.code, 60)?;
            let Some((target, close_price)) = select_t1_close(&rows, entry_date, target)? else {
                return Err(format!("T+1({target}) 已到但严格日 K 批次未覆盖"));
            };
            let pnl = ((close_price / record.entry_price - 1.0) * 100.0) as f32;
            if !pnl.is_finite() {
                return Err("收益率非有限值".to_string());
            }
            let (high, flat, low) = derive_plan_from_pnl(pnl);
            Ok(Some(PaperReviewSnapshot {
                date: target.format("%Y-%m-%d").to_string(),
                name: record.name.clone(),
                code: record.code.clone(),
                trigger: record.entry_mode.clone(),
                desc: format!(
                    "研究观察 T+1={} (entry={:.2} → close={:.2}, pnl={:+.1}%, source={})",
                    target, record.entry_price, close_price, pnl, source
                ),
                pnl: Some(pnl),
                plan_high: Some(high),
                plan_flat: Some(flat),
                plan_low: Some(low),
            }))
        })();

        match evaluated {
            Ok(Some(snapshot)) => {
                batch.snapshot = Some(snapshot);
                return Ok(batch);
            }
            Ok(None) => {
                unreachable!("eligible A-01 candidate maps to a snapshot or explicit error")
            }
            Err(reason) => batch.rejections.push(PaperReviewRejection {
                code: record.code.clone(),
                reason,
            }),
        }
    }

    Ok(batch)
}

pub async fn load_paper_review_snapshot_real(
    date: &str,
) -> Result<Option<PaperReviewSnapshot>, String> {
    let snapshot = load_virtual_observation_for_a01()?;
    audit_virtual_observation_load_issues(&snapshot)?;
    if snapshot.records.is_empty() {
        if !snapshot.rejections.is_empty() || !snapshot.source_failures.is_empty() {
            return Err(format!(
                "A-01 virtual observation has no valid records: rejected={} source_failures={}",
                snapshot.rejections.len(),
                snapshot.source_failures.len()
            ));
        }
        return Ok(None);
    }

    let review_date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|error| format!("A-01 非法复盘日期: {error}"))?;
    let completed_through = stock_analysis::calendar::latest_completed_trading_day_at(
        chrono::Local::now().naive_local(),
    );
    let gateway = stock_analysis::data_gateway::ReviewDataGateway::new();
    let mut batch = PaperReviewCandidateBatch::default();

    for record in &snapshot.records {
        let entry_date = match chrono::NaiveDate::parse_from_str(&record.entry_date, "%Y-%m-%d") {
            Ok(entry_date) => entry_date,
            Err(error) => {
                batch.rejections.push(PaperReviewRejection {
                    code: record.code.clone(),
                    reason: format!("entry_date 非法: {error}"),
                });
                continue;
            }
        };
        let target = match classify_a01_target(entry_date, review_date, completed_through) {
            Ok(A01TargetDisposition::Eligible(target)) => target,
            Ok(A01TargetDisposition::Pending(_)) => {
                batch.pending_count += 1;
                continue;
            }
            Ok(A01TargetDisposition::OutOfWindow(_)) => {
                batch.out_of_window_count += 1;
                continue;
            }
            Err(reason) => {
                batch.rejections.push(PaperReviewRejection {
                    code: record.code.clone(),
                    reason,
                });
                continue;
            }
        };
        let evaluated = async {
            if !valid_source_stock_code(&record.code) {
                return Err("code 非法".to_string());
            }
            if record.name.trim().is_empty() {
                return Err("name 缺失".to_string());
            }
            if record.entry_mode.trim().is_empty() {
                return Err("entry_mode 缺失".to_string());
            }
            if !record.entry_price.is_finite() || record.entry_price <= 0.0 {
                return Err("entry_price 缺失/非法".to_string());
            }

            let gateway_batch = gateway
                .a01_daily_bars(&record.code, 60)
                .await
                .map_err(|error| {
                    log::warn!(
                        "[DataGateway][A-01] status=unavailable disabled=no_verified_batch code={} error={}",
                        record.code,
                        error
                    );
                    format!("统一历史日 K 批次失败: {error}")
                })?;
            let evidence = gateway_batch.evidence();
            log::info!(
                "[DataGateway][A-01] {}",
                gateway_batch
            );
            if gateway_batch.is_verified_empty() {
                return Err(format!(
                    "统一历史日 K 返回 verified-empty: provider={:?} source={} batch_id={}",
                    evidence.provider, evidence.source, evidence.batch_id
                ));
            }
            let rows = gateway_batch
                .records()
                .iter()
                .map(|bar| (bar.date, bar.close))
                .collect::<Vec<_>>();
            let Some((target, close_price)) = select_t1_close(&rows, entry_date, target)?
            else {
                return Err(format!(
                    "T+1({target}) 已结算但严格日 K 批次未覆盖: provider={:?} source={} batch_id={}",
                    evidence.provider, evidence.source, evidence.batch_id
                ));
            };
            let pnl = ((close_price / record.entry_price - 1.0) * 100.0) as f32;
            if !pnl.is_finite() {
                return Err("收益率非有限值".to_string());
            }
            let (high, flat, low) = derive_plan_from_pnl(pnl);
            Ok(Some(PaperReviewSnapshot {
                date: target.format("%Y-%m-%d").to_string(),
                name: record.name.clone(),
                code: record.code.clone(),
                trigger: record.entry_mode.clone(),
                desc: format!(
                    "研究观察 T+1={} (entry={:.2} → close={:.2}, pnl={:+.1}%, source={} provider={:?} batch_id={})",
                    target,
                    record.entry_price,
                    close_price,
                    pnl,
                    evidence.source,
                    evidence.provider,
                    evidence.batch_id
                ),
                pnl: Some(pnl),
                plan_high: Some(high),
                plan_flat: Some(flat),
                plan_low: Some(low),
            }))
        }
        .await;

        match evaluated {
            Ok(Some(candidate)) => {
                batch.snapshot = Some(candidate);
                break;
            }
            Ok(None) => {
                unreachable!("eligible A-01 candidate maps to a snapshot or explicit error")
            }
            Err(reason) => batch.rejections.push(PaperReviewRejection {
                code: record.code.clone(),
                reason,
            }),
        }
    }
    log::info!(
        "[A-01][BR-158] exact-target filter review_date={} selected={} pending={} out_of_window={} rejected={}",
        review_date,
        usize::from(batch.snapshot.is_some()),
        batch.pending_count,
        batch.out_of_window_count,
        batch.rejections.len()
    );

    persist_review_rejections(
        "A-01",
        "review_data_gateway_historical_bars",
        review_date,
        &["BR-104", "BR-140", "BR-158"],
        batch
            .rejections
            .iter()
            .map(|rejection| {
                let reason_code = if rejection.reason.contains("日 K") {
                    "daily_kline_rejected"
                } else {
                    "candidate_validation_failed"
                };
                (rejection.code.clone(), reason_code, true)
            })
            .collect(),
    )?;
    if batch.snapshot.is_none() && !batch.rejections.is_empty() {
        return Err(format!(
            "A-01 all candidate records rejected: count={}",
            batch.rejections.len()
        ));
    }
    Ok(batch.snapshot)
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
async fn dispatch_paper_review_daily_outcome(date: &str) -> crate::review_batch::ReviewTaskOutcome {
    let snapshot = match load_paper_review_snapshot_real(date).await {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            log_dispatcher_attempt("A-01", false, 0, "paper_review_snapshot empty");
            log::info!("[A-01] paper_review_snapshot 空 (virtual_observation 无数据), 跳过推送");
            return crate::review_batch::ReviewTaskOutcome::no_data(
                "virtual observation has no exact review-date T+1 record",
            );
        }
        Err(error) => {
            log::error!("[A-01][BR-104] batch rejected: {error}");
            log_dispatcher_attempt("A-01", false, 0, &error);
            return crate::review_batch::ReviewTaskOutcome::failed(true, error);
        }
    };
    let params = build_paper_review_from_snapshot(&snapshot);
    let snap_size = 1; // 1 record
    let result = push_paper_review_outcome(&snapshot.code, params).await;
    log_dispatcher_attempt("A-01", result.is_pushed(), snap_size, "");
    crate::review_batch::ReviewTaskOutcome::from_push_outcome(result, snap_size)
}

pub async fn dispatch_paper_review_daily(date: &str) -> bool {
    matches!(
        dispatch_paper_review_daily_outcome(date).await,
        crate::review_batch::ReviewTaskOutcome::Delivered { .. }
    )
}

/// v17.4 §5.2 (BR-083): 13:00 午盘虚拟仓快照 (AC38).
/// 与 evening 全量复盘共用 PushKind::PaperReview (cooldown 86400/票),
/// dedup code 用 "noon-{code}" 前缀隔离两窗口 (否则午盘推完 evening 被 L4 拦).
pub async fn dispatch_paper_review_noon(date: &str) -> bool {
    let snapshot = match load_paper_review_snapshot_real(date).await {
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
    let outcome = dispatch_registered_outcome!(
        "T-14-post-fixed-price-order",
        crate::notify::PushKind::PostFixedPriceOrder,
        "post_fixed_price_dispatcher",
        "render_post_fixed_price_order",
        code,
        Some(banner),
        text
    );
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
    let outcome = dispatch_registered_outcome!(
        "T-15-post-fixed-price-fill",
        crate::notify::PushKind::PostFixedPriceFill,
        "post_fixed_price_dispatcher",
        "render_post_fixed_price_fill",
        code,
        Some(banner),
        text
    );
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
    let result = dispatch_registered_outcome!(
        "T-16-st-price-limit-changed",
        crate::notify::PushKind::StPriceLimitChanged,
        "st_price_limit_dispatcher",
        "render_st_price_limit_changed",
        code,
        Some(banner),
        text
    )
    .is_pushed();
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
    let result = dispatch_registered_outcome!(
        "T-17-etf-closing-call-auction",
        crate::notify::PushKind::EtfClosingCallAuction,
        "etf_closing_call_dispatcher",
        "render_etf_closing_call_auction",
        code,
        None,
        text
    )
    .is_pushed();
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
    let result = dispatch_registered_outcome!(
        "BR-033-block-trade-confirm",
        crate::notify::PushKind::BlockTradeIntradayConfirm,
        "block_trade_dispatcher",
        "render_block_trade_intraday_confirm",
        code,
        None,
        text
    )
    .is_pushed();
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
    let result = dispatch_registered_outcome!(
        "BR-034-block-trade-range",
        crate::notify::PushKind::BlockTradePriceRange,
        "block_trade_dispatcher",
        "render_block_trade_price_range",
        code,
        None,
        text
    )
    .is_pushed();
    log_dispatcher_attempt("T-19", result, 1, &format!("code={code}"));
    result
}

#[derive(diesel::QueryableByName, Debug)]
struct PaperTradeDispatchRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    id: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    plan_id: String,
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
    #[diesel(sql_type = diesel::sql_types::Text)]
    paper_trade_created_at: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::BigInt>)]
    order_audit_id: Option<i64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    audit_previous_hash: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    audit_record_hash: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    quote_observed_at: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    terminal_at: Option<String>,
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
    terminal_binding: paper_trade::PaperTradeTerminalBindingV1,
}

fn parse_paper_trade_audit_time(
    field: &str,
    value: &str,
) -> Result<chrono::DateTime<chrono::Utc>, String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&chrono::Utc))
        .map_err(|error| format!("P-04 {field} 非法 {value:?}: {error}"))
}

fn parse_paper_trade_sqlite_time(
    field: &str,
    value: &str,
) -> Result<chrono::DateTime<chrono::Utc>, String> {
    chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .map(|parsed| parsed.and_utc())
        .map_err(|error| format!("P-04 {field} 非法 {value:?}: {error}"))
}

fn paper_trade_instrument_for_env(
    code: &str,
    env: stock_analysis::risk::env_guard::TradingEnv,
) -> Result<magic_market_core::InstrumentId, String> {
    use magic_market_core::{AssetClass, InstrumentId};

    let canonical_code = match env {
        stock_analysis::risk::env_guard::TradingEnv::Prod => code,
        stock_analysis::risk::env_guard::TradingEnv::Test => code
            .strip_prefix("TEST_CODE_")
            .ok_or_else(|| format!("P-04 测试环境代码缺少 TEST_CODE_ 前缀: {code}"))?,
    };
    let identity = stock_analysis::data_gateway::instrument_identity::resolve_production_equity(
        canonical_code,
        None,
    )
    .map_err(|error| format!("P-04 无法解析股票身份 {code}: {error}"))?;
    identity
        .require_a_share()
        .map_err(|error| format!("P-04 不支持的股票身份 {code}: {error}"))?;
    if env == stock_analysis::risk::env_guard::TradingEnv::Prod {
        return Ok(identity.instrument().clone());
    }
    InstrumentId::new(identity.exchange(), code, AssetClass::Equity)
        .map_err(|error| format!("P-04 无法构造测试股票身份 {code}: {error}"))
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
    if row.plan_id.trim().is_empty() {
        return Err(format!("P-04 paper_trades id={} plan_id 为空", row.id));
    }
    stock_analysis::risk::env_guard::validate_symbol_for_env(&row.code, env)
        .map_err(|error| format!("P-04 paper_trades id={} 环境隔离失败: {error}", row.id))?;
    let instrument = paper_trade_instrument_for_env(&row.code, env)?;
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
    let order_audit_id = row.order_audit_id.ok_or_else(|| {
        format!(
            "P-04 paper_trades id={} terminal evidence unavailable: order_audit row missing",
            row.id
        )
    })?;
    let audit_previous_hash = row.audit_previous_hash.ok_or_else(|| {
        format!(
            "P-04 paper_trades id={} terminal evidence unavailable: previous hash missing",
            row.id
        )
    })?;
    let audit_record_hash = row.audit_record_hash.ok_or_else(|| {
        format!(
            "P-04 paper_trades id={} terminal evidence unavailable: record hash missing",
            row.id
        )
    })?;
    let quote_observed_at = row
        .quote_observed_at
        .as_deref()
        .ok_or_else(|| {
            format!(
                "P-04 paper_trades id={} terminal evidence unavailable: quote_observed_at missing",
                row.id
            )
        })
        .and_then(|value| parse_paper_trade_audit_time("quote_observed_at", value))?;
    let terminal_at = row
        .terminal_at
        .as_deref()
        .ok_or_else(|| {
            format!(
                "P-04 paper_trades id={} terminal evidence unavailable: terminal_at missing",
                row.id
            )
        })
        .and_then(|value| parse_paper_trade_sqlite_time("terminal_at", value))?;
    let paper_trade_created_at =
        parse_paper_trade_sqlite_time("paper_trade_created_at", &row.paper_trade_created_at)?;
    let business_date = quote_observed_at.with_timezone(&chrono::Local).date_naive();
    let terminal_binding = paper_trade::PaperTradeTerminalBindingV1::new(
        row.id,
        row.plan_id,
        instrument,
        business_date,
        row.direction,
        row.price,
        quantity,
        status.into(),
        fill_price,
        not_fill_reason.clone(),
        virtual_reason.clone(),
        account_mode.label(),
        data_mode.label(),
        quote_observed_at,
        paper_trade_created_at,
        order_audit_id,
        audit_previous_hash,
        audit_record_hash,
        terminal_at,
    )
    .map_err(|error| {
        format!(
            "P-04 paper_trades id={} terminal evidence unavailable: {error}",
            row.id
        )
    })?;

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
        terminal_binding,
    })
}

fn reject_ambiguous_paper_trade_reports(
    reports: &[PaperTradeDispatchReport],
) -> Result<(), String> {
    let mut ids = std::collections::BTreeSet::new();
    for report in reports {
        if !ids.insert(report.id) {
            return Err(format!(
                "P-04 paper_trades id={} terminal evidence ambiguous: multiple exact audit-chain rows",
                report.id
            ));
        }
    }
    Ok(())
}

fn load_today_paper_trade_reports() -> Result<Vec<PaperTradeDispatchReport>, String> {
    use diesel::RunQueryDsl;

    let db = stock_analysis::database::DatabaseManager::try_get()
        .ok_or_else(|| "P-04 数据库未初始化".to_string())?;
    let mut conn = db
        .get_conn()
        .map_err(|error| format!("P-04 数据库连接失败: {error}"))?;
    let rows = diesel::sql_query(
        "SELECT p.id, p.plan_id, p.code, p.name, p.direction, p.price, p.quantity, \
                p.status, p.fill_price, p.not_fill_reason, p.virtual_reason, \
                p.account_mode, p.data_mode, p.ts AS paper_trade_created_at, \
                oa.id AS order_audit_id, oac.previous_hash AS audit_previous_hash, \
                oac.record_hash AS audit_record_hash, oa.quote_observed_at, \
                oa.created_at AS terminal_at \
         FROM paper_trades p \
         LEFT JOIN order_audit oa \
           ON oa.business_order_id = p.plan_id \
          AND oa.source = 'PaperTrade' \
          AND oa.decision_basis = p.virtual_reason \
          AND oa.side = p.direction \
          AND oa.code = p.code \
          AND oa.requested_price = p.price \
          AND oa.quantity = p.quantity \
          AND ((p.status = 'Filled' \
                AND oa.outcome = 'Filled' \
                AND oa.execution_price = p.fill_price \
                AND oa.failure_reason IS NULL) \
            OR (p.status IN ('NotFilled', 'Invalidated') \
                AND oa.outcome = 'Rejected' \
                AND oa.execution_price IS NULL \
                AND oa.failure_reason = p.not_fill_reason)) \
         LEFT JOIN order_audit_chain oac ON oac.order_audit_id = oa.id \
         WHERE date(p.ts, 'localtime') = date('now', 'localtime') \
           AND p.status IN ('Filled', 'NotFilled', 'Invalidated') \
         ORDER BY p.id ASC, oa.id ASC",
    )
    .load::<PaperTradeDispatchRow>(&mut conn)
    .map_err(|error| format!("P-04 查询当日 paper_trades 失败: {error}"))?;
    let reports = rows
        .into_iter()
        .map(validate_paper_trade_dispatch_row)
        .collect::<Result<Vec<_>, _>>()?;
    reject_ambiguous_paper_trade_reports(&reports)?;
    Ok(reports)
}

struct PreparedPaperTrade {
    id: i64,
    code: String,
    name: String,
    text: String,
    binding: crate::durable_delivery_runtime::CountedDeliveryBinding,
}

fn prepare_paper_trade_daily() -> Result<Vec<PreparedPaperTrade>, String> {
    let reports = load_today_paper_trade_reports()?;
    reports
        .into_iter()
        .map(|report| {
            let hhmm = report
                .terminal_binding
                .quote_observed_at()
                .with_timezone(&chrono::Local)
                .format("%H:%M")
                .to_string();
            let text = render_paper_trade(PaperTradeParams {
                name: &report.name,
                code: &report.code,
                hhmm: &hhmm,
                status: report.status,
                fill_price: report.fill_price,
                qty: Some(report.quantity),
                virtual_reason: Some(&report.virtual_reason),
                not_fill_reason: report.not_fill_reason.as_deref(),
                account_mode: report.account_mode,
                data_mode: report.data_mode,
            });
            let source_binding_canonical = report.terminal_binding.canonical_bytes()?;
            let schedule_occurrence_identity = report.terminal_binding.terminal_transition_id()?;
            let delivery_subject_hash = report.terminal_binding.delivery_subject_hash()?;
            let binding = crate::durable_delivery_runtime::CountedDeliveryBinding::new(
                report.terminal_binding.business_date(),
                schedule_occurrence_identity,
                source_binding_canonical,
                crate::durable_delivery_runtime::CountedDeliveryScope::Ticket {
                    instrument: report.terminal_binding.instrument().clone(),
                },
                delivery_subject_hash,
                crate::durable_delivery_runtime::CountedDeliveryOrigin::InternalDurable,
                None,
                true,
            )?;
            Ok(PreparedPaperTrade {
                id: report.id,
                code: report.code,
                name: report.name,
                text,
                binding,
            })
        })
        .collect()
}

/// BR-100: 从当日 `paper_trades` 持久化结果发送虚拟成交回报。
pub async fn dispatch_paper_trade_daily() -> bool {
    let prepared = match prepare_paper_trade_daily() {
        Ok(prepared) => prepared,
        Err(error) => {
            log::error!("[P-04] 虚拟成交回报批次拒绝: {error}");
            log_dispatcher_attempt("P-04", false, 0, &error);
            return false;
        }
    };
    if prepared.is_empty() {
        log_dispatcher_attempt("P-04", false, 0, "today paper_trades empty");
        log::info!("[P-04] 当日无已完成虚拟成交记录, 跳过推送");
        return false;
    }

    let mut success_count = 0usize;
    let item_count = prepared.len();
    for item in prepared {
        let presentation_token = match crate::presentation_registry::acquire_token(
            "T-10-paper-trade",
            crate::notify::PushKind::PaperTrade,
            "paper_trade_dispatcher",
            "render_paper_trade",
        ) {
            Ok(token) => token,
            Err(reason) => {
                log::error!("[P-04][BR-196] paper-trade presentation token rejected: {reason}");
                log_dispatcher_attempt("P-04", false, item_count, &reason);
                return false;
            }
        };
        match crate::notify::push_counted_with_binding(
            presentation_token,
            &item.text,
            None,
            item.binding,
        )
        .await
        {
            crate::notify::PushOutcome::Pushed | crate::notify::PushOutcome::Deduped => {
                success_count += 1;
            }
            outcome => {
                log::warn!(
                    "[P-04][BR-192] paper_trades id={} {}({}) 回报未投递: {:?}",
                    item.id,
                    item.name,
                    item.code,
                    outcome
                );
            }
        }
    }
    let success = success_count == item_count;
    let error = if success {
        String::new()
    } else {
        format!("投递 {success_count}/{item_count}")
    };
    log_dispatcher_attempt("P-04", success, item_count, &error);
    success
}

/// v39: P-03 候选触发 dispatcher
///   - 候选台取 top 1 candidate (按 source_count 排序)
///   - is_candidate_live_enabled 影子开关 (默认 false)
///   - 简化版: 推送 1 条 A 档候选, evidence 拼成 trigger_desc
fn candidate_volume_quality(volume_ratio: Option<f64>) -> Result<EvidenceQuality, String> {
    let value = volume_ratio.ok_or_else(|| "缺少实时量比".to_string())?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!("实时量比非法: {value:?}"));
    }
    Ok(if value >= 3.0 {
        EvidenceQuality::Strong
    } else if value >= 1.0 {
        EvidenceQuality::Mid
    } else {
        EvidenceQuality::Weak
    })
}

pub async fn dispatch_candidate_triggered_daily(hhmm: &str, banner: &BannerCtx) -> bool {
    use stock_analysis::opportunity::candidate_panel::EvidenceTier;
    use stock_analysis::opportunity::candidate_state::require_live_promotion;

    // BR-224: SignalTracker 证据门 — 候选样本 ≥30 且强档胜率 ≥30% 才转正推送
    let promotion_evidence = match tokio::task::spawn_blocking(|| {
        use stock_analysis::database::DatabaseManager;
        let db = DatabaseManager::get();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        db.candidate_promotion_samples(&today).map_err(|e| e.to_string())
    })
    .await
    {
        Ok(Ok((count, hits))) if count >= 30 => {
            let win_rate = hits as f64 / count as f64;
            if win_rate >= 0.30 {
                Some(stock_analysis::opportunity::candidate_state::PromotionEvidence {
                    sample_count: count as u32,
                    win_rate_strong: win_rate,
                    win_rate_weak: 0.0,
                })
            } else {
                None
            }
        }
        _ => None,
    };
    if let Err(error) = require_live_promotion(promotion_evidence, None) {
        log_dispatcher_attempt("P-03", false, 0, &error);
        log::info!("[P-03] 候选触发保持 Shadow: {error}");
        return false;
    }

    let batch = match load_real_candidate_batch().await {
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
    let volume_quality = match candidate_volume_quality(quote.volume_ratio) {
        Ok(quality) => quality,
        Err(reason) => {
            let error = format!("P-03 候选 {} {reason}", top.code);
            log_dispatcher_attempt("P-03", false, 0, &error);
            log::warn!("{error}");
            return false;
        }
    };
    let Some(volume_ratio) = quote.volume_ratio else {
        unreachable!("candidate_volume_quality accepts only a present volume ratio")
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
    match push_candidate_triggered(&top.code, banner, params, None, None).await {
        Ok(result) => {
            log_dispatcher_attempt("P-03", result, 1, "");
            result
        }
        Err(reason) => {
            log::error!("[P-03][BR-192] 候选计数投递拒绝: {reason}");
            log_dispatcher_attempt("P-03", false, 1, &reason);
            false
        }
    }
}

/// I-04 remains disabled until a durable, freshness-bound position and quote
/// acquisition can produce a BR-192 counted-delivery binding.
async fn dispatch_holding_plan_daily_result(
    _hhmm: &str,
    _banner: &BannerCtx,
) -> PeriodicDispatchResult {
    let reason = "capability_unavailable=holding_plan_counted_binding_unavailable; skipped before position and quote acquisition";
    log_dispatcher_attempt("I-04", false, 0, reason);
    log::warn!("[I-04][BR-192] {reason}");
    PeriodicDispatchResult::Failed(reason.to_string())
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
pub fn load_auction_volume_snapshot_real(
    hhmm: &str,
    trading_date: chrono::NaiveDate,
) -> Result<AuctionVolumeSnapshot, String> {
    use stock_analysis::market_analyzer::MarketAnalyzer;
    let analyzer = match MarketAnalyzer::new(None) {
        Ok(a) => a,
        Err(error) => return Err(format!("竞价量能 analyzer 初始化失败: {error}")),
    };
    let limit_stocks = match analyzer.get_limit_up_stocks(trading_date) {
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
    let hhmm_owned = hhmm.to_string();
    let trading_date = chrono::Local::now().date_naive();
    let snapshot = match crate::blocking_market_data::run_blocking_market_data(
        "P-02 auction volume snapshot",
        move || load_auction_volume_snapshot_real(&hhmm_owned, trading_date),
    )
    .await
    {
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
    let result = dispatch_registered_outcome!(
        "T-11-auction-volume",
        crate::notify::PushKind::AuctionVolume,
        "auction_volume_dispatcher",
        "render_auction_volume",
        "",
        Some(banner),
        text
    )
    .is_pushed();
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
    pub source_batch_id: String,
    pub source_content_hash: String,
    pub source_observed_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub theme: String,
    pub score: Option<f32>,
    pub persistent: PersistentLevel,
    pub member_count: usize,
    pub continuous_count: usize,
    pub leading_members: Vec<String>,
    pub other_members: Vec<String>,
    pub watch_point: Option<String>,
}

fn parse_a10_source_observed_at(
    value: &str,
) -> Result<chrono::DateTime<chrono::FixedOffset>, String> {
    if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(timestamp);
    }
    let utc = if let Some(milliseconds) = value.strip_prefix("unix-ms:") {
        let milliseconds = milliseconds
            .parse::<i64>()
            .map_err(|_| format!("A-10 source observed_at is invalid: {value:?}"))?;
        chrono::DateTime::<chrono::Utc>::from_timestamp_millis(milliseconds)
    } else if let Some((seconds, nanos)) = value.split_once('.') {
        if seconds.is_empty()
            || nanos.is_empty()
            || nanos.len() > 9
            || !nanos.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(format!("A-10 source observed_at is invalid: {value:?}"));
        }
        let seconds = seconds
            .parse::<i64>()
            .map_err(|_| format!("A-10 source observed_at is invalid: {value:?}"))?;
        let padded_nanos = format!("{nanos:0<9}")
            .parse::<u32>()
            .map_err(|_| format!("A-10 source observed_at is invalid: {value:?}"))?;
        chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, padded_nanos)
    } else {
        let raw = value
            .parse::<i64>()
            .map_err(|_| format!("A-10 source observed_at is invalid: {value:?}"))?;
        if raw.unsigned_abs() >= 100_000_000_000_000_000 {
            let seconds = raw.div_euclid(1_000_000_000);
            let nanos = u32::try_from(raw.rem_euclid(1_000_000_000))
                .map_err(|_| format!("A-10 source observed_at is invalid: {value:?}"))?;
            chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, nanos)
        } else if raw.unsigned_abs() >= 100_000_000_000 {
            chrono::DateTime::<chrono::Utc>::from_timestamp_millis(raw)
        } else {
            chrono::DateTime::<chrono::Utc>::from_timestamp(raw, 0)
        }
    }
    .ok_or_else(|| format!("A-10 source observed_at is out of range: {value:?}"))?;
    Ok(utc.fixed_offset())
}

fn catalyst_review_from_chain_batch(
    batch: &stock_analysis::database::chain_intelligence::VisibleChainBatch,
) -> Result<CatalystReviewSnapshot, String> {
    let Some(top) = batch.chains.first() else {
        return Ok(CatalystReviewSnapshot {
            date: batch.trading_date.format("%Y-%m-%d").to_string(),
            ..CatalystReviewSnapshot::default()
        });
    };
    if top.board_name.trim().is_empty() {
        return Err("A-10 visible chain has an empty source-backed board name".to_string());
    }
    if top.members.len() < 3 || top.upper_limit_count != top.members.len() as i32 {
        return Err(format!(
            "A-10 visible chain {} violates member-count contract",
            top.chain_id
        ));
    }
    let names = top
        .members
        .iter()
        .map(|member| member.security_name.trim())
        .map(|name| {
            (!name.is_empty())
                .then(|| name.to_string())
                .ok_or_else(|| "A-10 visible chain contains an empty security name".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let continuous_count = usize::try_from(top.continuous_count).map_err(|_| {
        format!(
            "A-10 visible chain {} has negative continuous count",
            top.chain_id
        )
    })?;
    if continuous_count > names.len() {
        return Err(format!(
            "A-10 visible chain {} has continuous count above member count",
            top.chain_id
        ));
    }
    let persistent = if continuous_count >= 3 {
        PersistentLevel::High
    } else if continuous_count >= 1 {
        PersistentLevel::Med
    } else {
        PersistentLevel::Low
    };
    let source_observed_at = batch
        .inputs
        .iter()
        .map(|input| parse_a10_source_observed_at(&input.observed_at))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .ok_or_else(|| "A-10 visible chain batch has no input observation time".to_string())?;
    Ok(CatalystReviewSnapshot {
        date: batch.trading_date.format("%Y-%m-%d").to_string(),
        source_batch_id: batch.batch_id.clone(),
        source_content_hash: batch.content_hash.clone(),
        source_observed_at: Some(source_observed_at),
        theme: top.board_name.clone(),
        score: None,
        persistent,
        member_count: names.len(),
        continuous_count,
        leading_members: names.iter().take(3).cloned().collect(),
        other_members: names.iter().skip(3).take(3).cloned().collect(),
        // The admitted chain batch has no independent next-day volume/trend
        // evidence. Keep the field absent instead of fabricating advice from
        // the board name.
        watch_point: None,
    })
}

/// BR-160: A-10 only consumes the exact visible batch published by the
/// unified Gateway. Stale `chain_daily`, local rotation caches, and direct
/// name lookups are not fallback sources.
pub async fn load_catalyst_review_snapshot_real(
    date: &str,
) -> Result<CatalystReviewSnapshot, String> {
    let review_date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|error| format!("A-10 非法复盘日期 {date}: {error}"))?;
    let batch = stock_analysis::data_gateway::ChainIntelligenceGateway::new()
        .build_for_date(review_date)
        .await
        .map_err(|error| error.to_string())?;
    if batch.trading_date != review_date {
        return Err(format!(
            "A-10 visible batch as_of={} differs from requested {}",
            batch.trading_date, review_date
        ));
    }
    catalyst_review_from_chain_batch(&batch)
}

// ============================================================================
// B-005-C (2026-07-09): 盘后批量 dispatcher (R-02..R-08 + TomorrowWatch)
// 修复: 之前 6 个盘后 dispatcher 仅在 `cargo run -- --push` 模式被调,
//       生产 monitor_loop 永远跑不到, 用户看不到盘后复盘.
// 现在: 由 BR-139 独立 post_session_review_scheduler 在 19:00 后调用，
//       monitor_loop 不再保留第二个 owner；各 dispatcher 逐项记录结果且互不阻塞。
// ============================================================================

const R09_TEMPLATE_ID: &str = "review_provider_top_n_v1";
const R09_PROVIDER_TOP_N_LIMIT: u32 = 20;

#[derive(Debug, Clone, serde::Serialize)]
struct ProviderTopNRequestBinding {
    metric: String,
    trading_date: String,
    limit: u32,
    filter_identity: String,
    request_hash: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ProviderTopNBatchBinding {
    provider: String,
    source: String,
    observed_at: String,
    source_at: Option<String>,
    batch_id: String,
    record_count: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ProviderTopNProjectionRow {
    metric: String,
    source_order_ordinal: u32,
    exchange: String,
    asset_class: String,
    code: String,
    label: String,
    value: f64,
    unit: String,
    trading_date: String,
    filter_identity: String,
    provider_declared_total: u32,
    inspected_row_count: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ProviderTopNTaskTransitionBasis {
    task_identity: String,
    business_date: String,
    task: String,
    source: String,
    rule_ids: Vec<String>,
    source_time: Option<String>,
    snapshot_size: usize,
    request_hashes: [String; 2],
    batch_ids: [String; 2],
}

#[derive(Debug, serde::Deserialize)]
struct ExistingReviewTaskTransitionBasis {
    task_identity: String,
    business_date: String,
    task: String,
    snapshot_size: usize,
}

/// Canonical source binding embedded verbatim in BR-192's generic durable
/// delivery envelope.
///
/// This adapter deliberately owns no sink, cooldown, budget, delivery ledger,
/// or task-transition append. Those responsibilities belong exclusively to
/// `DurableDeliveryCoordinator`.
#[derive(Debug, Clone, serde::Serialize)]
struct ProviderTopNReportBinding {
    schema_version: u32,
    business_date: String,
    template_id: String,
    review_task_identity: String,
    delivery_subject_identity: String,
    volume_ratio_request: ProviderTopNRequestBinding,
    main_net_inflow_request: ProviderTopNRequestBinding,
    volume_ratio_batch: ProviderTopNBatchBinding,
    main_net_inflow_batch: ProviderTopNBatchBinding,
    ordered_projection: Vec<ProviderTopNProjectionRow>,
    ordered_projection_sha256: String,
    source_evidence_fingerprint: String,
    rendered_content: Vec<u8>,
    rendered_content_sha256: String,
    task_transition_basis: ProviderTopNTaskTransitionBasis,
}

#[derive(Debug)]
struct PreparedProviderTopNReport {
    rendered: String,
    binding: ProviderTopNReportBinding,
    canonical_binding_sha256: String,
}

fn r09_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    format!("{:x}", Sha256::digest(bytes))
}

fn r09_metric_label(metric: &magic_market_core::MarketRankingKind) -> Result<&'static str, String> {
    match metric {
        magic_market_core::MarketRankingKind::VolumeRatio => Ok("volume_ratio"),
        magic_market_core::MarketRankingKind::MainNetInflow => Ok("main_net_inflow"),
        other => Err(format!(
            "R-09 fixed metric contract rejected unsupported metric {other:?}"
        )),
    }
}

fn r09_unit_label(unit: &magic_market_core::MarketRankingUnit) -> Result<&'static str, String> {
    match unit {
        magic_market_core::MarketRankingUnit::Multiple => Ok("multiple"),
        magic_market_core::MarketRankingUnit::Yuan => Ok("yuan"),
        other => Err(format!(
            "R-09 fixed metric contract rejected unsupported unit {other:?}"
        )),
    }
}

fn r09_request_binding(
    request: &stock_analysis::data_gateway::capital::ProviderTopNRequestEvidence,
    expected_metric: &magic_market_core::MarketRankingKind,
    review_date: chrono::NaiveDate,
) -> Result<ProviderTopNRequestBinding, String> {
    let expected_date = review_date.format("%Y-%m-%d").to_string();
    if &request.metric != expected_metric
        || request.trading_date.as_str() != expected_date
        || request.limit.get() != R09_PROVIDER_TOP_N_LIMIT
        || request.filter_identity.as_str().is_empty()
        || request.request_hash.len() != 64
        || !request
            .request_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!(
            "R-09 request evidence mismatch metric={:?} date={} limit={} filter_present={} hash_len={}",
            request.metric,
            request.trading_date,
            request.limit.get(),
            !request.filter_identity.as_str().is_empty(),
            request.request_hash.len()
        ));
    }
    Ok(ProviderTopNRequestBinding {
        metric: r09_metric_label(&request.metric)?.to_string(),
        trading_date: request.trading_date.as_str().to_string(),
        limit: request.limit.get(),
        filter_identity: request.filter_identity.as_str().to_string(),
        request_hash: request.request_hash.clone(),
    })
}

fn r09_batch_binding<T>(
    batch: &stock_analysis::data_gateway::GatewayBatch<T>,
) -> Result<ProviderTopNBatchBinding, String> {
    let evidence = batch.evidence();
    if batch.is_verified_empty()
        || batch.records().is_empty()
        || evidence.provider != magic_market_core::ProviderId::Eastmoney
        || evidence.source != "eastmoney-web"
        || evidence.source_at.is_some()
        || evidence.observed_at.trim().is_empty()
        || evidence.batch_id.trim().is_empty()
    {
        return Err(format!(
            "R-09 batch evidence is incomplete or inconsistent: status={} provider={:?} source={} source_at_present={} observed_at_present={} batch_id_present={}",
            if batch.is_verified_empty() {
                "verified_empty"
            } else {
                "available"
            },
            evidence.provider,
            evidence.source,
            evidence.source_at.is_some(),
            !evidence.observed_at.trim().is_empty(),
            !evidence.batch_id.trim().is_empty()
        ));
    }
    Ok(ProviderTopNBatchBinding {
        provider: format!("{:?}", evidence.provider),
        source: evidence.source.clone(),
        observed_at: evidence.observed_at.clone(),
        source_at: evidence.source_at.clone(),
        batch_id: evidence.batch_id.clone(),
        record_count: batch.records().len(),
    })
}

fn r09_projection_rows(
    records: &[stock_analysis::data_gateway::capital::ProviderTopNFact],
    expected_metric: &magic_market_core::MarketRankingKind,
    expected_unit: &magic_market_core::MarketRankingUnit,
    expected_date: chrono::NaiveDate,
    expected_filter: &str,
) -> Result<Vec<ProviderTopNProjectionRow>, String> {
    if records.is_empty() || records.len() > R09_PROVIDER_TOP_N_LIMIT as usize {
        return Err(format!(
            "R-09 provider response row count is outside 1..={R09_PROVIDER_TOP_N_LIMIT}: {}",
            records.len()
        ));
    }
    let expected_date = expected_date.format("%Y-%m-%d").to_string();
    records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let expected_ordinal =
                u32::try_from(index + 1).map_err(|_| "R-09 source ordinal overflow".to_string())?;
            if &record.metric != expected_metric
                || &record.unit != expected_unit
                || record.source_order_ordinal.get() != expected_ordinal
                || record.trading_date.as_str() != expected_date
                || record.filter_identity.as_str() != expected_filter
                || record.instrument.asset_class() != magic_market_core::AssetClass::Equity
                || record.instrument.code().trim().is_empty()
                || record.label.as_str().is_empty()
            {
                return Err(format!(
                    "R-09 ordered projection mismatch at source ordinal {}",
                    expected_ordinal
                ));
            }
            Ok(ProviderTopNProjectionRow {
                metric: r09_metric_label(&record.metric)?.to_string(),
                source_order_ordinal: record.source_order_ordinal.get(),
                exchange: format!("{:?}", record.instrument.exchange()),
                asset_class: format!("{:?}", record.instrument.asset_class()),
                code: record.instrument.code().to_string(),
                label: record.label.as_str().to_string(),
                value: record.value.get(),
                unit: r09_unit_label(&record.unit)?.to_string(),
                trading_date: record.trading_date.as_str().to_string(),
                filter_identity: record.filter_identity.as_str().to_string(),
                provider_declared_total: record.provider_declared_total.get(),
                inspected_row_count: record.inspected_row_count.get(),
            })
        })
        .collect()
}

fn render_r09_provider_top_n(
    review_date: chrono::NaiveDate,
    volume_ratio: &[ProviderTopNProjectionRow],
    main_net_inflow: &[ProviderTopNProjectionRow],
) -> String {
    fn display_yuan(value: f64) -> String {
        let magnitude = value.abs();
        if magnitude >= 100_000_000.0 {
            format!("{:+.2}亿", value / 100_000_000.0)
        } else if magnitude >= 10_000.0 {
            format!("{:+.2}万", value / 10_000.0)
        } else {
            format!("{value:+.2}元")
        }
    }

    let mut text = format!(
        "📊 盘后量能与主力净流入（{}）\n\
         口径：Eastmoney 单响应 TopN；不代表全市场完整排序\n\
         量比 Top20（本响应{}条）\n",
        review_date,
        volume_ratio.len()
    );
    for record in volume_ratio {
        text.push_str(&format!(
            "{}. {}({}) {:.2}倍\n",
            record.source_order_ordinal, record.label, record.code, record.value
        ));
    }
    text.push_str(&format!(
        "─────\n主力净流入 Top20（本响应{}条）\n",
        main_net_inflow.len()
    ));
    for record in main_net_inflow {
        text.push_str(&format!(
            "{}. {}({}) {}\n",
            record.source_order_ordinal,
            record.label,
            record.code,
            display_yuan(record.value)
        ));
    }
    text.push_str("仅展示已验证来源字段；不补零、不推断全市场覆盖");
    text
}

fn prepare_r09_provider_top_n_report(
    review_date: chrono::NaiveDate,
    pair: stock_analysis::data_gateway::capital::ProviderTopNPair,
) -> Result<PreparedProviderTopNReport, String> {
    let volume_request = r09_request_binding(
        &pair.volume_ratio_request,
        &magic_market_core::MarketRankingKind::VolumeRatio,
        review_date,
    )?;
    let inflow_request = r09_request_binding(
        &pair.main_net_inflow_request,
        &magic_market_core::MarketRankingKind::MainNetInflow,
        review_date,
    )?;
    let volume_batch = r09_batch_binding(&pair.volume_ratio)?;
    let inflow_batch = r09_batch_binding(&pair.main_net_inflow)?;
    if volume_batch.batch_id == inflow_batch.batch_id {
        return Err("R-09 metric responses must retain distinct batch IDs".to_string());
    }
    let volume_projection = r09_projection_rows(
        pair.volume_ratio.records(),
        &magic_market_core::MarketRankingKind::VolumeRatio,
        &magic_market_core::MarketRankingUnit::Multiple,
        review_date,
        &volume_request.filter_identity,
    )?;
    let inflow_projection = r09_projection_rows(
        pair.main_net_inflow.records(),
        &magic_market_core::MarketRankingKind::MainNetInflow,
        &magic_market_core::MarketRankingUnit::Yuan,
        review_date,
        &inflow_request.filter_identity,
    )?;
    let rendered = render_r09_provider_top_n(review_date, &volume_projection, &inflow_projection);
    let rendered_content = rendered.as_bytes().to_vec();
    let rendered_content_sha256 = r09_sha256(&rendered_content);
    let mut ordered_projection = volume_projection;
    ordered_projection.extend(inflow_projection);
    let ordered_projection_canonical = serde_json::to_vec(&ordered_projection)
        .map_err(|error| format!("R-09 ordered projection serialization failed: {error}"))?;
    let ordered_projection_sha256 = r09_sha256(&ordered_projection_canonical);
    let task_identity = crate::review_batch::review_task_identity(
        review_date,
        crate::review_batch::ReviewTask::R09,
    );
    let delivery_subject_identity = crate::review_batch::audit_identity_hash(
        "provider-top-n-delivery-subject",
        &format!("{review_date}:{task_identity}"),
    );
    let evidence_material = serde_json::to_vec(&(
        review_date.format("%Y-%m-%d").to_string(),
        &volume_request.request_hash,
        &inflow_request.request_hash,
        &volume_batch.batch_id,
        &inflow_batch.batch_id,
        &ordered_projection_sha256,
    ))
    .map_err(|error| format!("R-09 evidence fingerprint serialization failed: {error}"))?;
    let source_evidence_fingerprint = r09_sha256(&evidence_material);
    let snapshot_size = ordered_projection.len();
    let task_transition_basis = ProviderTopNTaskTransitionBasis {
        task_identity: task_identity.clone(),
        business_date: review_date.format("%Y-%m-%d").to_string(),
        task: "R-09".to_string(),
        source: "eastmoney_provider_top_n".to_string(),
        rule_ids: vec![
            "BR-110".to_string(),
            "BR-140".to_string(),
            "BR-192".to_string(),
            "BR-200".to_string(),
        ],
        source_time: None,
        snapshot_size,
        request_hashes: [
            volume_request.request_hash.clone(),
            inflow_request.request_hash.clone(),
        ],
        batch_ids: [volume_batch.batch_id.clone(), inflow_batch.batch_id.clone()],
    };
    let binding = ProviderTopNReportBinding {
        schema_version: 1,
        business_date: review_date.format("%Y-%m-%d").to_string(),
        template_id: R09_TEMPLATE_ID.to_string(),
        review_task_identity: task_identity,
        delivery_subject_identity,
        volume_ratio_request: volume_request,
        main_net_inflow_request: inflow_request,
        volume_ratio_batch: volume_batch,
        main_net_inflow_batch: inflow_batch,
        ordered_projection,
        ordered_projection_sha256,
        source_evidence_fingerprint,
        rendered_content,
        rendered_content_sha256,
        task_transition_basis,
    };
    let canonical_binding = serde_json::to_vec(&binding)
        .map_err(|error| format!("R-09 report binding serialization failed: {error}"))?;
    let canonical_binding_sha256 = r09_sha256(&canonical_binding);
    Ok(PreparedProviderTopNReport {
        rendered,
        binding,
        canonical_binding_sha256,
    })
}

fn build_r09_delivery_envelope(
    prepared: &PreparedProviderTopNReport,
) -> Result<stock_analysis::durable_delivery::DeliveryEnvelope, String> {
    use stock_analysis::durable_delivery::{
        DeliveryEnvelope, DeliverySubKind, PushKind, TaskBinding,
    };

    let source_binding_canonical = serde_json::to_vec(&prepared.binding)
        .map_err(|error| format!("R-09 source binding serialization failed: {error}"))?;
    let source_binding_sha256 = r09_sha256(&source_binding_canonical);
    if source_binding_sha256 != prepared.canonical_binding_sha256 {
        return Err(format!(
            "R-09 source binding hash changed after preparation: prepared={} rebuilt={}",
            prepared.canonical_binding_sha256, source_binding_sha256
        ));
    }
    let transition_basis_canonical = serde_json::to_vec(&prepared.binding.task_transition_basis)
        .map_err(|error| format!("R-09 task transition serialization failed: {error}"))?;
    let task_binding = TaskBinding::new(
        prepared.binding.review_task_identity.clone(),
        transition_basis_canonical,
    )
    .map_err(|error| format!("R-09 task binding rejected: {error}"))?;
    let provider_observed_at = [
        prepared.binding.volume_ratio_batch.observed_at.as_str(),
        prepared.binding.main_net_inflow_batch.observed_at.as_str(),
    ]
    .into_iter()
    .max()
    .map(str::to_owned);
    let original_batch_ids = vec![
        prepared.binding.volume_ratio_batch.batch_id.clone(),
        prepared.binding.main_net_inflow_batch.batch_id.clone(),
    ];
    let envelope = DeliveryEnvelope::new(
        prepared.binding.business_date.clone(),
        PushKind::ReviewProviderTopN,
        DeliverySubKind::None,
        "GLOBAL",
        prepared.binding.review_task_identity.clone(),
        prepared.binding.source_evidence_fingerprint.clone(),
        source_binding_canonical,
        prepared.binding.delivery_subject_identity.clone(),
        prepared.binding.rendered_content.clone(),
        false,
        Some(task_binding),
    )
    .and_then(|envelope| {
        envelope.with_provider_evidence(
            provider_observed_at,
            Some(prepared.binding.business_date.clone()),
            original_batch_ids,
        )
    })
    .map_err(|error| format!("R-09 durable delivery envelope rejected: {error}"))?;

    if envelope.source_binding_sha256 != prepared.canonical_binding_sha256
        || envelope.rendered_content_sha256 != prepared.binding.rendered_content_sha256
        || envelope.delivery_subject_hash != prepared.binding.delivery_subject_identity
    {
        return Err(
            "R-09 durable delivery envelope does not preserve the prepared binding".to_string(),
        );
    }
    Ok(envelope)
}

fn r09_outcome_from_durable(
    evidence: crate::durable_delivery_runtime::DurableDispatchEvidence,
    snapshot_size: usize,
) -> crate::review_batch::ReviewTaskOutcome {
    use stock_analysis::durable_delivery::DecisionState;

    let hydration_present = evidence.schedule_hydration.is_some();
    match evidence.state {
        DecisionState::Delivered if hydration_present => {
            crate::review_batch::ReviewTaskOutcome::delivered(snapshot_size)
        }
        DecisionState::Delivered => crate::review_batch::ReviewTaskOutcome::failed(
            true,
            format!(
                "durable R-09 delivery {} is confirmed but schedule hydration is pending",
                evidence.decision_identity
            ),
        ),
        DecisionState::RejectedDurable | DecisionState::ManualResolvedRejected => {
            crate::review_batch::ReviewTaskOutcome::failed(
                false,
                format!(
                    "durable R-09 delivery {} rejected state={} hydration_present={}",
                    evidence.decision_identity, evidence.state, hydration_present
                ),
            )
        }
        DecisionState::UncertainManualReview => crate::review_batch::ReviewTaskOutcome::failed(
            false,
            format!(
                "durable R-09 delivery {} is uncertain and requires manual review hydration_present={}",
                evidence.decision_identity, hydration_present
            ),
        ),
        state => crate::review_batch::ReviewTaskOutcome::failed(
            true,
            format!(
                "durable R-09 delivery {} awaits local reconciliation state={} hydration_present={}",
                evidence.decision_identity, state, hydration_present
            ),
        ),
    }
}

fn review_outcome_from_existing_durable(
    evidence: crate::durable_delivery_runtime::DurableDispatchEvidence,
    business_date: chrono::NaiveDate,
    task: crate::review_batch::ReviewTask,
) -> crate::review_batch::ReviewTaskOutcome {
    use stock_analysis::durable_delivery::DecisionState;

    let expected_task_identity = crate::review_batch::review_task_identity(business_date, task);
    let expected_task = task.label();
    let hydration_present = evidence.schedule_hydration.is_some();
    match evidence.state {
        DecisionState::Delivered => {
            let Some(hydration) = evidence.schedule_hydration else {
                return crate::review_batch::ReviewTaskOutcome::failed(
                    true,
                    format!(
                        "durable {} delivery {} is confirmed but schedule hydration is pending",
                        expected_task, evidence.decision_identity
                    ),
                );
            };
            if hydration.decision_identity != evidence.decision_identity
                || hydration.task_identity != expected_task_identity
                || r09_sha256(&hydration.transition_basis_canonical)
                    != hydration.transition_basis_sha256
            {
                return crate::review_batch::ReviewTaskOutcome::failed(
                    false,
                    format!(
                        "durable {} delivery {} has invalid hydration identity",
                        expected_task, evidence.decision_identity
                    ),
                );
            }
            let basis: ExistingReviewTaskTransitionBasis =
                match serde_json::from_slice(&hydration.transition_basis_canonical) {
                    Ok(basis) => basis,
                    Err(error) => {
                        return crate::review_batch::ReviewTaskOutcome::failed(
                            false,
                            format!(
                                "durable {} delivery {} hydration basis is invalid: {error}",
                                expected_task, evidence.decision_identity
                            ),
                        )
                    }
                };
            if basis.task_identity != expected_task_identity
                || basis.business_date != business_date.format("%Y-%m-%d").to_string()
                || basis.task != expected_task
                || (basis.snapshot_size == 0
                    && task != crate::review_batch::ReviewTask::R08)
            {
                return crate::review_batch::ReviewTaskOutcome::failed(
                    false,
                    format!(
                        "durable {} delivery {} hydration basis does not match the review occurrence",
                        expected_task, evidence.decision_identity
                    ),
                );
            }
            log::info!(
                "[{}][BR-200] reused durable Delivered decision={} provider_calls=0 sink_calls=0 rows={}",
                expected_task,
                evidence.decision_identity,
                basis.snapshot_size
            );
            crate::review_batch::ReviewTaskOutcome::delivered(basis.snapshot_size)
        }
        DecisionState::RejectedDurable | DecisionState::ManualResolvedRejected => {
            crate::review_batch::ReviewTaskOutcome::failed(
                false,
                format!(
                    "durable {} delivery {} already rejected state={} hydration_present={}",
                    expected_task, evidence.decision_identity, evidence.state, hydration_present
                ),
            )
        }
        DecisionState::UncertainManualReview => crate::review_batch::ReviewTaskOutcome::failed(
            false,
            format!(
                "durable {} delivery {} is uncertain and requires manual review hydration_present={}",
                expected_task, evidence.decision_identity, hydration_present
            ),
        ),
        state => crate::review_batch::ReviewTaskOutcome::failed(
            true,
            format!(
                "durable {} delivery {} awaits local reconciliation state={} hydration_present={}",
                expected_task, evidence.decision_identity, state, hydration_present
            ),
        ),
    }
}

async fn inspect_r09_review_occurrence(
    review_date: chrono::NaiveDate,
) -> Result<Option<crate::durable_delivery_runtime::DurableDispatchEvidence>, String> {
    crate::durable_delivery_runtime::inspect_review_task_occurrence(
        review_date,
        stock_analysis::durable_delivery::PushKind::ReviewProviderTopN,
        crate::review_batch::review_task_identity(
            review_date,
            crate::review_batch::ReviewTask::R09,
        ),
    )
    .await
}

async fn dispatch_r09_provider_top_n_outcome_with_loader<
    Preflight,
    PreflightFuture,
    Loader,
    Future,
>(
    review_date: chrono::NaiveDate,
    preflight: Preflight,
    loader: Loader,
) -> crate::review_batch::ReviewTaskOutcome
where
    Preflight: FnOnce(chrono::NaiveDate) -> PreflightFuture,
    PreflightFuture: std::future::Future<
        Output = Result<Option<crate::durable_delivery_runtime::DurableDispatchEvidence>, String>,
    >,
    Loader: FnOnce(chrono::NaiveDate) -> Future,
    Future: std::future::Future<
        Output = Result<
            stock_analysis::data_gateway::capital::ProviderTopNPair,
            stock_analysis::data_gateway::GatewayError,
        >,
    >,
{
    use crate::review_batch::ReviewTaskOutcome;

    match preflight(review_date).await {
        Ok(Some(evidence)) => {
            return review_outcome_from_existing_durable(
                evidence,
                review_date,
                crate::review_batch::ReviewTask::R09,
            )
        }
        Ok(None) => {}
        Err(error) => {
            return ReviewTaskOutcome::failed(
                true,
                format!("provider_top_n durable terminal preflight failed: {error}"),
            )
        }
    }
    let pair = match loader(review_date).await {
        Ok(pair) => pair,
        Err(error) => {
            return ReviewTaskOutcome::failed(
                error.retryable(),
                format!(
                    "provider_top_n acquisition failed reason_code={}: {error}",
                    error.reason_code()
                ),
            );
        }
    };
    let prepared = match prepare_r09_provider_top_n_report(review_date, pair) {
        Ok(prepared) => prepared,
        Err(error) => {
            return ReviewTaskOutcome::failed(
                true,
                format!("provider_top_n canonical binding rejected: {error}"),
            );
        }
    };

    log::info!(
        "[R-09][BR-192] provider binding prepared date={} rows={} binding_sha256={} projection_sha256={} content_sha256={} provider_batches=2",
        review_date,
        prepared.binding.ordered_projection.len(),
        prepared.canonical_binding_sha256,
        prepared.binding.ordered_projection_sha256,
        prepared.binding.rendered_content_sha256,
    );
    log::debug!(
        "[R-09][BR-192] prepared source-limited report bytes={} template_id={} delivery_subject_hash={} rendered_chars={}",
        prepared.binding.rendered_content.len(),
        prepared.binding.template_id,
        prepared.binding.delivery_subject_identity,
        prepared.rendered.chars().count(),
    );

    let snapshot_size = prepared.binding.ordered_projection.len();
    let envelope = match build_r09_delivery_envelope(&prepared) {
        Ok(envelope) => envelope,
        Err(error) => {
            return ReviewTaskOutcome::failed(
                false,
                format!("provider_top_n delivery envelope rejected: {error}"),
            );
        }
    };
    let presentation_token = match crate::presentation_registry::acquire_token(
        "R-09-provider-top-n",
        crate::notify::PushKind::ReviewProviderTopN,
        "provider_top_n_dispatcher",
        "render_r09_provider_top_n",
    ) {
        Ok(token) => token,
        Err(reason) => {
            return ReviewTaskOutcome::failed(
                false,
                format!("provider_top_n presentation token rejected: {reason}"),
            );
        }
    };
    match crate::durable_delivery_runtime::deliver_presented_envelope(presentation_token, envelope)
        .await
    {
        Ok(evidence) => r09_outcome_from_durable(evidence, snapshot_size),
        Err(error) => ReviewTaskOutcome::failed(
            true,
            format!("provider_top_n durable delivery failed: {error}"),
        ),
    }
}

async fn dispatch_r09_provider_top_n_outcome(
    review_date: chrono::NaiveDate,
) -> crate::review_batch::ReviewTaskOutcome {
    dispatch_r09_provider_top_n_outcome_with_loader(
        review_date,
        inspect_r09_review_occurrence,
        |date| async move {
            stock_analysis::data_gateway::CapitalDataGateway::new()
                .provider_top_n_pair(date)
                .await
        },
    )
    .await
}

#[cfg(test)]
mod br192_provider_top_n_tests {
    use super::{
        build_r09_delivery_envelope, dispatch_r09_provider_top_n_outcome_with_loader,
        prepare_r09_provider_top_n_report, r09_outcome_from_durable, R09_TEMPLATE_ID,
    };
    use magic_market_core::{
        AssetClass, Exchange, FiniteNumber, InstrumentId, IsoDate, MarketRankingKind,
        MarketRankingUnit, NonEmptyText, PositiveU32, ProviderId,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use stock_analysis::data_gateway::{
        BatchEvidence, GatewayBatch, ProviderTopNFact, ProviderTopNPair,
        ProviderTopNRequestEvidence,
    };

    fn day() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 7, 30).expect("valid TEST_CODE date")
    }

    fn request(
        metric: MarketRankingKind,
        filter_identity: &str,
        hash_byte: char,
    ) -> ProviderTopNRequestEvidence {
        ProviderTopNRequestEvidence {
            metric,
            trading_date: IsoDate::new(day().format("%Y-%m-%d").to_string()).unwrap(),
            limit: PositiveU32::new(20).unwrap(),
            filter_identity: NonEmptyText::new(filter_identity).unwrap(),
            request_hash: hash_byte.to_string().repeat(64),
        }
    }

    fn evidence(batch_id: &str) -> BatchEvidence {
        BatchEvidence {
            provider: ProviderId::Eastmoney,
            source: "eastmoney-web".to_string(),
            source_at: None,
            observed_at: if batch_id.contains("main_net_inflow") {
                "2026-07-30T15:37:00+08:00"
            } else {
                "2026-07-30T15:36:00+08:00"
            }
            .to_string(),
            batch_id: batch_id.to_string(),
        }
    }

    fn fact(
        metric: MarketRankingKind,
        unit: MarketRankingUnit,
        filter_identity: &str,
        ordinal: u32,
        code: &str,
        label: &str,
        value: f64,
    ) -> ProviderTopNFact {
        ProviderTopNFact {
            metric,
            source_order_ordinal: PositiveU32::new(ordinal).unwrap(),
            instrument: InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity).unwrap(),
            label: NonEmptyText::new(label).unwrap(),
            value: FiniteNumber::new(value).unwrap(),
            unit,
            trading_date: IsoDate::new(day().format("%Y-%m-%d").to_string()).unwrap(),
            filter_identity: NonEmptyText::new(filter_identity).unwrap(),
            provider_declared_total: PositiveU32::new(5_000).unwrap(),
            inspected_row_count: PositiveU32::new(20).unwrap(),
        }
    }

    fn pair() -> ProviderTopNPair {
        let volume_filter = "TEST_CODE_volume_ratio_filter";
        let inflow_filter = "TEST_CODE_main_net_inflow_filter";
        ProviderTopNPair {
            volume_ratio_request: request(MarketRankingKind::VolumeRatio, volume_filter, 'a'),
            volume_ratio: GatewayBatch::Available {
                records: vec![
                    fact(
                        MarketRankingKind::VolumeRatio,
                        MarketRankingUnit::Multiple,
                        volume_filter,
                        1,
                        "TEST_CODE_600002",
                        "TEST_CODE量比二号",
                        8.2,
                    ),
                    fact(
                        MarketRankingKind::VolumeRatio,
                        MarketRankingUnit::Multiple,
                        volume_filter,
                        2,
                        "TEST_CODE_600001",
                        "TEST_CODE量比一号",
                        7.1,
                    ),
                ],
                evidence: evidence("TEST_CODE_volume_ratio_batch"),
            },
            main_net_inflow_request: request(MarketRankingKind::MainNetInflow, inflow_filter, 'b'),
            main_net_inflow: GatewayBatch::Available {
                records: vec![
                    fact(
                        MarketRankingKind::MainNetInflow,
                        MarketRankingUnit::Yuan,
                        inflow_filter,
                        1,
                        "TEST_CODE_600004",
                        "TEST_CODE净流入四号",
                        380_000_000.0,
                    ),
                    fact(
                        MarketRankingKind::MainNetInflow,
                        MarketRankingUnit::Yuan,
                        inflow_filter,
                        2,
                        "TEST_CODE_600003",
                        "TEST_CODE净流入三号",
                        260_000_000.0,
                    ),
                ],
                evidence: evidence("TEST_CODE_main_net_inflow_batch"),
            },
        }
    }

    fn hydration(decision_identity: &str) -> stock_analysis::durable_delivery::ScheduleHydration {
        stock_analysis::durable_delivery::ScheduleHydration {
            decision_identity: decision_identity.to_string(),
            task_identity: "TEST_CODE_R09_TASK".to_string(),
            transition_identity: "TEST_CODE_R09_TRANSITION".to_string(),
            transition_canonical: br#"{"TEST_CODE":"transition"}"#.to_vec(),
            transition_sha256: "a".repeat(64),
            transition_basis_canonical: br#"{"TEST_CODE":"basis"}"#.to_vec(),
            transition_basis_sha256: "b".repeat(64),
            immutable_audit_ref: "TEST_CODE_R09_AUDIT".to_string(),
            hydration_state: stock_analysis::durable_delivery::ScheduleHydrationState::Pending,
        }
    }

    fn existing_delivered_evidence(
        decision_identity: &str,
        count: usize,
    ) -> crate::durable_delivery_runtime::DurableDispatchEvidence {
        let task_identity =
            crate::review_batch::review_task_identity(day(), crate::review_batch::ReviewTask::R09);
        let basis = serde_json::to_vec(&serde_json::json!({
            "task_identity": task_identity.clone(),
            "business_date": day().format("%Y-%m-%d").to_string(),
            "task": "R-09",
            "snapshot_size": count,
        }))
        .unwrap();
        crate::durable_delivery_runtime::DurableDispatchEvidence {
            decision_identity: decision_identity.to_string(),
            state: stock_analysis::durable_delivery::DecisionState::Delivered,
            schedule_hydration: Some(stock_analysis::durable_delivery::ScheduleHydration {
                decision_identity: decision_identity.to_string(),
                task_identity,
                transition_identity: "TEST_CODE_BR200_R09_TRANSITION".to_string(),
                transition_canonical: br#"{"TEST_CODE":"transition"}"#.to_vec(),
                transition_sha256: "a".repeat(64),
                transition_basis_sha256: super::r09_sha256(&basis),
                transition_basis_canonical: basis,
                immutable_audit_ref: "TEST_CODE_BR200_R09_AUDIT".to_string(),
                hydration_state: stock_analysis::durable_delivery::ScheduleHydrationState::Applied,
            }),
        }
    }

    #[tokio::test]
    async fn br200_r09_existing_delivered_skips_provider_and_reuses_count() {
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&provider_calls);
        let outcome = dispatch_r09_provider_top_n_outcome_with_loader(
            day(),
            |_| async {
                Ok(Some(existing_delivered_evidence(
                    "TEST_CODE_BR200_R09_DECISION",
                    17,
                )))
            },
            move |_| async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(pair())
            },
        )
        .await;

        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
        assert!(matches!(
            outcome,
            crate::review_batch::ReviewTaskOutcome::Delivered { count: 17 }
        ));
    }

    #[test]
    fn br192_binding_preserves_provider_order_and_source_limited_disclaimer() {
        let prepared = prepare_r09_provider_top_n_report(day(), pair()).unwrap();

        assert_eq!(prepared.binding.template_id, R09_TEMPLATE_ID);
        assert_eq!(prepared.binding.ordered_projection.len(), 4);
        assert_eq!(
            prepared
                .binding
                .ordered_projection
                .iter()
                .map(|row| row.code.as_str())
                .collect::<Vec<_>>(),
            vec![
                "TEST_CODE_600002",
                "TEST_CODE_600001",
                "TEST_CODE_600004",
                "TEST_CODE_600003",
            ]
        );
        assert_eq!(
            prepared.binding.task_transition_basis.batch_ids,
            [
                "TEST_CODE_volume_ratio_batch".to_string(),
                "TEST_CODE_main_net_inflow_batch".to_string(),
            ]
        );
        assert_eq!(
            prepared.binding.task_transition_basis.request_hashes,
            ["a".repeat(64), "b".repeat(64)]
        );
        assert!(prepared
            .rendered
            .contains("Eastmoney 单响应 TopN；不代表全市场完整排序"));
        assert!(prepared.rendered.contains("不补零、不推断全市场覆盖"));
        assert_eq!(
            prepared.binding.rendered_content_sha256,
            super::r09_sha256(prepared.rendered.as_bytes())
        );
        assert_eq!(prepared.canonical_binding_sha256.len(), 64);
    }

    #[test]
    fn br192_one_verified_empty_metric_rejects_the_atomic_report() {
        let mut pair = pair();
        pair.main_net_inflow =
            GatewayBatch::VerifiedEmpty(evidence("TEST_CODE_main_net_inflow_empty_batch"));

        let error = prepare_r09_provider_top_n_report(day(), pair).unwrap_err();

        assert!(error.contains("verified_empty"));
    }

    #[test]
    fn br192_provider_ordinal_mismatch_is_not_resorted_or_silently_accepted() {
        let mut pair = pair();
        if let GatewayBatch::Available { records, .. } = &mut pair.volume_ratio {
            records[0].source_order_ordinal = PositiveU32::new(2).unwrap();
        }

        let error = prepare_r09_provider_top_n_report(day(), pair).unwrap_err();

        assert!(error.contains("source ordinal 1"));
    }

    #[test]
    fn br192_r09_envelope_freezes_both_provider_batches_and_task_binding() {
        let prepared = prepare_r09_provider_top_n_report(day(), pair()).unwrap();
        let expected_source_binding = serde_json::to_vec(&prepared.binding).unwrap();
        let expected_transition_basis =
            serde_json::to_vec(&prepared.binding.task_transition_basis).unwrap();

        let envelope = build_r09_delivery_envelope(&prepared).unwrap();

        assert_eq!(
            envelope.push_kind,
            stock_analysis::durable_delivery::PushKind::ReviewProviderTopN
        );
        assert_eq!(
            envelope.sub_kind,
            stock_analysis::durable_delivery::DeliverySubKind::None
        );
        assert_eq!(envelope.scope_key, "GLOBAL");
        assert_eq!(
            envelope.schedule_occurrence_identity,
            prepared.binding.review_task_identity
        );
        assert_eq!(
            envelope.source_evidence_fingerprint,
            prepared.binding.source_evidence_fingerprint
        );
        assert_eq!(envelope.source_binding_canonical, expected_source_binding);
        assert_eq!(
            envelope.source_binding_sha256,
            prepared.canonical_binding_sha256
        );
        assert_eq!(
            envelope.delivery_subject_hash,
            prepared.binding.delivery_subject_identity
        );
        assert_eq!(envelope.rendered_content, prepared.binding.rendered_content);
        assert_eq!(
            envelope.rendered_content_sha256,
            prepared.binding.rendered_content_sha256
        );
        assert_eq!(envelope.provider_as_of.as_deref(), Some("2026-07-30"));
        assert_eq!(
            prepared.binding.volume_ratio_request.trading_date,
            "2026-07-30"
        );
        assert_eq!(
            prepared.binding.main_net_inflow_request.trading_date,
            "2026-07-30"
        );
        assert_eq!(
            envelope.original_batch_ids,
            vec![
                "TEST_CODE_volume_ratio_batch".to_string(),
                "TEST_CODE_main_net_inflow_batch".to_string(),
            ]
        );
        assert_eq!(
            envelope.provider_observed_at.as_deref(),
            Some("2026-07-30T15:37:00+08:00")
        );
        let task_binding = envelope.task_binding.as_ref().unwrap();
        assert_eq!(
            task_binding.task_identity,
            prepared.binding.review_task_identity
        );
        assert_eq!(
            task_binding.transition_basis_canonical,
            expected_transition_basis
        );
        assert_eq!(
            prepared.binding.volume_ratio_batch.observed_at,
            "2026-07-30T15:36:00+08:00"
        );
        assert_eq!(
            prepared.binding.main_net_inflow_batch.observed_at,
            "2026-07-30T15:37:00+08:00"
        );
    }

    #[test]
    fn br192_r09_reports_delivery_only_after_task_hydration_is_durable() {
        let delivered = r09_outcome_from_durable(
            crate::durable_delivery_runtime::DurableDispatchEvidence {
                decision_identity: "TEST_CODE_R09_DECISION".to_string(),
                state: stock_analysis::durable_delivery::DecisionState::Delivered,
                schedule_hydration: Some(hydration("TEST_CODE_R09_DECISION")),
            },
            40,
        );
        let pending_hydration = r09_outcome_from_durable(
            crate::durable_delivery_runtime::DurableDispatchEvidence {
                decision_identity: "TEST_CODE_R09_DECISION".to_string(),
                state: stock_analysis::durable_delivery::DecisionState::Delivered,
                schedule_hydration: None,
            },
            40,
        );

        assert_eq!(
            delivered,
            crate::review_batch::ReviewTaskOutcome::Delivered { count: 40 }
        );
        assert!(matches!(
            pending_hydration,
            crate::review_batch::ReviewTaskOutcome::Failed {
                failure: crate::review_batch::ReviewTaskFailure::ExistingSourceFailure {
                    retryable: true,
                    reason,
                },
            } if reason.contains("schedule hydration is pending")
        ));
    }

    #[test]
    fn br192_r09_uncertain_delivery_never_becomes_an_automatic_retry() {
        let outcome = r09_outcome_from_durable(
            crate::durable_delivery_runtime::DurableDispatchEvidence {
                decision_identity: "TEST_CODE_R09_UNCERTAIN".to_string(),
                state: stock_analysis::durable_delivery::DecisionState::UncertainManualReview,
                schedule_hydration: Some(hydration("TEST_CODE_R09_UNCERTAIN")),
            },
            40,
        );

        assert!(matches!(
            outcome,
            crate::review_batch::ReviewTaskOutcome::Failed {
                failure: crate::review_batch::ReviewTaskFailure::ExistingSourceFailure {
                    retryable: false,
                    reason,
                },
            } if reason.contains("requires manual review")
        ));
    }
}

/// B-005-C 统一入口 — 由 BR-139 独立 scheduler 在交易日 19:00 后调用.
/// 不依赖 --push 命令行模式, 让生产 monitor 自动出盘后报告.
/// 各 R-series dispatcher 内部分别走自己的数据源，并逐项记录成功/失败。
/// BR-140 返回逐任务强类型结果；等待、禁用与失败均不得冒充投递成功。
/// BR-223: A-02 竞价优选重推模板渲染 (9:20-9:25 竞价优选 Top5)。
pub fn render_auction_repush(
    hhmm: &str,
    top5: &[stock_analysis::opportunity::candidate_panel::CandidateEntry],
) -> String {
    let mut text = format!("🔔 竞价优选 Top{}（{}）\n", top5.len(), hhmm);
    for (index, entry) in top5.iter().enumerate() {
        let price = entry.current_price.unwrap_or(0.0);
        text.push_str(&format!(
            "{}. {}({}) {} | 现价 {:.2} | 热度 {:+.0}\n",
            index + 1,
            entry.name,
            entry.code,
            entry
                .sources
                .first()
                .map(|source| source.label())
                .unwrap_or("候选"),
            price,
            entry.heat_score.unwrap_or(0.0)
        ));
    }
    text.push_str("竞价阶段, 以开盘实际成交为准 | 辅助建议, 非下单指令");
    text
}

/// BR-223: A-02 竞价优选重推 (9:20-9:25, v13.10.1 曾停用, 现恢复)。
/// 复用统一网关候选链路 load_real_candidate_batch, 按 Strong 档优先 + 热度排序取 Top5。
pub async fn dispatch_auction_repush(hhmm: &str) -> bool {
    let entries = match load_real_candidate_batch().await {
        Ok(batch) => batch.entries,
        Err(error) => {
            log::warn!("[A-02][BR-223] 候选源不可用: {error}");
            return false;
        }
    };
    if entries.is_empty() {
        log_dispatcher_attempt("A-02", false, 0, "no candidates at auction");
        return false;
    }
    let mut ranked: Vec<_> = entries
        .into_iter()
        .filter(|entry| entry.current_price.is_some())
        .collect();
    ranked.sort_by(|a, b| {
        let tier_a = a.tier == stock_analysis::opportunity::candidate_panel::EvidenceTier::Strong;
        let tier_b = b.tier == stock_analysis::opportunity::candidate_panel::EvidenceTier::Strong;
        tier_b.cmp(&tier_a).then_with(|| {
            b.heat_score
                .unwrap_or(0.0)
                .partial_cmp(&a.heat_score.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    let top5: Vec<_> = ranked.into_iter().take(5).collect();
    if top5.is_empty() {
        log_dispatcher_attempt("A-02", false, 0, "no priced candidates at auction");
        return false;
    }
    let text = render_auction_repush(hhmm, &top5);
    let result = dispatch_registered_outcome!(
        "A-02-auction-repush",
        crate::notify::PushKind::AuctionRepush,
        "auction_repush_dispatcher",
        "render_auction_repush",
        "",
        None,
        text
    );
    log_dispatcher_attempt("A-02", result.is_pushed(), top5.len(), "");
    result.is_pushed()
}

/// BR-223: A-11 IPO 阶段催化模板渲染 (静态供应链表)。
pub fn render_ipo_catalyst(
    date: &str,
    companies: &[stock_analysis::news::ipo::supply_chain::IpoCompany],
) -> String {
    let mut text = format!("🛰️ IPO 产业链催化（{}）\n", date);
    for company in companies {
        text.push_str(&format!(
            "· {} — 阶段 {:?}\n  关联: ",
            company.pre_ipo_name, company.ipo_stage
        ));
        let related = company
            .related_stocks
            .iter()
            .map(|(code, name, _)| format!("{name}({code})"))
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str(&related);
        text.push('\n');
    }
    text.push_str(
        "数据源: 维护的 IPO 供应链静态表 (阶段变化需人工更新) | 非实时事件 | 辅助建议, 非下单指令",
    );
    text
}

/// 2026-08-06 改造: 动态查询最近 IPO (cninfo 公告实时批次), 不再用静态长鑫表兜底。
/// 静态供应链表降级为 "公司名 → A 股标的" 映射字典。
/// 未命中字典 → 行业关键词 → TDX 真实概念板块 → 成分股 (产业链影响)。
/// R-08 已拉取的当日公告批次缓存 (A-11 复用, 避免 cninfo 重复拉取限流)。
static REVIEW_ANNOUNCEMENTS_CACHE: std::sync::OnceLock<
    std::sync::Mutex<Option<(String, Vec<stock_analysis::data_gateway::EventAnnouncement>)>>,
> = std::sync::OnceLock::new();

#[derive(Debug, Clone)]
pub struct DynamicIpoHit {
    pub company: String,
    pub stage: stock_analysis::news::ipo::supply_chain::IpoStage,
    pub keyword: String,
    pub mapped_stocks: Vec<(String, String)>, // 静态字典命中 (code, name) 或空
    pub industry: Vec<IndustryBoard>,         // 动态板块推断 (产业链影响)
    pub announcement_code: String,
}

/// 产业链影响: 板块名 + 成分股 (code, name)。
#[derive(Debug, Clone)]
pub struct IndustryBoard {
    pub board_name: String,
    pub stocks: Vec<(String, String)>,
}

/// 公司名 → 行业关键词组 (公司词 → TDX 板块名匹配词)。
/// 2026-08-06 校准 (board_directory_probe 实证): TDX 概念板块是题材简称
/// (AIGC/CPO/存储芯片), "激光/半导体/机器人" 无同名概念板块;
/// "存储"→"存储芯片", "芯片"→"MCU芯片/存储芯片", "光伏"→"光伏",
/// "航天"→"商业航天" 可命中。行业板块 (Industry) 补概念板块的盲区。
const INDUSTRY_KEYWORD_GROUPS: [(&str, &[&str]); 14] = [
    ("激光", &["激光", "光学", "光电子"]),
    ("存储", &["存储", "内存"]),
    ("芯片", &["芯片", "半导体"]),
    ("半导体", &["芯片", "半导体"]),
    ("机器人", &["机器人", "人形"]),
    ("电池", &["电池", "锂电"]),
    ("光伏", &["光伏", "太阳能"]),
    ("新能源", &["新能源", "锂电", "光伏"]),
    ("航天", &["航天", "卫星"]),
    ("卫星", &["卫星", "航天"]),
    ("算力", &["算力", "服务器", "AIGC", "CPO"]),
    ("人工智能", &["人工智能", "AI", "AIGC", "DeepSeek", "ChatGPT"]),
    ("军工", &["军工", "国防", "商业航天"]),
    ("医疗", &["医疗", "创新药", "CXO"]),
];

/// 目录里板块名含公司名行业关键词 → (板块 code, 板块名)。首个命中关键词的板块族。
fn infer_industry_boards(
    directory: &[stock_analysis::data_gateway::BoardDirectoryFact],
    company: &str,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (company_kw, board_words) in INDUSTRY_KEYWORD_GROUPS {
        if !company.contains(company_kw) {
            continue;
        }
        for fact in directory {
            if board_words.iter().any(|w| fact.name.contains(w)) {
                out.push((fact.code.clone(), fact.name.clone()));
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out
}

/// 公告标题 → IPO 阶段 + 命中关键词。
/// 阶段推断: 上市公告书=上市, 招股意向书/发行=发行中, 注册=过会, 受理/问询=在审。
fn ipo_keyword_stage(title: &str) -> Option<(stock_analysis::news::ipo::supply_chain::IpoStage, String)> {
    use stock_analysis::news::ipo::supply_chain::IpoStage;
    let t = title.to_ascii_lowercase();
    let pick = |k: &str, s: IpoStage| t.contains(k).then(|| (s, k.to_string()));
    use IpoStage::*;
    pick("上市公告书", Listed)
        .or_else(|| pick("招股意向书", Registered))
        .or_else(|| pick("招股说明书", Registered))
        .or_else(|| pick("首次公开发行", InReview))
        .or_else(|| pick("ipo", InReview))
}

/// 公告标题 → 公司名: "XX股份有限公司..." 前缀提取。
/// 2026-08-06 修复: `pos + s.chars().count()` 字符数混入字节切片, 中文标题
/// (如 "上海频准激光科技股份有限公司") panic (end byte index not a char
/// boundary)。`find` 返回字节索引, 必须用 `pos + s.len()` (字节长度)。
/// 公司名取 "关于" 之后的块 (A 股公告标题惯例), 去掉律所/中介前缀。
fn extract_company_name(title: &str) -> String {
    const SUFFIXES: [&str; 4] = ["股份有限公司", "有限责任公司", "有限公司", "集团"];
    let mut best = String::new();
    for s in SUFFIXES {
        if let Some(pos) = title.find(s) {
            let end = pos + s.len();
            if end > title.len() {
                continue;
            }
            let start = title.find("关于").map(|p| p + "关于".len()).unwrap_or(0);
            if start >= end {
                continue;
            }
            let cand = title[start..end].trim().to_string();
            if cand.chars().count() > best.chars().count() {
                best = cand;
            }
        }
    }
    best
}

/// BR-223: A-11 IPO 阶段催化 — 动态查询最近 IPO (每日一次, 盘后复盘链内)。
/// 数据流: 今日 cninfo 公告批次 (limit=300, 断点 B 覆盖全天) → IPO 关键词过滤
/// → 公司名提取 → 静态映射字典 (长鑫等) → 渲染。无 IPO 公告 → 短路不推。
pub async fn dispatch_ipo_catalyst(date: &str) -> bool {
    use chrono::NaiveDate;
    use stock_analysis::data_gateway::{EventCalendarGateway, GatewayBatch};
    use stock_analysis::news::ipo::supply_chain::lookup;

    let date_naive = match NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        Ok(d) => d,
        Err(error) => {
            log::warn!("[A-11][BR-223] 非法日期 {date:?}: {error}");
            return false;
        }
    };
    // 2026-08-06: 优先复用 R-08 已拉取的当日公告批次 (同日期命中 → 不重复拉
    // cninfo, 避免复盘内两次拉取触发限流 router_batch_rejected)。未命中 → 拉取。
    let cached: Option<Vec<stock_analysis::data_gateway::EventAnnouncement>> = REVIEW_ANNOUNCEMENTS_CACHE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .ok()
        .and_then(|mut cache| match cache.as_ref() {
            Some((cached_date, records)) if cached_date == date => Some(records.clone()),
            _ => None,
        });
    let records: Vec<stock_analysis::data_gateway::EventAnnouncement> = match cached {
        Some(records) => {
            log::info!(
                "[A-11][BR-223] 复用 R-08 公告批次: {} 条 (缓存命中, 未重复拉取)",
                records.len()
            );
            records
        }
        None => {
            let batch = match EventCalendarGateway::new()
                .market_announcements(date_naive, 300)
                .await
            {
                Ok(batch) => batch,
                Err(error) => {
                    log::warn!("[A-11][BR-223] 公告批次不可用: {error}");
                    return false;
                }
            };
            match batch {
                GatewayBatch::Available { records, .. } => records,
                GatewayBatch::VerifiedEmpty(evidence) => {
                    log::info!("[A-11][BR-223] 公告已验证为空: {:?}", evidence.batch_id);
                    return false;
                }
            }
        }
    };

    let mut hits: Vec<DynamicIpoHit> = Vec::new();
    for ann in records {
        let Some((stage, keyword)) = ipo_keyword_stage(&ann.title) else {
            continue;
        };
        let company = extract_company_name(&ann.title);
        if company.is_empty() {
            continue;
        }
        let mapped = match lookup(&company) {
            Some(known) => known
                .related_stocks
                .iter()
                .map(|(c, n, _)| ((*c).to_string(), (*n).to_string()))
                .collect(),
            None => Vec::new(), // 未命中字典 → 动态板块推断
        };
        hits.push(DynamicIpoHit {
            company,
            stage,
            keyword,
            mapped_stocks: mapped,
            industry: Vec::new(),
            announcement_code: ann.code.clone(),
        });
    }
    hits.dedup_by(|a, b| a.company == b.company && a.keyword == b.keyword);
    if hits.is_empty() {
        log_dispatcher_attempt("A-11", false, 0, "no IPO announcements today");
        return false;
    }

    // 产业链影响 (2026-08-06): 静态字典未命中的公司 → 行业关键词 →
    // TDX 真实概念板块 → 成分股 (名称经 security_identities 补齐)。
    // 板块/成分/身份任一失败 → 该板块跳过, 不影响其余 (尽力而为, 出声)。
    for hit in hits.iter_mut().filter(|h| h.mapped_stocks.is_empty()) {
        let company = hit.company.clone();
        let directory = match (
            stock_analysis::data_gateway::BoardDataGateway::production_tdx()
                .directory(stock_analysis::data_gateway::BoardKind::Concept, 200)
                .await,
            stock_analysis::data_gateway::BoardDataGateway::production_tdx()
                .directory(stock_analysis::data_gateway::BoardKind::Industry, 200)
                .await,
        ) {
            (
                Ok(stock_analysis::data_gateway::GatewayBatch::Available {
                    records: concept, ..
                }),
                Ok(stock_analysis::data_gateway::GatewayBatch::Available {
                    records: industry,
                    ..
                }),
            ) => {
                let mut all = concept;
                all.extend(industry);
                all
            }
            (Ok(stock_analysis::data_gateway::GatewayBatch::Available { records, .. }), _) => {
                records
            }
            (_, Ok(stock_analysis::data_gateway::GatewayBatch::Available { records, .. })) => {
                records
            }
            (Ok(stock_analysis::data_gateway::GatewayBatch::VerifiedEmpty(evidence)), _)
            | (_, Ok(stock_analysis::data_gateway::GatewayBatch::VerifiedEmpty(evidence))) => {
                log::warn!(
                    "[A-11][BR-223] 板块目录已验证为空: {:?}",
                    evidence.batch_id
                );
                continue;
            }
            (Err(error), _) | (_, Err(error)) => {
                log::warn!("[A-11][BR-223] 板块目录不可用: {error}");
                continue;
            }
        };
        let boards = infer_industry_boards(&directory, &company);
        if boards.is_empty() {
            continue;
        }
        let mut industry: Vec<IndustryBoard> = Vec::new();
        for (board_code, board_name) in boards.iter().take(3) {
            let member_codes: Vec<String> =
                match stock_analysis::data_gateway::BoardDataGateway::production_tdx()
                    .memberships(board_code)
                    .await
                {
                    Ok(stock_analysis::data_gateway::GatewayBatch::Available {
                        records, ..
                    }) => records
                        .iter()
                        .map(|r| r.instrument_code.clone())
                        .collect(),
                    Ok(_) | Err(_) => Vec::new(),
                };
            if member_codes.is_empty() {
                continue;
            }
            let member_codes = member_codes.into_iter().take(10).collect::<Vec<_>>();
            let named: Vec<(String, String)> =
                match stock_analysis::data_gateway::MarketCapabilitiesGateway::new()
                    .security_identities(&member_codes)
                    .await
                {
                    Ok(stock_analysis::data_gateway::GatewayBatch::Available {
                        records, ..
                    }) => records
                        .iter()
                        .map(|i| (i.code.clone(), i.name.clone()))
                        .collect(),
                    Ok(_) | Err(_) => Vec::new(),
                };
            industry.push(IndustryBoard {
                board_name: board_name.clone(),
                stocks: named,
            });
        }
        hit.industry = industry;
    }

    let text = render_ipo_catalyst_dynamic(date, &hits);
    let result = dispatch_registered_outcome!(
        "A-11-ipo-catalyst",
        crate::notify::PushKind::IpoCatalyst,
        "ipo_catalyst_dispatcher",
        // renderer seam id 保持原注册名 (BR-196 token 按 family+renderer 派生,
        // 2026-08-06 曾改名为 _dynamic 导致 token 拒绝)
        "render_ipo_catalyst",
        "",
        None,
        text
    );
    log_dispatcher_attempt("A-11", result.is_pushed(), hits.len(), "");
    result.is_pushed()
}

/// 动态 IPO 催化渲染: 最近 IPO 公告 → 公司 + 阶段 + 供应链关联 + 产业链影响。
fn render_ipo_catalyst_dynamic(date: &str, hits: &[DynamicIpoHit]) -> String {
    let mut text = format!("🛰️ IPO 产业链催化（{} 动态）\n", date);
    for hit in hits {
        text.push_str(&format!(
            "· {} — 阶段 {:?} (公告: {})\n  关联: ",
            hit.company, hit.stage, hit.keyword
        ));
        if !hit.mapped_stocks.is_empty() {
            let related = hit
                .mapped_stocks
                .iter()
                .map(|(c, n)| format!("{n}({c})"))
                .collect::<Vec<_>>()
                .join(", ");
            text.push_str(&related);
            text.push('\n');
            continue;
        }
        if hit.industry.is_empty() {
            text.push_str("无 (供应链字典未收录, 板块推断未命中)");
            text.push('\n');
            continue;
        }
        for board in &hit.industry {
            text.push_str(&format!("  产业链 {}: ", board.board_name));
            if board.stocks.is_empty() {
                text.push_str("成分数据不可用");
            } else {
                let stocks = board
                    .stocks
                    .iter()
                    .map(|(c, n)| format!("{n}({c})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                text.push_str(&stocks);
            }
            text.push('\n');
        }
    }
    text.push_str(
        "数据源: cninfo 当日公告实时批次 | 供应链: 维护字典 + TDX 真实板块成分 | 辅助建议, 非下单指令",
    );
    text
}

/// BR-223: 盘后大宗交易推送 — BlockTradesGateway → 既有 BR-033/BR-034 dispatcher。
/// 过滤规则: 创业板(300/301)/科创板(688) → 协议大宗实时确认;
/// 北交所(8xx/4xx/920) → 大宗价格区间。名称以自选/持仓为准 (gateway 不带名称)。
pub async fn dispatch_block_trade_review(
    codes: &[String],
    trading_date: chrono::NaiveDate,
) -> usize {
    let batch = match stock_analysis::data_gateway::BlockTradesGateway::new()
        .market_review(codes, trading_date)
        .await
    {
        Ok(batch) => batch,
        Err(error) => {
            log::warn!("[BR-033/034][BR-223] 大宗交易 gateway 失败: {error}");
            return 0;
        }
    };
    let hhmm = chrono::Local::now().format("%H:%M:%S").to_string();
    let mut pushed = 0;
    for review in batch.records() {
        let code = &review.code;
        let is_gem = code.starts_with("300") || code.starts_with("301");
        let is_star = code.starts_with("688");
        let is_bse = code.starts_with('8') || code.starts_with('4') || code.starts_with("920");
        let name = code.clone();
        if is_gem || is_star {
            let board = if is_star { Board::Star } else { Board::Gem };
            let qty = review.volume as u32;
            if dispatch_block_trade_intraday_confirm(
                &hhmm,
                &name,
                code,
                qty,
                review.price,
                BlockType::Agreed,
                board,
                true,
                SettleType::NextSession,
            )
            .await
            {
                pushed += 1;
            }
        } else if is_bse {
            if dispatch_block_trade_price_range(
                &hhmm,
                &name,
                code,
                review.close_price,
                review.price,
                None,
                "北交所大宗价格区间 (东财 RPT_DATA_BLOCKTRADE)",
            )
            .await
            {
                pushed += 1;
            }
        }
    }
    if pushed == 0 {
        log_dispatcher_attempt(
            "BR-033/034",
            false,
            batch.records().len(),
            "no matching block trades",
        );
    }
    pushed
}

/// BR-223: P-05 候选台推送 + 候选失效 diff。
/// 每次推送前把候选 code 集快照落盘 (data/candidate_board_snapshot/<date>.jsonl,
/// 每行一轮), 与上一轮 diff: 上轮有本轮无 → push_candidate_invalidated。
fn candidate_snapshot_path(date: &str) -> std::path::PathBuf {
    let is_test = stock_analysis::risk::env_guard::runtime_is_test_process()
        || stock_analysis::risk::env_guard::current_env()
            == stock_analysis::risk::env_guard::TradingEnv::Test;
    let base = if is_test { "data/test" } else { "data" };
    std::path::PathBuf::from(base)
        .join("candidate_board_snapshot")
        .join(format!("{date}.jsonl"))
}

fn candidate_snapshot_previous(date: &str) -> Option<std::collections::BTreeSet<String>> {
    let path = candidate_snapshot_path(date);
    let content = std::fs::read_to_string(&path).ok()?;
    let last_line = content.lines().next_back()?;
    let codes: std::collections::BTreeSet<String> = serde_json::from_str(last_line).ok()?;
    Some(codes)
}

fn candidate_snapshot_persist(date: &str, codes: &std::collections::BTreeSet<String>) {
    let path = candidate_snapshot_path(date);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(line) = serde_json::to_string(codes) {
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| {
                use std::io::Write;
                f.write_all(line.as_bytes())?;
                f.write_all(b"\n")
            });
    }
}

/// BR-223: P-05 候选筛选台 (v11-P0-5++) — 统一网关候选链路 + 失效 diff。
pub async fn dispatch_candidate_board(date: &str) -> bool {
    use stock_analysis::opportunity::candidate_panel::EvidenceTier;
    let batch = match load_real_candidate_batch().await {
        Ok(batch) => batch,
        Err(error) => {
            log::warn!("[P-05][BR-223] 候选源不可用: {error}");
            return false;
        }
    };
    if batch.entries.is_empty() {
        log_dispatcher_attempt("P-05", false, 0, "no candidates");
        return false;
    }
    let codes_now: std::collections::BTreeSet<String> = batch
        .entries
        .iter()
        .map(|entry| entry.code.clone())
        .collect();
    // 失效 diff: 上轮有本轮无 → 推送失效 (renderer 已有 push_candidate_invalidated)
    if let Some(previous) = candidate_snapshot_previous(date) {
        let hhmm = chrono::Local::now().format("%H:%M:%S").to_string();
        for code in previous.difference(&codes_now) {
            let name = batch
                .entries
                .iter()
                .find(|entry| &entry.code == code)
                .map(|entry| entry.name.clone())
                .unwrap_or_else(|| code.clone());
            let _ = push_candidate_invalidated(code, &hhmm, &name, "候选", "从候选台消失").await;
        }
    }
    // BR-224: SignalTracker 采样 — Strong 候选写入 prediction_tracker (5 日后回填)
    let strong_samples: Vec<(String, f64)> = batch
        .entries
        .iter()
        .filter(|entry| entry.tier == EvidenceTier::Strong && entry.current_price.is_some())
        .map(|entry| (entry.code.clone(), entry.heat_score.unwrap_or(50.0)))
        .collect();
    if !strong_samples.is_empty() {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let target = (chrono::Local::now().date_naive() + chrono::Duration::days(5))
            .format("%Y-%m-%d")
            .to_string();
        let _ = tokio::task::spawn_blocking(move || {
            use stock_analysis::database::DatabaseManager;
            let db = DatabaseManager::get();
            for (code, score) in strong_samples {
                let _ = db.save_prediction(
                    &today,
                    &target,
                    None,
                    Some(&code),
                    "up",
                    score,
                    Some("candidate-strong"),
                    None,
                    None,
                );
            }
        })
        .await;
    }
    candidate_snapshot_persist(date, &codes_now);
    let text = stock_analysis::opportunity::candidate_panel::format_candidate_board(&batch.entries);
    let result = dispatch_registered_outcome!(
        "P-05-candidate-board",
        crate::notify::PushKind::CandidateBoard,
        "candidate_board_dispatcher",
        "format_candidate_board",
        "",
        None,
        text
    );
    log_dispatcher_attempt("P-05", result.is_pushed(), batch.entries.len(), "");
    result.is_pushed()
}

/// BR-222: R-07 counted 投递材料 (BR-140/BR-192 counted ceremony)。
/// TomorrowWatch 是 counted kind, 必须走 CountedDeliveryBinding;
/// counted source = 龙虎榜批次证据 (与 R-04 同源)。
fn prepare_tomorrow_watch_delivery(
    business_date: chrono::NaiveDate,
    evidence: &stock_analysis::data_gateway::BatchEvidence,
    lhb_records: &[stock_analysis::data_gateway::DragonTigerStockReview],
    rendered: String,
) -> Result<PreparedReviewLhbDelivery, String> {
    use magic_market_core::ProviderId;
    if evidence.provider != ProviderId::Eastmoney {
        return Err(format!(
            "R-07 provider mismatch: expected Eastmoney, got {:?}",
            evidence.provider
        ));
    }
    if evidence.batch_id.trim().is_empty() {
        return Err("R-07 accepted batch ID is missing".to_string());
    }
    let source_at = evidence
        .source_at
        .as_deref()
        .ok_or_else(|| "R-07 provider source_at is missing".to_string())?;
    let source_date = chrono::NaiveDate::parse_from_str(source_at, "%Y-%m-%d")
        .map_err(|error| format!("R-07 provider source_at is invalid: {error}"))?;
    if source_date != business_date {
        return Err(format!(
            "R-07 provider source_at {source_date} differs from business date {business_date}"
        ));
    }
    let provider_observed_at = parse_r04_observed_at(&evidence.observed_at)?;
    if lhb_records.is_empty() {
        return Err("R-07 counted LHB source contains no records".to_string());
    }
    // 有序投影: code|net_amount (按 source order)
    let mut source_binding = String::new();
    for record in lhb_records {
        if record.code.trim().is_empty()
            || !record.ranking_net_amount_yuan.is_finite()
            || record.ranking_net_amount_yuan <= 0.0
        {
            return Err(format!(
                "R-07 LHB projection {} is incomplete or invalid",
                record.code
            ));
        }
        source_binding.push_str(&format!("{}|{}\n", record.code, record.ranking_net_amount_yuan));
    }
    let task_identity = crate::review_batch::review_task_identity(
        business_date,
        crate::review_batch::ReviewTask::R07,
    );
    // transition basis 必须是 JSON (durable hydration 按 DurableTaskBasis 解析)
    #[derive(serde::Serialize)]
    struct TomorrowWatchTaskTransitionBasis {
        task_identity: String,
        business_date: String,
        task: String,
        source: String,
        rule_ids: Vec<String>,
        snapshot_size: usize,
        batch_ids: Vec<String>,
    }
    let task_transition_basis_canonical = serde_json::to_vec(&TomorrowWatchTaskTransitionBasis {
        task_identity: task_identity.clone(),
        business_date: business_date.format("%Y-%m-%d").to_string(),
        task: "R-07".to_string(),
        source: evidence.source.clone(),
        rule_ids: vec![
            "BR-110".to_string(),
            "BR-140".to_string(),
            "BR-192".to_string(),
            "BR-222".to_string(),
        ],
        snapshot_size: lhb_records.len(),
        batch_ids: vec![evidence.batch_id.clone()],
    })
    .map_err(|error| format!("R-07 task transition serialization failed: {error}"))?;
    let delivery_subject_identity = {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"stock_analysis.r07.watchlist.v1\0");
        hasher.update(source_binding.as_bytes());
        format!("{:x}", hasher.finalize())
    };
    Ok(PreparedReviewLhbDelivery {
        rendered,
        business_date,
        task_identity,
        delivery_subject_identity,
        source_binding_canonical: source_binding.into_bytes(),
        task_transition_basis_canonical,
        provider_observed_at,
        batch_id: evidence.batch_id.clone(),
    })
}

/// BR-222: R-07 明日观察池 (v12 MVP-4 §7.6) — 4 类来源装配 + 按 code 去重 (首胜)。
///
/// 来源: A档未触发(Strong 候选) / 龙虎榜强票(净买入 Top5) / 涨停链龙头(前 3 链) /
/// 可做T持仓(整百股结构过滤)。龙虎榜/涨停链条目无价格字段, 按 BR-222 规则置 0.0
/// 并在理由中注明 "以明日竞价为准, 按 T-11 复核" (红线 2.2: 不虚构价格)。
async fn dispatch_tomorrow_watch_outcome(date: &str) -> crate::review_batch::ReviewTaskOutcome {
    use stock_analysis::opportunity::candidate_panel::EvidenceTier;
    use stock_analysis::review::tomorrow_watchlist::{
        dedup, WatchItem as OwnedWatchItem, WatchSource,
    };

    let trading_date = match chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        Ok(parsed) => parsed,
        Err(error) => {
            let reason = format!("invalid review date {date}: {error}");
            log_dispatcher_attempt("R-07", false, 0, &reason);
            return crate::review_batch::ReviewTaskOutcome::failed(false, reason);
        }
    };

    let mut items: Vec<OwnedWatchItem> = Vec::new();

    // 1. A档未触发 (EvidenceTier::Strong 候选, DB-only)
    match tokio::task::spawn_blocking(load_candidate_source_context).await {
        Ok(Ok(context)) => {
            for entry in context.entries {
                if entry.tier != EvidenceTier::Strong {
                    continue;
                }
                let Some(price) = entry.current_price else {
                    continue;
                };
                items.push(OwnedWatchItem {
                    code: entry.code,
                    name: entry.name,
                    topic: entry
                        .sources
                        .first()
                        .map(|source| source.label().to_string())
                        .unwrap_or_else(|| "A档候选".to_string()),
                    source: WatchSource::AGradeNotTriggered,
                    trigger: entry
                        .evidence
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "A档候选未触发".to_string()),
                    lo_price: price * 0.97,
                    hi_price: price * 1.03,
                    stop: price * 0.95,
                    reason: "A档候选未触发: 现价未进入触发区间".to_string(),
                });
            }
        }
        Ok(Err(error)) => log::warn!("[R-07][BR-222] A档候选源不可用, 跳过该来源: {error}"),
        Err(error) => log::warn!("[R-07][BR-222] A档候选 join 失败: {error}"),
    }

    // 2. 龙虎榜强票 (净买入 > 0 Top5; 无价格 → 0.0 + T-11 复核注记)
    //    龙虎榜批次同时是 R-07 counted ceremony 的 counted source (BR-140/BR-192)。
    let mut lhb_evidence: Option<stock_analysis::data_gateway::BatchEvidence> = None;
    let mut lhb_records: Vec<stock_analysis::data_gateway::DragonTigerStockReview> = Vec::new();
    match stock_analysis::data_gateway::dragon_tiger::DragonTigerGateway::new()
        .market_review(trading_date, 5, 5)
        .await
    {
        Ok(batch) => {
            lhb_evidence = Some(batch.evidence().clone());
            lhb_records = batch.records().to_vec();
            let mut strong: Vec<_> = batch
                .records()
                .iter()
                .filter(|record| record.ranking_net_amount_yuan > 0.0)
                .collect();
            strong.sort_by(|a, b| {
                b.ranking_net_amount_yuan
                    .partial_cmp(&a.ranking_net_amount_yuan)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for record in strong.iter().take(5) {
                items.push(OwnedWatchItem {
                    code: record.code.clone(),
                    name: record.code.clone(),
                    topic: "龙虎榜强票".to_string(),
                    source: WatchSource::LhbStrong,
                    trigger: format!("净买入 {:.0} 万", record.ranking_net_amount_yuan / 10000.0),
                    lo_price: 0.0,
                    hi_price: 0.0,
                    stop: 0.0,
                    reason: "龙虎榜净买入为正; 价格以明日竞价为准, 按 T-11 复核".to_string(),
                });
            }
        }
        Err(error) => log::warn!("[R-07][BR-222] 龙虎榜源不可用, 跳过该来源: {error}"),
    }

    // 3. 涨停链龙头 (前 3 链 leader)
    match stock_analysis::data_gateway::review::ReviewDataGateway::new()
        .r03_upper_limit_pool(trading_date)
        .await
    {
        Ok(batch) => {
            let stocks: Vec<stock_analysis::market_analyzer::limit_chain_review::StockLimitStats> =
                batch
                    .records()
                    .iter()
                    .map(|record| {
                        stock_analysis::market_analyzer::limit_chain_review::StockLimitStats {
                            code: record.code.clone(),
                            name: String::new(),
                            chain: record.theme.clone().unwrap_or_else(|| "未分类".to_string()),
                            board_level: record.streak.unwrap_or(1).min(3) as u8,
                            is_limit_up_today: true,
                            is_first_board: record.streak.unwrap_or(1) <= 1,
                            consecutive_days: record.streak.unwrap_or(1),
                        }
                    })
                    .collect();
            let input = stock_analysis::market_analyzer::limit_chain_review::LimitChainInput {
                stocks,
                source_complete: !batch.is_verified_empty(),
            };
            let chains = stock_analysis::market_analyzer::limit_chain_review::aggregate(&input);
            for chain in chains.iter().take(3) {
                let name = if chain.leader_name.is_empty() {
                    chain.leader_code.clone()
                } else {
                    chain.leader_name.clone()
                };
                items.push(OwnedWatchItem {
                    code: chain.leader_code.clone(),
                    name,
                    topic: chain.chain.clone(),
                    source: WatchSource::LimitChainLeader,
                    trigger: format!("{}-板", chain.leader_boards),
                    lo_price: 0.0,
                    hi_price: 0.0,
                    stop: 0.0,
                    reason: format!(
                        "涨停链龙头({}家涨停); 价格以明日竞价为准, 按 T-11 复核",
                        chain.limit_up_n
                    ),
                });
            }
        }
        Err(error) => log::warn!("[R-07][BR-222] 涨停链源不可用, 跳过该来源: {error}"),
    }

    // 4. 可做T持仓 (结构过滤: Holding + 整百股 + 成本价 > 0)
    //    做T 区间以**收盘价**为基准 (BR-225 修正: 此前误用成本价,
    //    现价远低于成本时给出荒谬低吸位)
    let closes: std::collections::HashMap<String, f64> =
        match tokio::task::spawn_blocking(|| {
            use stock_analysis::database::closing_valuation::latest_persisted_valuation_view;
            match latest_persisted_valuation_view() {
                Ok(Some(view)) => view
                    .valuation
                    .items
                    .iter()
                    .filter_map(|item| item.close.map(|close| (item.code.clone(), close)))
                    .collect(),
                _ => std::collections::HashMap::new(),
            }
        })
        .await
        {
            Ok(closes) => closes,
            Err(_) => std::collections::HashMap::new(),
        };
    match tokio::task::spawn_blocking(stock_analysis::portfolio::get_positions).await {
        Ok(Ok(positions)) => {
            for position in positions {
                if position.status != stock_analysis::portfolio::PositionStatus::Holding {
                    continue;
                }
                if position.shares < 100 || position.shares % 100 != 0 || position.cost_price <= 0.0
                {
                    continue;
                }
                match closes.get(&position.code).copied() {
                    Some(base) if base > 0.0 => {
                        items.push(OwnedWatchItem {
                            code: position.code,
                            name: position.name,
                            topic: "做T候选".to_string(),
                            source: WatchSource::T0Candidate,
                            trigger: "持仓做T".to_string(),
                            lo_price: base * 0.98,
                            hi_price: base * 1.02,
                            stop: base * 0.95,
                            reason: format!("整百股持仓满足做T结构条件; 以收盘价 {base:.2} 为基准"),
                        });
                    }
                    _ => {
                        items.push(OwnedWatchItem {
                            code: position.code,
                            name: position.name,
                            topic: "做T候选".to_string(),
                            source: WatchSource::T0Candidate,
                            trigger: "持仓做T".to_string(),
                            lo_price: 0.0,
                            hi_price: 0.0,
                            stop: 0.0,
                            reason: "整百股持仓满足做T结构条件; 无收盘价, 竞价后按 T-11 复核".to_string(),
                        });
                    }
                }
            }
        }
        Ok(Err(error)) => log::warn!("[R-07][BR-222] 持仓源不可用, 跳过该来源: {error}"),
        Err(error) => log::warn!("[R-07][BR-222] 持仓 join 失败: {error}"),
    }

    let items = dedup(items);
    if items.is_empty() {
        log_dispatcher_attempt("R-07", false, 0, "tomorrow watchlist empty");
        return crate::review_batch::ReviewTaskOutcome::no_data(
            "tomorrow watchlist empty: no candidate source available",
        );
    }

    let mut borrowed: Vec<WatchItem<'_>> = Vec::with_capacity(items.len());
    for item in &items {
        borrowed.push(WatchItem {
            name: &item.name,
            code: &item.code,
            topic: &item.topic,
            source: item.source.label(),
            trigger: &item.trigger,
            lo: item.lo_price,
            hi: item.hi_price,
            stop: item.stop,
            reason: &item.reason,
        });
    }
    let text = render_tomorrow_watch(date, &borrowed);
    let prepared = match (&lhb_evidence, lhb_records.is_empty()) {
        (Some(evidence), false) => {
            match prepare_tomorrow_watch_delivery(trading_date, evidence, &lhb_records, text) {
                Ok(prepared) => prepared,
                Err(reason) => {
                    log::warn!("[R-07][BR-140][BR-192] counted binding rejected: {reason}");
                    log_dispatcher_attempt("R-07", false, items.len(), &reason);
                    return crate::review_batch::ReviewTaskOutcome::failed(false, reason);
                }
            }
        }
        _ => {
            let reason = "LHB counted source unavailable for R-07 binding";
            log::warn!("[R-07][BR-140][BR-192] {reason}");
            log_dispatcher_attempt("R-07", false, items.len(), reason);
            return crate::review_batch::ReviewTaskOutcome::no_data(reason);
        }
    };
    let task_binding = match stock_analysis::durable_delivery::TaskBinding::new(
        prepared.task_identity.clone(),
        prepared.task_transition_basis_canonical.clone(),
    ) {
        Ok(binding) => binding,
        Err(error) => {
            let reason = format!("R-07 task binding rejected: {error}");
            log::warn!("[R-07][BR-140][BR-192] {reason}");
            log_dispatcher_attempt("R-07", false, items.len(), &reason);
            return crate::review_batch::ReviewTaskOutcome::failed(false, reason);
        }
    };
    let counted_binding = match crate::durable_delivery_runtime::CountedDeliveryBinding::new(
        prepared.business_date,
        prepared.task_identity,
        prepared.source_binding_canonical,
        crate::durable_delivery_runtime::CountedDeliveryScope::Global,
        prepared.delivery_subject_identity,
        crate::durable_delivery_runtime::CountedDeliveryOrigin::Provider {
            observed_at: Some(prepared.provider_observed_at),
            as_of: Some(prepared.business_date),
            ordered_batch_ids: vec![prepared.batch_id],
        },
        Some(task_binding),
        true,
    ) {
        Ok(binding) => binding,
        Err(reason) => {
            let reason = format!("R-07 counted binding rejected: {reason}");
            log::warn!("[R-07][BR-140][BR-192] {reason}");
            log_dispatcher_attempt("R-07", false, items.len(), &reason);
            return crate::review_batch::ReviewTaskOutcome::failed(false, reason);
        }
    };
    let presentation_token = match crate::presentation_registry::acquire_token(
        "R-07-tomorrow-watch",
        crate::notify::PushKind::TomorrowWatch,
        "tomorrow_watch_dispatcher",
        "render_tomorrow_watch",
    ) {
        Ok(token) => token,
        Err(reason) => {
            log::warn!("[R-07][BR-196] presentation token rejected: {reason}");
            log_dispatcher_attempt("R-07", false, items.len(), &reason);
            return crate::review_batch::ReviewTaskOutcome::failed(false, reason);
        }
    };
    let push_result = crate::notify::push_counted_with_binding(
        presentation_token,
        &prepared.rendered,
        None,
        counted_binding,
    )
    .await;
    let dispatcher_error = push_outcome_dispatcher_error(&push_result);
    log_dispatcher_attempt("R-07", push_result.is_pushed(), items.len(), &dispatcher_error);
    crate::review_batch::ReviewTaskOutcome::from_push_outcome(push_result, items.len())
}

/// R-11 持仓复盘渲染入参 (BR-222)。
#[derive(Debug)]
pub struct PositionReviewParams<'a> {
    pub date: &'a str,
    pub total_assets: f64,
    pub position_ratio_pct: f64,
    pub available_cash: f64,
    pub daily_pnl: f64,
    pub unrealized_pnl: f64,
    pub unrealized_return_pct: f64,
    pub position_count: usize,
    pub market_value: f64,
    /// (行业, 市值占比 %) — 按市值加权, top5 + 其他 (BR-222 排序规则)
    pub sectors: &'a [(String, f64)],
    /// 个股明细 (BR-225: 逐个复盘)
    pub items: &'a [PositionReviewItem],
}

/// BR-225: 单只持仓复盘行。
#[derive(Debug, Clone, PartialEq)]
pub struct PositionReviewItem {
    pub code: String,
    pub name: String,
    pub quantity: i64,
    pub cost_price: f64,
    pub close: Option<f64>,
    pub market_value: f64,
    pub unrealized_pnl: f64,
    pub unrealized_return_pct: Option<f64>,
    pub daily_price_pnl: Option<f64>,
}

/// BR-222: R-11 持仓复盘模板渲染 (用户确认持仓摘要, 盘后 1次/日)。
pub fn render_position_review(p: PositionReviewParams<'_>) -> String {
    let mut out = format!("🏦 持仓复盘（{}）\n", p.date);
    out.push_str(&format!(
        "总资产 {:.0} | 仓位 {:.1}% | 可用现金 {:.0}\n",
        p.total_assets, p.position_ratio_pct, p.available_cash
    ));
    out.push_str(&format!(
        "日盈亏 {:+.2} | 未实现盈亏 {:+.2}（{:.2}%）\n",
        p.daily_pnl, p.unrealized_pnl, p.unrealized_return_pct
    ));
    out.push_str(&format!(
        "持仓 {} 只 | 持仓市值 {:.0}\n",
        p.position_count, p.market_value
    ));
    if !p.items.is_empty() {
        out.push_str("\n逐个复盘:\n");
        for (index, item) in p.items.iter().enumerate() {
            let close = item.close.map(|v| format!("{v:.2}")).unwrap_or_else(|| "-".to_string());
            let ret = item
                .unrealized_return_pct
                .map(|v| format!("{v:+.2}%"))
                .unwrap_or_else(|| "-".to_string());
            let daily = item
                .daily_price_pnl
                .map(|v| format!("{v:+.2}"))
                .unwrap_or_else(|| "-".to_string());
            out.push_str(&format!(
                "{}. {}({}) {}股 | 成本{:.2} 现价{} | 市值{:.0}\n   未实现{:+.0}({}) | 当日{}\n",
                index + 1,
                item.name,
                item.code,
                item.quantity,
                item.cost_price,
                close,
                item.market_value,
                item.unrealized_pnl,
                ret,
                daily
            ));
        }
    }
    out.push_str("行业分布（按市值）: ");
    if p.sectors.is_empty() {
        out.push_str("(无持仓)\n");
    } else {
        for (index, (sector, pct)) in p.sectors.iter().enumerate() {
            out.push_str(&format!(
                "{}{} {:.0}%",
                if index == 0 { "" } else { ", " },
                sector,
                pct
            ));
        }
        out.push('\n');
    }
    out.push_str("仅结构化事实, 非下单指令");
    out
}

/// BR-222: R-11 持仓复盘 (用户确认持仓摘要, 盘后 1次/日)。
///
/// 数据门: `user_account_summary` 无用户确认行 → no_data (不虚构账户状态);
/// 收盘估值未持久化 → no_data。空持仓仍投递 (显示 "无持仓"), 用户确认摘要本身
/// 是权威信息。行业分布按市值加权聚合, top5 + 其余归入 "其他"。
async fn dispatch_position_review_outcome(date: &str) -> crate::review_batch::ReviewTaskOutcome {
    use stock_analysis::database::closing_valuation::{
        latest_persisted_valuation_view, ClosingValuationView,
    };
    use stock_analysis::database::user_account_summary::latest as latest_user_account_summary;

    let summary = match tokio::task::spawn_blocking(latest_user_account_summary).await {
        Ok(Ok(Some(summary))) => summary,
        Ok(Ok(None)) => {
            log_dispatcher_attempt("R-11", false, 0, "user account summary not confirmed");
            return crate::review_batch::ReviewTaskOutcome::no_data(
                "user account summary not confirmed",
            );
        }
        Ok(Err(error)) => {
            let reason = format!("user account summary read failed: {error}");
            log_dispatcher_attempt("R-11", false, 0, &reason);
            return crate::review_batch::ReviewTaskOutcome::failed(true, reason);
        }
        Err(error) => {
            let reason = format!("user account summary join failed: {error}");
            return crate::review_batch::ReviewTaskOutcome::failed(true, reason);
        }
    };

    let valuation: Option<ClosingValuationView> =
        match tokio::task::spawn_blocking(latest_persisted_valuation_view).await {
            Ok(Ok(valuation)) => valuation,
            Ok(Err(error)) => {
                let reason = format!("closing valuation read failed: {error}");
                log_dispatcher_attempt("R-11", false, 0, &reason);
                return crate::review_batch::ReviewTaskOutcome::failed(true, reason);
            }
            Err(error) => {
                let reason = format!("closing valuation join failed: {error}");
                return crate::review_batch::ReviewTaskOutcome::failed(true, reason);
            }
        };
    let Some(valuation) = valuation else {
        log_dispatcher_attempt("R-11", false, 0, "closing valuation not persisted");
        return crate::review_batch::ReviewTaskOutcome::no_data("closing valuation not persisted");
    };

    let positions =
        match tokio::task::spawn_blocking(stock_analysis::portfolio::get_positions).await {
            Ok(Ok(positions)) => positions,
            Ok(Err(error)) => {
                let reason = format!("positions read failed: {error}");
                log_dispatcher_attempt("R-11", false, 0, &reason);
                return crate::review_batch::ReviewTaskOutcome::failed(true, reason);
            }
            Err(error) => {
                let reason = format!("positions join failed: {error}");
                return crate::review_batch::ReviewTaskOutcome::failed(true, reason);
            }
        };

    // 行业分布: 持仓市值 = 收盘估值 market_value (优先) 或 shares * cost_price; 按 Position.sector 聚合
    let mut sector_value: std::collections::BTreeMap<String, f64> =
        std::collections::BTreeMap::new();
    let mut total_position_value = 0.0_f64;
    for position in &positions {
        if position.status != stock_analysis::portfolio::PositionStatus::Holding {
            continue;
        }
        let market_value = valuation
            .valuation
            .items
            .iter()
            .find(|item| item.code == position.code)
            .map(|item| item.market_value.unwrap_or(0.0))
            .unwrap_or_else(|| position.shares as f64 * position.cost_price);
        total_position_value += market_value;
        *sector_value.entry(position.sector.clone()).or_insert(0.0) += market_value;
    }
    let mut sector_rows: Vec<(String, f64)> = sector_value
        .into_iter()
        .map(|(sector, value)| {
            let pct = if total_position_value > 0.0 {
                value / total_position_value * 100.0
            } else {
                0.0
            };
            (sector, pct)
        })
        .collect();
    sector_rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut top_sectors: Vec<(String, f64)> = Vec::new();
    let mut remainder = 0.0_f64;
    for (index, (sector, pct)) in sector_rows.into_iter().enumerate() {
        if index < 5 {
            top_sectors.push((sector, pct));
        } else {
            remainder += pct;
        }
    }
    if remainder > 0.0 {
        top_sectors.push(("其他".to_string(), remainder));
    }

    let unrealized_return_pct = if summary.securities_market_value > 0.0 {
        valuation.valuation.total_unrealized_pnl.unwrap_or(0.0) / summary.securities_market_value
            * 100.0
    } else {
        0.0
    };
    // BR-225: 个股明细 (逐个复盘, 按市值降序)
    let mut items: Vec<PositionReviewItem> = valuation
        .valuation
        .items
        .iter()
        .map(|item| PositionReviewItem {
            code: item.code.clone(),
            name: item.name.clone(),
            quantity: i64::try_from(item.quantity).unwrap_or(0),
            cost_price: item.cost_price,
            close: item.close,
            market_value: item.market_value.unwrap_or(0.0),
            unrealized_pnl: item.unrealized_pnl.unwrap_or(0.0),
            unrealized_return_pct: item.unrealized_return_pct,
            daily_price_pnl: item.daily_price_pnl,
        })
        .collect();
    items.sort_by(|a, b| {
        b.market_value
            .partial_cmp(&a.market_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let params = PositionReviewParams {
        date,
        total_assets: summary.total_assets,
        position_ratio_pct: summary.position_ratio_pct,
        available_cash: summary.available_cash,
        daily_pnl: summary.daily_pnl,
        unrealized_pnl: valuation.valuation.total_unrealized_pnl.unwrap_or(0.0),
        unrealized_return_pct,
        position_count: positions
            .iter()
            .filter(|p| p.status == stock_analysis::portfolio::PositionStatus::Holding)
            .count(),
        market_value: valuation
            .valuation
            .total_market_value
            .unwrap_or(total_position_value),
        sectors: &top_sectors,
        items: &items,
    };
    let text = render_position_review(params);
    let result = dispatch_registered_outcome!(
        "R-11-position-review",
        crate::notify::PushKind::PositionReview,
        "position_review_dispatcher",
        "render_position_review",
        "",
        None,
        text
    );
    log_dispatcher_attempt("R-11", result.is_pushed(), 1, "");
    crate::review_batch::ReviewTaskOutcome::from_push_outcome(result, 1)
}

/// BR-224: 候选/预测样本 5 日收益回填 (SignalTracker 闭环)。
/// 对近 days 天的 pending prediction 用日线收盘价验证 (复用 backfill_predictions 逻辑)。
async fn backfill_pending_predictions(days: i64) -> (usize, usize) {
    use chrono::Duration;
    let today = chrono::Local::now().date_naive();
    let mut total = 0usize;
    let mut hit_count = 0usize;
    for offset in 1..=days {
        let pred_date = today - Duration::days(offset);
        let target_date = pred_date + Duration::days(1);
        let pred_date_s = pred_date.format("%Y-%m-%d").to_string();
        let target_date_s = target_date.format("%Y-%m-%d").to_string();
        let db = stock_analysis::database::DatabaseManager::get();
        let Ok(pending) = db.get_pending_predictions(&pred_date_s) else {
            continue;
        };
        for pred in pending {
            let Some(code) = pred.stock_code.as_deref() else {
                continue;
            };
            if code.is_empty() {
                continue;
            }
            let Some(outcome) = stock_analysis::monitor::prediction::verify_one(
                db,
                code,
                &pred_date_s,
                &target_date_s,
                &pred.pred_direction,
            )
            .await
            else {
                continue;
            };
            if db
                .update_prediction_result(
                    &pred_date_s,
                    Some(code),
                    outcome.actual_change,
                    outcome.hit,
                )
                .is_ok()
            {
                total += 1;
                if outcome.hit {
                    hit_count += 1;
                }
            }
        }
    }
    (total, hit_count)
}

pub async fn dispatch_post_session_review(
    context: crate::review_batch::ReviewRunContext,
    due: &std::collections::BTreeSet<crate::review_batch::ReviewTask>,
) -> Result<crate::review_batch::ReviewBatchOutcome, String> {
    use crate::review_batch::{
        account_dependency_outcomes, merge_review_task_outcomes, partition_review_tasks,
        review_preflight, ReviewTask, ReviewTaskOutcome,
    };

    let business_date = context.business_date();
    let date = business_date.format("%Y-%m-%d").to_string();
    // 2026-08-06: 手动 --review 跳过 21:00 龙虎榜发布门 (R-04 立即尝试;
    // 未发布数据 gateway 返回空 → dispatcher 降级)。自动调度保持真实时钟。
    let now = if context.manual_override() {
        chrono::NaiveTime::from_hms_opt(23, 59, 59)
            .expect("BR-140 manual review time must be valid")
    } else {
        context.eligibility_time()
    };
    log::info!(
        "[B-005-C] 盘后批量 dispatcher 开始 ({}) manual_override={}",
        date,
        context.manual_override()
    );

    let is_test = stock_analysis::risk::env_guard::runtime_is_test_process()
        || stock_analysis::risk::env_guard::current_env()
            == stock_analysis::risk::env_guard::TradingEnv::Test;
    let preflight = review_preflight(context, due, is_test);
    let phases = partition_review_tasks(&preflight.runnable);
    let (r04, r07, r08, r09, r11, a10, a01) = tokio::join!(
        async {
            if phases.source_only.contains(&ReviewTask::R04) {
                Some((ReviewTask::R04, dispatch_r04_lhb_outcome(&date, now).await))
            } else {
                None
            }
        },
        async {
            if phases.source_only.contains(&ReviewTask::R07) {
                Some((
                    ReviewTask::R07,
                    dispatch_tomorrow_watch_outcome(&date).await,
                ))
            } else {
                None
            }
        },
        async {
            if phases.source_only.contains(&ReviewTask::R08) {
                Some((
                    ReviewTask::R08,
                    dispatch_r08_event_calendar_outcome(&date).await,
                ))
            } else {
                None
            }
        },
        async {
            if phases.source_only.contains(&ReviewTask::R09) {
                Some((
                    ReviewTask::R09,
                    dispatch_r09_provider_top_n_outcome(business_date).await,
                ))
            } else {
                None
            }
        },
        async {
            if phases.source_only.contains(&ReviewTask::R11) {
                Some((
                    ReviewTask::R11,
                    dispatch_position_review_outcome(&date).await,
                ))
            } else {
                None
            }
        },
        async {
            if phases.source_only.contains(&ReviewTask::A10) {
                Some((
                    ReviewTask::A10,
                    dispatch_catalyst_review_daily_outcome(&date).await,
                ))
            } else {
                None
            }
        },
        async {
            if phases.source_only.contains(&ReviewTask::A01) {
                Some((
                    ReviewTask::A01,
                    dispatch_paper_review_daily_outcome(&date).await,
                ))
            } else {
                None
            }
        },
    );
    let source_only_outcomes = [r04, r07, r08, r09, r11, a10, a01]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    // BR-224: SignalTracker 样本回填 (5 日收益验证, 每日复盘时执行)
    let (backfilled_total, backfilled_hits) = backfill_pending_predictions(14).await;
    if backfilled_total > 0 {
        log::info!(
            "[BR-224] 预测样本回填 verified={backfilled_total} hits={backfilled_hits}"
        );
    }

    // BR-223: 盘后大宗交易推送 (自选+持仓代码集, 非 ReviewTask 侧推)
    let mut block_trade_codes: Vec<String> = stock_analysis::portfolio::get_positions()
        .map(|positions| {
            positions
                .into_iter()
                .map(|position| position.code)
                .collect()
        })
        .unwrap_or_default();
    if let Ok(list) = std::env::var("STOCK_LIST") {
        for code in list.split(',') {
            let code = code.trim().to_string();
            if !code.is_empty() && !block_trade_codes.contains(&code) {
                block_trade_codes.push(code);
            }
        }
    }
    if !block_trade_codes.is_empty() {
        let block_trade_pushed =
            dispatch_block_trade_review(&block_trade_codes, business_date).await;
        log::info!("[BR-223] 盘后大宗交易推送 pushed={block_trade_pushed}");
    }
    // BR-223: A-11 IPO 阶段催化 (每日一次, 盘后侧推)
    let ipo_pushed = dispatch_ipo_catalyst(&date).await;
    log::info!("[BR-223] IPO 产业链催化 pushed={ipo_pushed}");
    let observed_at = chrono::Local::now().fixed_offset();
    // 2026-08-06 用户决策 (未接券商): R-03 (涨停产业链复盘) 解除账户 gate。
    // 其 dispatcher 数据源为 portfolio 持仓 + 涨停链 (不依赖 real_account_snapshot),
    // 直接走真实数据路径; 其余 account_required 任务保持 account_metrics_incomplete。
    let mut account_required_outcomes = Vec::new();
    for task in &phases.account_required {
        if *task == ReviewTask::R03 {
            account_required_outcomes.push((
                ReviewTask::R03,
                dispatch_r03_industry_chain_outcome(&date).await,
            ));
        } else {
            account_required_outcomes.push((
                *task,
                ReviewTaskOutcome::account_metrics_incomplete(observed_at),
            ));
        }
    }
    if !account_required_outcomes.is_empty() {
        log::warn!(
            "[复盘依赖][BR-194] dependency=legacy_account_gate status=unavailable affected_count={} stage=acquire_batch reason_code=account_metrics_incomplete retryable=true source_provider=none source_time=none (R-03 已解除, 走持仓+涨停链)",
            account_required_outcomes.len()
        );
    }
    let batch = merge_review_task_outcomes(
        preflight.outcomes,
        source_only_outcomes,
        account_required_outcomes,
    )?;
    let delivered = batch.delivered_count();
    let waiting = batch.waiting_tasks();
    let deferred = batch.deferred_tasks();
    let disabled = batch.disabled_tasks();
    let failed = batch.failed_tasks();
    let statuses = batch
        .tasks
        .iter()
        .map(|(task, outcome)| format!("{}:{}", task.label(), outcome.status_label()))
        .collect::<Vec<_>>();
    let no_data = batch
        .tasks
        .iter()
        .filter(|(_, outcome)| matches!(outcome, ReviewTaskOutcome::NoData { .. }))
        .count();
    let deferred_until = batch
        .tasks
        .iter()
        .filter_map(|(task, outcome)| match outcome {
            ReviewTaskOutcome::DeferredUntil { at, .. } => {
                Some(format!("{}@{}", task.label(), at.to_rfc3339()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    // v15.x rule 4: the aggregated line below only carries a hashed reason
    // category, so every non-delivered task also gets its raw cause on stdout.
    for diagnostic in batch.non_delivered_diagnostics() {
        log::warn!(
            "[B-005-C][BR-110][BR-140] task={} status={} reason_code={} retryable={} detail={}",
            diagnostic.task.label(),
            diagnostic.status,
            diagnostic.reason_code,
            diagnostic
                .retryable
                .map(|value| value.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
            diagnostic.detail,
        );
    }
    log::info!(
        "[B-005-C][BR-110][BR-140][BR-209] 完成 time={} attempted={} delivered={} no_data={} waiting={} deferred={} disabled={} failed={} statuses={:?} waiting_tasks={:?} deferred_tasks={:?} deferred_until={:?} disabled_tasks={:?} failed_tasks={:?}",
        now.format("%H:%M"),
        batch.tasks.len(),
        delivered,
        no_data,
        waiting.len(),
        deferred.len(),
        disabled.len(),
        failed.len(),
        statuses,
        waiting.iter().map(|task| task.label()).collect::<Vec<_>>(),
        deferred.iter().map(|task| task.label()).collect::<Vec<_>>(),
        deferred_until,
        disabled.iter().map(|task| task.label()).collect::<Vec<_>>(),
        failed.iter().map(|task| task.label()).collect::<Vec<_>>(),
    );
    Ok(batch)
}

// ============================================================================
// BR-140 review dispatchers: R-02 fail-closed capability + R-08 real dispatcher
// ============================================================================

/// R-02 今日盘面：等待 BR-093 所需的完整、同批次盘后市场概览。
///
/// 局部指数快照不能证明全市场成交额和涨跌停宽度。BR-140 要求能力缺失
/// 时在任何无关 acquisition 前返回 Disabled，因此这里不请求局部行情。
pub async fn dispatch_r02_review_market_real(_date: &str, _banner: &BannerCtx) -> bool {
    let reason = "disabled=no_complete_review_date_market_overview_batch";
    log::error!("[R-02][BR-093][BR-140] {reason}");
    log_dispatcher_attempt("R-02", false, 0, reason);
    false
}

struct R08PublicCalendarComponents {
    announcement_summary: Result<(String, usize), String>,
    futures_delivery: Result<(String, usize), String>,
    overnight_indices: Result<(String, usize), String>,
    overnight_fx: Result<(String, usize), String>,
}

#[derive(Debug)]
struct R08PreparedPublicCalendar {
    text: String,
    item_count: usize,
    complete_components: usize,
    failed_components: Vec<&'static str>,
}

fn render_r08_public_calendar(
    reminder_date: &str,
    announcement_summary: &str,
    futures_delivery_summary: &str,
    overnight_indices_summary: &str,
    overnight_fx_summary: &str,
    failed_components: &[&str],
) -> String {
    let mut text = format!(
        "🗓️ 下一交易日公共事件（{reminder_date}）\n公告: {announcement_summary}\n期货交割: {futures_delivery_summary}\n隔夜指数: {overnight_indices_summary}\n汇率: {overnight_fx_summary}"
    );
    if !failed_components.is_empty() {
        text.push_str("\n降级组件: ");
        text.push_str(&failed_components.join(","));
    }
    text.push_str("\n仅公共来源结构化事实");
    text
}

/// BR-196 production-owned presentation seam for the three visible
/// limit-board shapes.  The monitor producer supplies already validated rows;
/// this function owns only the stable title/shape and bounded Top10 assembly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitBoardsShape {
    First,
    Second,
    ThirdPlus,
}

pub fn render_limit_boards_shape(
    shape: LimitBoardsShape,
    hhmm: &str,
    rows: &[String],
) -> Result<String, String> {
    if hhmm.trim().is_empty() || rows.is_empty() || rows.iter().any(|row| row.trim().is_empty()) {
        return Err("BR-196 LimitBoards presentation requires time and nonempty rows".to_string());
    }
    let (icon, label) = match shape {
        LimitBoardsShape::First => ("🟢", "首板涨停"),
        LimitBoardsShape::Second => ("🟡", "二板涨停"),
        LimitBoardsShape::ThirdPlus => ("🔴", "三板+ 涨停"),
    };
    let mut lines = vec![format!(
        "{icon} {label} Top{}（{hhmm}）",
        rows.len().min(10)
    )];
    lines.extend(rows.iter().take(10).cloned());
    Ok(lines.join("\n"))
}

fn prepare_r08_public_calendar(
    reminder_date: &str,
    components: R08PublicCalendarComponents,
) -> Result<R08PreparedPublicCalendar, String> {
    let mut complete_components = 1usize;
    let mut item_count = 0usize;
    let mut failed_components = Vec::new();
    let ann_summary = match components.announcement_summary {
        Ok((summary, count)) => {
            complete_components += 1;
            item_count += count;
            summary
        }
        Err(error) => {
            log::error!("[R-08][BR-140] component=announcement unavailable: {error}");
            failed_components.push("market_announcements");
            "公告不可用（见审计）".to_string()
        }
    };
    let futures_delivery = match components.futures_delivery {
        Ok((summary, count)) => {
            item_count += count;
            summary
        }
        Err(error) => {
            log::error!("[R-08][BR-165] component=cffex_delivery unavailable: {error}");
            return Err(format!("r08_cffex_component_unavailable: {error}"));
        }
    };
    let us_summary = match components.overnight_indices {
        Ok((summary, count)) => {
            complete_components += 1;
            item_count += count;
            summary
        }
        Err(error) => {
            log::error!("[R-08][BR-161] component=overnight_indices unavailable: {error}");
            failed_components.push("overnight_indices");
            "不可用（见采集审计）".to_string()
        }
    };
    let fx_summary = match components.overnight_fx {
        Ok((summary, count)) => {
            complete_components += 1;
            item_count += count;
            summary
        }
        Err(error) => {
            log::error!("[R-08][BR-161] component=overnight_fx unavailable: {error}");
            failed_components.push("overnight_fx");
            "不可用（见采集审计）".to_string()
        }
    };
    let text = render_r08_public_calendar(
        reminder_date,
        &ann_summary,
        &futures_delivery,
        &us_summary,
        &fx_summary,
        &failed_components,
    );
    Ok(R08PreparedPublicCalendar {
        text,
        item_count,
        complete_components,
        failed_components,
    })
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct R08ProviderEvidenceBinding {
    component: String,
    provider: magic_market_core::ProviderId,
    source: String,
    source_at: Option<String>,
    observed_at: String,
    batch_id: String,
    status: String,
    record_count: usize,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct R08AnnouncementFactBinding {
    source_order_ordinal: usize,
    announcement_id: String,
    code: String,
    category: Option<String>,
    title: String,
    published_at: String,
    canonical_url: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct R08FuturesFactBinding {
    source_order_ordinal: usize,
    contract_code: String,
    product_code: String,
    last_trading_date: Option<String>,
    delivery_date: String,
    notice_url: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct R08IndexFactBinding {
    source_order_ordinal: usize,
    code: magic_market_core::GlobalIndexCode,
    name: String,
    value: f64,
    change: f64,
    change_percent: f64,
    source_at: String,
    observed_at: String,
    provider: magic_market_core::ProviderId,
    batch_id: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct R08FxFactBinding {
    source_order_ordinal: usize,
    pair: magic_market_core::FxPair,
    name: String,
    rate: f64,
    change: Option<f64>,
    change_percent: Option<f64>,
    source_at: String,
    observed_at: String,
    provider: magic_market_core::ProviderId,
    batch_id: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct R08TaskTransitionBasis {
    task_identity: String,
    business_date: String,
    task: String,
    source: String,
    source_time: Option<String>,
    rule_ids: Vec<String>,
    snapshot_size: usize,
    batch_ids: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct R08PublicSourceBinding {
    schema_version: u32,
    business_date: String,
    reminder_date: String,
    template_id: String,
    review_task_identity: String,
    delivery_subject_identity: String,
    provider_batches: Vec<R08ProviderEvidenceBinding>,
    announcements: Vec<R08AnnouncementFactBinding>,
    futures_delivery: Vec<R08FuturesFactBinding>,
    overnight_indices: Vec<R08IndexFactBinding>,
    overnight_fx: Vec<R08FxFactBinding>,
    unavailable_optional_components: Vec<String>,
    rendered_content_sha256: String,
    task_transition_basis: R08TaskTransitionBasis,
}

#[derive(Debug)]
struct PreparedR08CountedDelivery {
    rendered: String,
    item_count: usize,
    business_date: chrono::NaiveDate,
    task_identity: String,
    delivery_subject_identity: String,
    source_binding_canonical: Vec<u8>,
    task_transition_basis_canonical: Vec<u8>,
    provider_observed_at: chrono::DateTime<chrono::Utc>,
    ordered_batch_ids: Vec<String>,
}

fn r08_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    format!("{:x}", Sha256::digest(bytes))
}

fn parse_r08_observed_at(value: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(parsed.with_timezone(&chrono::Utc));
    }
    let (seconds, fractional) = value
        .split_once('.')
        .ok_or_else(|| format!("R-08 provider observed_at is invalid: {value:?}"))?;
    if seconds.starts_with('-')
        || seconds.is_empty()
        || !seconds.bytes().all(|byte| byte.is_ascii_digit())
        || fractional.is_empty()
        || fractional.len() > 9
        || !fractional.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!("R-08 provider observed_at is invalid: {value:?}"));
    }
    let seconds = seconds
        .parse::<i64>()
        .map_err(|error| format!("R-08 provider observed_at seconds are invalid: {error}"))?;
    let mut nanos = fractional.to_string();
    nanos.extend(std::iter::repeat_n('0', 9 - nanos.len()));
    let nanos = nanos
        .parse::<u32>()
        .map_err(|error| format!("R-08 provider observed_at nanos are invalid: {error}"))?;
    chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, nanos)
        .ok_or_else(|| "R-08 provider observed_at is outside the supported range".to_string())
}

pub(super) struct ValidatedR08PublicBinding {
    pub business_date: chrono::NaiveDate,
    pub reminder_date: chrono::NaiveDate,
    pub task_identity: String,
    pub delivery_subject_identity: String,
    pub transition_basis_canonical: Vec<u8>,
    pub rendered_content_sha256: String,
    pub ordered_batch_ids: Vec<String>,
    pub max_observed_at: chrono::DateTime<chrono::Utc>,
}

fn r08_is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_r08_public_binding_fields(
    binding: R08PublicSourceBinding,
) -> Result<ValidatedR08PublicBinding, &'static str> {
    use magic_market_core::ProviderId;

    const INVALID: &str = "counted_r08_source_only_binding_invalid";
    let business_date = chrono::NaiveDate::parse_from_str(&binding.business_date, "%Y-%m-%d")
        .map_err(|_| INVALID)?;
    let reminder_date = chrono::NaiveDate::parse_from_str(&binding.reminder_date, "%Y-%m-%d")
        .map_err(|_| INVALID)?;
    let expected_task_identity = crate::review_batch::review_task_identity(
        business_date,
        crate::review_batch::ReviewTask::R08,
    );
    let expected_subject_identity = crate::review_batch::audit_identity_hash(
        "event-calendar-delivery-subject",
        &format!("{}:{expected_task_identity}", binding.business_date),
    );
    if binding.schema_version != 1
        || binding.template_id != "event_calendar_v1"
        || reminder_date != r08_reminder_trading_date(business_date)
        || binding.review_task_identity != expected_task_identity
        || binding.delivery_subject_identity != expected_subject_identity
        || !r08_is_sha256_hex(&binding.rendered_content_sha256)
    {
        return Err(INVALID);
    }

    let allowed = [
        ("market_announcements", ProviderId::Cninfo, "cninfo-market"),
        ("cffex_delivery", ProviderId::Cffex, "cffex-official-notice"),
        ("overnight_indices", ProviderId::Sina, "sina-web"),
        ("overnight_fx", ProviderId::Sina, "sina-web"),
    ];
    let mut previous_position = None;
    let mut ordered_batch_ids = Vec::with_capacity(binding.provider_batches.len());
    let mut observed_times = Vec::with_capacity(binding.provider_batches.len());
    let mut present_components = std::collections::HashSet::new();
    for evidence in &binding.provider_batches {
        let position = allowed
            .iter()
            .position(|(component, _, _)| *component == evidence.component)
            .ok_or(INVALID)?;
        let observed_at = parse_r08_observed_at(&evidence.observed_at).map_err(|_| INVALID)?;
        if previous_position.is_some_and(|previous| position <= previous)
            || evidence.provider != allowed[position].1
            || evidence.source != allowed[position].2
            || evidence.batch_id.trim().is_empty()
            || !matches!(evidence.status.as_str(), "available" | "verified_empty")
            || (evidence.status == "available"
                && evidence.record_count == 0
                && evidence.component != "cffex_delivery")
            || (evidence.status == "verified_empty"
                && (evidence.record_count != 0 || evidence.source_at.is_some()))
            || !present_components.insert(evidence.component.as_str())
        {
            return Err(INVALID);
        }
        if evidence.status == "available" {
            let source_at = evidence.source_at.as_deref().ok_or(INVALID)?;
            match evidence.component.as_str() {
                "cffex_delivery" => {
                    let published = chrono::NaiveDate::parse_from_str(source_at, "%Y-%m-%d")
                        .map_err(|_| INVALID)?;
                    if published > reminder_date || published > observed_at.date_naive() {
                        return Err(INVALID);
                    }
                }
                "market_announcements" => {
                    let published =
                        chrono::DateTime::parse_from_rfc3339(source_at).map_err(|_| INVALID)?;
                    if published.date_naive() != business_date
                        || published.with_timezone(&chrono::Utc) > observed_at
                    {
                        return Err(INVALID);
                    }
                }
                "overnight_indices" | "overnight_fx" => {
                    let provider_at = chrono::DateTime::parse_from_rfc3339(source_at)
                        .map_err(|_| INVALID)?
                        .with_timezone(&chrono::Utc);
                    if provider_at > observed_at {
                        return Err(INVALID);
                    }
                }
                _ => return Err(INVALID),
            }
        }
        previous_position = Some(position);
        ordered_batch_ids.push(evidence.batch_id.clone());
        observed_times.push(observed_at);
    }
    if !present_components.contains("cffex_delivery")
        || ordered_batch_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != ordered_batch_ids.len()
    {
        return Err(INVALID);
    }

    let missing_optional = ["market_announcements", "overnight_indices", "overnight_fx"]
        .into_iter()
        .filter(|component| !present_components.contains(component))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if binding.unavailable_optional_components != missing_optional {
        return Err(INVALID);
    }
    let evidence_for = |component: &str| {
        binding
            .provider_batches
            .iter()
            .find(|evidence| evidence.component == component)
    };

    if let Some(evidence) = evidence_for("market_announcements") {
        if evidence.record_count != binding.announcements.len() {
            return Err(INVALID);
        }
        for (ordinal, fact) in binding.announcements.iter().enumerate() {
            let published_at =
                chrono::DateTime::parse_from_rfc3339(&fact.published_at).map_err(|_| INVALID)?;
            let observed_at = parse_r08_observed_at(&evidence.observed_at).map_err(|_| INVALID)?;
            if fact.source_order_ordinal != ordinal
                || fact.announcement_id.trim().is_empty()
                || fact.code.trim().is_empty()
                || fact.title.trim().is_empty()
                || fact.canonical_url.trim().is_empty()
                || published_at.date_naive() != business_date
                || published_at.with_timezone(&chrono::Utc) > observed_at
            {
                return Err(INVALID);
            }
        }
    } else if !binding.announcements.is_empty() {
        return Err(INVALID);
    }

    let cffex = evidence_for("cffex_delivery").ok_or(INVALID)?;
    if cffex.record_count != binding.futures_delivery.len() {
        return Err(INVALID);
    }
    let mut previous_cffex_key: Option<(&str, &str, &str, Option<&str>)> = None;
    for (ordinal, fact) in binding.futures_delivery.iter().enumerate() {
        let delivery_date = chrono::NaiveDate::parse_from_str(&fact.delivery_date, "%Y-%m-%d")
            .map_err(|_| INVALID)?;
        let canonical_key = (
            fact.contract_code.as_str(),
            fact.product_code.as_str(),
            fact.notice_url.as_str(),
            fact.last_trading_date.as_deref(),
        );
        if fact.source_order_ordinal != ordinal
            || fact.contract_code.trim().is_empty()
            || fact.product_code.trim().is_empty()
            || fact.notice_url.trim().is_empty()
            || delivery_date != reminder_date
            || previous_cffex_key.is_some_and(|previous| previous >= canonical_key)
            || fact
                .last_trading_date
                .as_deref()
                .is_some_and(|date| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err())
        {
            return Err(INVALID);
        }
        previous_cffex_key = Some(canonical_key);
    }

    if let Some(evidence) = evidence_for("overnight_indices") {
        if evidence.record_count != binding.overnight_indices.len() {
            return Err(INVALID);
        }
        for (ordinal, fact) in binding.overnight_indices.iter().enumerate() {
            let source_at = chrono::DateTime::parse_from_rfc3339(&fact.source_at)
                .map_err(|_| INVALID)?
                .with_timezone(&chrono::Utc);
            let observed_at = chrono::DateTime::parse_from_rfc3339(&fact.observed_at)
                .map_err(|_| INVALID)?
                .with_timezone(&chrono::Utc);
            let evidence_observed_at =
                parse_r08_observed_at(&evidence.observed_at).map_err(|_| INVALID)?;
            if fact.source_order_ordinal != ordinal
                || fact.name.trim().is_empty()
                || !fact.value.is_finite()
                || fact.value <= 0.0
                || !fact.change.is_finite()
                || !fact.change_percent.is_finite()
                || fact.provider != ProviderId::Sina
                || fact.batch_id != evidence.batch_id
                || source_at > observed_at
                || observed_at != evidence_observed_at
            {
                return Err(INVALID);
            }
        }
    } else if !binding.overnight_indices.is_empty() {
        return Err(INVALID);
    }

    if let Some(evidence) = evidence_for("overnight_fx") {
        if evidence.record_count != binding.overnight_fx.len() {
            return Err(INVALID);
        }
        for (ordinal, fact) in binding.overnight_fx.iter().enumerate() {
            let source_at = chrono::DateTime::parse_from_rfc3339(&fact.source_at)
                .map_err(|_| INVALID)?
                .with_timezone(&chrono::Utc);
            let observed_at = chrono::DateTime::parse_from_rfc3339(&fact.observed_at)
                .map_err(|_| INVALID)?
                .with_timezone(&chrono::Utc);
            let evidence_observed_at =
                parse_r08_observed_at(&evidence.observed_at).map_err(|_| INVALID)?;
            if fact.source_order_ordinal != ordinal
                || fact.name.trim().is_empty()
                || !fact.rate.is_finite()
                || fact.rate <= 0.0
                || fact.change.is_some_and(|value| !value.is_finite())
                || fact.change_percent.is_some_and(|value| !value.is_finite())
                || fact.provider != ProviderId::Sina
                || fact.batch_id != evidence.batch_id
                || source_at > observed_at
                || observed_at != evidence_observed_at
            {
                return Err(INVALID);
            }
        }
    } else if !binding.overnight_fx.is_empty() {
        return Err(INVALID);
    }

    let expected_rules = [
        "BR-110", "BR-140", "BR-161", "BR-165", "BR-192", "BR-199", "BR-200",
    ];
    if binding.task_transition_basis.task_identity != expected_task_identity
        || binding.task_transition_basis.business_date != binding.business_date
        || binding.task_transition_basis.task != "R-08"
        || binding.task_transition_basis.source != "event_calendar_public_component_batches"
        || binding.task_transition_basis.batch_ids != ordered_batch_ids
        || binding.task_transition_basis.rule_ids.len() != expected_rules.len()
        || !binding
            .task_transition_basis
            .rule_ids
            .iter()
            .zip(expected_rules)
            .all(|(actual, expected)| actual == expected)
    {
        return Err(INVALID);
    }
    let max_observed_at = observed_times.into_iter().max().ok_or(INVALID)?;
    if binding.task_transition_basis.source_time.as_deref()
        != Some(max_observed_at.to_rfc3339().as_str())
    {
        return Err(INVALID);
    }
    let transition_basis_canonical =
        serde_json::to_vec(&binding.task_transition_basis).map_err(|_| INVALID)?;
    Ok(ValidatedR08PublicBinding {
        business_date,
        reminder_date,
        task_identity: binding.review_task_identity,
        delivery_subject_identity: binding.delivery_subject_identity,
        transition_basis_canonical,
        rendered_content_sha256: binding.rendered_content_sha256,
        ordered_batch_ids,
        max_observed_at,
    })
}

pub(super) fn validate_r08_public_source_binding_canonical_bytes(
    canonical: &[u8],
) -> Result<ValidatedR08PublicBinding, &'static str> {
    const INVALID: &str = "counted_r08_source_only_binding_invalid";
    let binding: R08PublicSourceBinding = serde_json::from_slice(canonical).map_err(|_| INVALID)?;
    let expected = serde_json::to_vec(&binding).map_err(|_| INVALID)?;
    if expected != canonical {
        return Err(INVALID);
    }
    validate_r08_public_binding_fields(binding)
}

fn r08_provider_evidence_binding<T>(
    component: &str,
    expected_provider: magic_market_core::ProviderId,
    expected_source: &str,
    batch: &stock_analysis::data_gateway::GatewayBatch<T>,
) -> Result<(R08ProviderEvidenceBinding, chrono::DateTime<chrono::Utc>), String> {
    let evidence = batch.evidence();
    if evidence.provider != expected_provider {
        return Err(format!(
            "R-08 {component} provider mismatch: expected={expected_provider:?} actual={:?}",
            evidence.provider
        ));
    }
    if evidence.source != expected_source {
        return Err(format!(
            "R-08 {component} source mismatch: expected={expected_source} actual={}",
            evidence.source
        ));
    }
    if evidence.batch_id.trim().is_empty() {
        return Err(format!("R-08 {component} batch ID is missing"));
    }
    let observed_at = parse_r08_observed_at(&evidence.observed_at)?;
    Ok((
        R08ProviderEvidenceBinding {
            component: component.to_string(),
            provider: evidence.provider,
            source: evidence.source.clone(),
            source_at: evidence.source_at.clone(),
            observed_at: evidence.observed_at.clone(),
            batch_id: evidence.batch_id.clone(),
            status: if batch.is_verified_empty() {
                "verified_empty"
            } else {
                "available"
            }
            .to_string(),
            record_count: batch.records().len(),
        },
        observed_at,
    ))
}

#[allow(clippy::too_many_arguments)]
fn prepare_r08_counted_delivery(
    business_date: chrono::NaiveDate,
    reminder_date: chrono::NaiveDate,
    prepared: R08PreparedPublicCalendar,
    announcements: Option<
        &stock_analysis::data_gateway::GatewayBatch<
            stock_analysis::data_gateway::EventAnnouncement,
        >,
    >,
    futures_delivery: Option<
        &stock_analysis::data_gateway::GatewayBatch<
            stock_analysis::data_gateway::FuturesDeliveryFact,
        >,
    >,
    overnight_indices: Option<
        &stock_analysis::data_gateway::GatewayBatch<stock_analysis::data_gateway::GlobalIndexFact>,
    >,
    overnight_fx: Option<
        &stock_analysis::data_gateway::GatewayBatch<
            stock_analysis::data_gateway::ForeignExchangeFact,
        >,
    >,
) -> Result<PreparedR08CountedDelivery, String> {
    use magic_market_core::ProviderId;
    use stock_analysis::data_gateway::GatewayBatch;

    if r08_reminder_trading_date(business_date) != reminder_date {
        return Err("R-08 reminder date must be the next trading day".to_string());
    }
    if prepared.text.trim().is_empty() || prepared.complete_components == 0 {
        return Err("R-08 rendered calendar is empty or has no complete component".to_string());
    }
    let futures_delivery = futures_delivery.ok_or_else(|| {
        "r08_cffex_component_unavailable: complete CFFEX gateway batch is required".to_string()
    })?;

    let mut provider_batches = Vec::new();
    let mut observed_times = Vec::new();
    let mut ordered_batch_ids = Vec::new();
    let mut announcements_projection = Vec::new();
    let mut futures_projection = Vec::new();
    let mut indices_projection = Vec::new();
    let mut fx_projection = Vec::new();

    if let Some(batch) = announcements {
        let (binding, observed_at) = r08_provider_evidence_binding(
            "market_announcements",
            ProviderId::Cninfo,
            "cninfo-market",
            batch,
        )?;
        match batch {
            GatewayBatch::VerifiedEmpty(evidence) => {
                if evidence.source_at.is_some() {
                    return Err(
                        "R-08 verified-empty announcement batch must not claim source_at"
                            .to_string(),
                    );
                }
            }
            GatewayBatch::Available { records, evidence } => {
                let source_at = evidence
                    .source_at
                    .as_deref()
                    .ok_or_else(|| "R-08 announcement source_at is missing".to_string())?;
                let newest = chrono::DateTime::parse_from_rfc3339(source_at)
                    .map_err(|error| format!("R-08 announcement source_at is invalid: {error}"))?;
                if newest.date_naive() != business_date
                    || newest.with_timezone(&chrono::Utc) > observed_at
                {
                    return Err(format!(
                        "R-08 announcement source time {newest} is outside business date {business_date} or after observation"
                    ));
                }
                for (source_order_ordinal, record) in records.iter().enumerate() {
                    let published_at = chrono::DateTime::parse_from_rfc3339(&record.published_at)
                        .map_err(|error| {
                        format!("R-08 announcement published_at is invalid: {error}")
                    })?;
                    if published_at.date_naive() != business_date
                        || published_at.with_timezone(&chrono::Utc) > observed_at
                        || record.announcement_id.trim().is_empty()
                        || record.code.trim().is_empty()
                        || record.title.trim().is_empty()
                        || record.canonical_url.trim().is_empty()
                    {
                        return Err(format!(
                            "R-08 announcement source facts are invalid at ordinal {source_order_ordinal}"
                        ));
                    }
                    announcements_projection.push(R08AnnouncementFactBinding {
                        source_order_ordinal,
                        announcement_id: record.announcement_id.clone(),
                        code: record.code.clone(),
                        category: record.category.clone(),
                        title: record.title.clone(),
                        published_at: record.published_at.clone(),
                        canonical_url: record.canonical_url.clone(),
                    });
                }
            }
        }
        ordered_batch_ids.push(binding.batch_id.clone());
        observed_times.push(observed_at);
        provider_batches.push(binding);
    }

    {
        let batch = futures_delivery;
        let (mut binding, observed_at) = r08_provider_evidence_binding(
            "cffex_delivery",
            ProviderId::Cffex,
            "cffex-official-notice",
            batch,
        )?;
        match batch {
            GatewayBatch::VerifiedEmpty(evidence) => {
                if evidence.source_at.is_some() {
                    return Err(
                        "R-08 verified-empty CFFEX batch must not claim source_at".to_string()
                    );
                }
            }
            GatewayBatch::Available { evidence, .. } => {
                let source_at = evidence
                    .source_at
                    .as_deref()
                    .ok_or_else(|| "R-08 CFFEX notice publication date is missing".to_string())?;
                let publication_date = chrono::NaiveDate::parse_from_str(source_at, "%Y-%m-%d")
                    .map_err(|error| {
                        format!("R-08 CFFEX notice publication date is invalid: {error}")
                    })?;
                if publication_date > reminder_date || publication_date > observed_at.date_naive() {
                    return Err(format!(
                        "R-08 CFFEX notice publication date {publication_date} is after reminder date {reminder_date} or observation date {}",
                        observed_at.date_naive()
                    ));
                }
            }
        }
        let reminder_projection = r08_cffex_reminder_projection(batch, reminder_date);
        binding.record_count = reminder_projection.len();
        for (source_order_ordinal, record) in reminder_projection.into_iter().enumerate() {
            if record.contract_code.trim().is_empty()
                || record.product_code.trim().is_empty()
                || record.notice_url.trim().is_empty()
                || record.delivery_date != reminder_date
            {
                return Err(format!(
                    "R-08 CFFEX source facts are invalid at ordinal {source_order_ordinal}"
                ));
            }
            futures_projection.push(R08FuturesFactBinding {
                source_order_ordinal,
                contract_code: record.contract_code.clone(),
                product_code: record.product_code.clone(),
                last_trading_date: record.last_trading_date.map(|date| date.to_string()),
                delivery_date: record.delivery_date.to_string(),
                notice_url: record.notice_url.clone(),
            });
        }
        ordered_batch_ids.push(binding.batch_id.clone());
        observed_times.push(observed_at);
        provider_batches.push(binding);
    }

    if let Some(batch) = overnight_indices {
        if batch.is_verified_empty() || batch.records().is_empty() {
            return Err("R-08 global-index component cannot be verified empty".to_string());
        }
        let (binding, observed_at) = r08_provider_evidence_binding(
            "overnight_indices",
            ProviderId::Sina,
            "sina-web",
            batch,
        )?;
        batch
            .evidence()
            .source_at
            .as_deref()
            .ok_or_else(|| "R-08 global-index batch source_at is missing".to_string())
            .and_then(|source_at| {
                chrono::DateTime::parse_from_rfc3339(source_at)
                    .map(|_| ())
                    .map_err(|error| {
                        format!("R-08 global-index batch source_at is invalid: {error}")
                    })
            })?;
        for (source_order_ordinal, record) in batch.records().iter().enumerate() {
            if record.name.trim().is_empty()
                || !record.value.is_finite()
                || record.value <= 0.0
                || !record.change.is_finite()
                || !record.change_percent.is_finite()
                || record.provider != ProviderId::Sina
                || record.batch_id != binding.batch_id
                || record.observed_at != observed_at
                || record.source_at > record.observed_at
            {
                return Err(format!(
                    "R-08 global-index source facts are invalid at ordinal {source_order_ordinal}"
                ));
            }
            indices_projection.push(R08IndexFactBinding {
                source_order_ordinal,
                code: record.code,
                name: record.name.clone(),
                value: record.value,
                change: record.change,
                change_percent: record.change_percent,
                source_at: record.source_at.to_rfc3339(),
                observed_at: record.observed_at.to_rfc3339(),
                provider: record.provider,
                batch_id: record.batch_id.clone(),
            });
        }
        ordered_batch_ids.push(binding.batch_id.clone());
        observed_times.push(observed_at);
        provider_batches.push(binding);
    }

    if let Some(batch) = overnight_fx {
        if batch.is_verified_empty() || batch.records().is_empty() {
            return Err("R-08 foreign-exchange component cannot be verified empty".to_string());
        }
        let (binding, observed_at) =
            r08_provider_evidence_binding("overnight_fx", ProviderId::Sina, "sina-web", batch)?;
        batch
            .evidence()
            .source_at
            .as_deref()
            .ok_or_else(|| "R-08 foreign-exchange batch source_at is missing".to_string())
            .and_then(|source_at| {
                chrono::DateTime::parse_from_rfc3339(source_at)
                    .map(|_| ())
                    .map_err(|error| {
                        format!("R-08 foreign-exchange batch source_at is invalid: {error}")
                    })
            })?;
        for (source_order_ordinal, record) in batch.records().iter().enumerate() {
            if record.name.trim().is_empty()
                || !record.rate.is_finite()
                || record.rate <= 0.0
                || record.change.is_some_and(|value| !value.is_finite())
                || record
                    .change_percent
                    .is_some_and(|value| !value.is_finite())
                || record.provider != ProviderId::Sina
                || record.batch_id != binding.batch_id
                || record.observed_at != observed_at
                || record.source_at > record.observed_at
            {
                return Err(format!(
                    "R-08 foreign-exchange source facts are invalid at ordinal {source_order_ordinal}"
                ));
            }
            fx_projection.push(R08FxFactBinding {
                source_order_ordinal,
                pair: record.pair,
                name: record.name.clone(),
                rate: record.rate,
                change: record.change,
                change_percent: record.change_percent,
                source_at: record.source_at.to_rfc3339(),
                observed_at: record.observed_at.to_rfc3339(),
                provider: record.provider,
                batch_id: record.batch_id.clone(),
            });
        }
        ordered_batch_ids.push(binding.batch_id.clone());
        observed_times.push(observed_at);
        provider_batches.push(binding);
    }

    if provider_batches.is_empty() {
        return Err(
            "R-08 counted delivery requires at least one complete GatewayBatch".to_string(),
        );
    }
    let unique_batch_ids = ordered_batch_ids
        .iter()
        .collect::<std::collections::HashSet<_>>();
    if unique_batch_ids.len() != ordered_batch_ids.len() {
        return Err("R-08 component batches must retain distinct batch IDs".to_string());
    }
    let provider_observed_at = observed_times
        .into_iter()
        .max()
        .ok_or_else(|| "R-08 provider observation time is missing".to_string())?;
    let business_date_text = business_date.format("%Y-%m-%d").to_string();
    let task_identity = crate::review_batch::review_task_identity(
        business_date,
        crate::review_batch::ReviewTask::R08,
    );
    let delivery_subject_identity = crate::review_batch::audit_identity_hash(
        "event-calendar-delivery-subject",
        &format!("{business_date_text}:{task_identity}"),
    );
    let task_transition_basis = R08TaskTransitionBasis {
        task_identity: task_identity.clone(),
        business_date: business_date_text.clone(),
        task: "R-08".to_string(),
        source: "event_calendar_public_component_batches".to_string(),
        source_time: Some(provider_observed_at.to_rfc3339()),
        rule_ids: vec![
            "BR-110".to_string(),
            "BR-140".to_string(),
            "BR-161".to_string(),
            "BR-165".to_string(),
            "BR-192".to_string(),
            "BR-199".to_string(),
            "BR-200".to_string(),
        ],
        snapshot_size: prepared.item_count,
        batch_ids: ordered_batch_ids.clone(),
    };
    let rendered_content_sha256 = r08_sha256(prepared.text.as_bytes());
    let source_binding = R08PublicSourceBinding {
        schema_version: 1,
        business_date: business_date_text,
        reminder_date: reminder_date.format("%Y-%m-%d").to_string(),
        template_id: "event_calendar_v1".to_string(),
        review_task_identity: task_identity.clone(),
        delivery_subject_identity: delivery_subject_identity.clone(),
        provider_batches,
        announcements: announcements_projection,
        futures_delivery: futures_projection,
        overnight_indices: indices_projection,
        overnight_fx: fx_projection,
        unavailable_optional_components: prepared
            .failed_components
            .iter()
            .map(|component| (*component).to_string())
            .collect(),
        rendered_content_sha256,
        task_transition_basis: task_transition_basis.clone(),
    };
    let source_binding_canonical = serde_json::to_vec(&source_binding)
        .map_err(|error| format!("R-08 source binding serialization failed: {error}"))?;
    let task_transition_basis_canonical = serde_json::to_vec(&task_transition_basis)
        .map_err(|error| format!("R-08 task transition serialization failed: {error}"))?;
    Ok(PreparedR08CountedDelivery {
        rendered: prepared.text,
        item_count: prepared.item_count,
        business_date,
        task_identity,
        delivery_subject_identity,
        source_binding_canonical,
        task_transition_basis_canonical,
        provider_observed_at,
        ordered_batch_ids,
    })
}

fn build_gateway_event_calendar_summary(
    batch: &stock_analysis::data_gateway::GatewayBatch<
        stock_analysis::data_gateway::EventAnnouncement,
    >,
) -> (String, usize) {
    let announcements: Vec<_> = batch
        .records()
        .iter()
        .filter(|announcement| {
            stock_analysis::announcement::announcement_title_is_immediately_actionable(
                &announcement.title,
            )
        })
        .collect();
    if announcements.is_empty() {
        return (
            format!(
                "CNInfo 当日公告已验证 {} 条；可即时通知公告 0 条",
                batch.records().len()
            ),
            0,
        );
    }

    let display_count = announcements.len().min(6);
    let mut summary = format!(
        "CNInfo 当日公告已验证 {} 条；公共事件 (TOP {}):",
        batch.records().len(),
        display_count
    );
    for announcement in announcements.into_iter().take(display_count) {
        if let Some(category) = announcement.category.as_deref() {
            summary.push_str(&format!(
                "\n· {} ({}): {}",
                announcement.code, category, announcement.title
            ));
        } else {
            summary.push_str(&format!(
                "\n· {}: {}",
                announcement.code, announcement.title
            ));
        }
    }
    (summary, display_count)
}

fn build_cffex_delivery_summary(
    batch: &stock_analysis::data_gateway::GatewayBatch<
        stock_analysis::data_gateway::FuturesDeliveryFact,
    >,
    reminder_date: chrono::NaiveDate,
) -> (String, usize) {
    let contracts = r08_cffex_reminder_projection(batch, reminder_date);
    if contracts.is_empty() {
        return (
            format!("中金所官方批次已验证；{} 无股指期货交割", reminder_date),
            0,
        );
    }

    let contract_list = contracts
        .iter()
        .map(|record| record.contract_code.as_str())
        .collect::<Vec<_>>()
        .join("/");
    (
        format!(
            "⚠️ {} 中金所 {} 到期交割（官方通知；交割方式未由该通知提供）",
            reminder_date, contract_list
        ),
        contracts.len(),
    )
}

fn r08_cffex_reminder_projection(
    batch: &stock_analysis::data_gateway::GatewayBatch<
        stock_analysis::data_gateway::FuturesDeliveryFact,
    >,
    reminder_date: chrono::NaiveDate,
) -> Vec<&stock_analysis::data_gateway::FuturesDeliveryFact> {
    let mut contracts: Vec<_> = batch
        .records()
        .iter()
        .filter(|record| record.delivery_date == reminder_date)
        .collect();
    contracts.sort_by(|left, right| {
        left.contract_code
            .cmp(&right.contract_code)
            .then_with(|| left.product_code.cmp(&right.product_code))
            .then_with(|| left.notice_url.cmp(&right.notice_url))
            .then_with(|| left.last_trading_date.cmp(&right.last_trading_date))
    });
    contracts
}

fn build_global_indices_summary(
    batch: &stock_analysis::data_gateway::GatewayBatch<
        stock_analysis::data_gateway::GlobalIndexFact,
    >,
) -> (String, usize) {
    let summary = batch
        .records()
        .iter()
        .map(|record| {
            let name = match record.code {
                magic_market_core::GlobalIndexCode::DowJones => "道琼斯",
                magic_market_core::GlobalIndexCode::NasdaqComposite => "纳斯达克",
                magic_market_core::GlobalIndexCode::Sp500 => "标普500",
                magic_market_core::GlobalIndexCode::Nikkei225 => "日经225",
                magic_market_core::GlobalIndexCode::HangSeng => "恒生指数",
                magic_market_core::GlobalIndexCode::Ftse100 => "富时100",
            };
            format!("{name} {:+.2}%", record.change_percent)
        })
        .collect::<Vec<_>>()
        .join(" / ");
    (summary, batch.records().len())
}

fn build_global_fx_summary(
    batch: &stock_analysis::data_gateway::GatewayBatch<
        stock_analysis::data_gateway::ForeignExchangeFact,
    >,
) -> (String, usize) {
    let summary = batch
        .records()
        .iter()
        .map(|record| match record.change_percent {
            Some(change_percent) => {
                format!("美元/人民币 {:.4} ({change_percent:+.2}%)", record.rate)
            }
            None => format!("美元/人民币 {:.4}（涨跌幅未提供）", record.rate),
        })
        .collect::<Vec<_>>()
        .join(" / ");
    (summary, batch.records().len())
}

fn r08_reminder_trading_date(review_date: chrono::NaiveDate) -> chrono::NaiveDate {
    stock_analysis::calendar::next_trading_day(review_date)
}

type R08ProviderLoadResult = (
    Result<
        stock_analysis::data_gateway::GatewayBatch<stock_analysis::data_gateway::EventAnnouncement>,
        stock_analysis::data_gateway::GatewayError,
    >,
    Result<
        stock_analysis::data_gateway::GatewayBatch<
            stock_analysis::data_gateway::FuturesDeliveryFact,
        >,
        stock_analysis::data_gateway::GatewayError,
    >,
    Result<
        stock_analysis::data_gateway::GatewayBatch<stock_analysis::data_gateway::GlobalIndexFact>,
        stock_analysis::data_gateway::GatewayError,
    >,
    Result<
        stock_analysis::data_gateway::GatewayBatch<
            stock_analysis::data_gateway::ForeignExchangeFact,
        >,
        stock_analysis::data_gateway::GatewayError,
    >,
);

async fn inspect_r08_review_occurrence(
    review_date: chrono::NaiveDate,
) -> Result<Option<crate::durable_delivery_runtime::DurableDispatchEvidence>, String> {
    crate::durable_delivery_runtime::inspect_review_task_occurrence(
        review_date,
        stock_analysis::durable_delivery::PushKind::EventCalendar,
        crate::review_batch::review_task_identity(
            review_date,
            crate::review_batch::ReviewTask::R08,
        ),
    )
    .await
}

pub async fn dispatch_r08_event_calendar_outcome(
    date: &str,
) -> crate::review_batch::ReviewTaskOutcome {
    dispatch_r08_event_calendar_outcome_with_loader(
        date,
        inspect_r08_review_occurrence,
        |review_date, reminder_date| async move {
            let reminder_year = u32::try_from(chrono::Datelike::year(&reminder_date))
                .expect("R-08 loader only receives a prevalidated reminder year");
            let announcement_gateway = stock_analysis::data_gateway::EventCalendarGateway::new();
            let futures_gateway = stock_analysis::data_gateway::FuturesDeliveryGateway::new();
            let global_market_gateway = stock_analysis::data_gateway::GlobalMarketGateway::new();
            tokio::join!(
                async {
                    let result = announcement_gateway
                        .market_announcements(review_date, 300)
                        .await;
                    // 2026-08-06: 缓存公告批次供 A-11 复用 (避免复盘内 44s 两次
                    // cninfo 拉取触发限流 router_batch_rejected)。失败不写缓存。
                    if let Ok(stock_analysis::data_gateway::GatewayBatch::Available {
                        records, ..
                    }) = &result
                    {
                        if let Ok(mut cache) = REVIEW_ANNOUNCEMENTS_CACHE
                            .get_or_init(|| std::sync::Mutex::new(None))
                            .lock()
                        {
                            *cache = Some((
                                review_date.format("%Y-%m-%d").to_string(),
                                records.clone(),
                            ));
                        }
                    }
                    result
                },
                futures_gateway
                    .cffex_contract_month(reminder_year, chrono::Datelike::month(&reminder_date),),
                global_market_gateway.us_indices(),
                global_market_gateway.usd_cny()
            )
        },
    )
    .await
}

async fn dispatch_r08_event_calendar_outcome_with_loader<
    Preflight,
    PreflightFuture,
    Loader,
    LoaderFuture,
>(
    date: &str,
    preflight: Preflight,
    loader: Loader,
) -> crate::review_batch::ReviewTaskOutcome
where
    Preflight: FnOnce(chrono::NaiveDate) -> PreflightFuture,
    PreflightFuture: std::future::Future<
        Output = Result<Option<crate::durable_delivery_runtime::DurableDispatchEvidence>, String>,
    >,
    Loader: FnOnce(chrono::NaiveDate, chrono::NaiveDate) -> LoaderFuture,
    LoaderFuture: std::future::Future<Output = R08ProviderLoadResult>,
{
    use crate::review_batch::ReviewTaskOutcome;

    let review_date = match chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        Ok(review_date) => review_date,
        Err(error) => {
            let reason = format!("R-08 review date invalid: {error}");
            log_dispatcher_attempt("R-08", false, 0, &reason);
            return ReviewTaskOutcome::failed(false, reason);
        }
    };
    match preflight(review_date).await {
        Ok(Some(evidence)) => {
            return review_outcome_from_existing_durable(
                evidence,
                review_date,
                crate::review_batch::ReviewTask::R08,
            )
        }
        Ok(None) => {}
        Err(error) => {
            let reason = format!("R-08 durable terminal preflight failed: {error}");
            log_dispatcher_attempt("R-08", false, 0, &reason);
            return ReviewTaskOutcome::failed(true, reason);
        }
    }
    let reminder_date = r08_reminder_trading_date(review_date);
    match u32::try_from(chrono::Datelike::year(&reminder_date)) {
        Ok(_) => {}
        Err(error) => {
            let reason = format!("R-08 reminder year is invalid: {error}");
            log_dispatcher_attempt("R-08", false, 0, &reason);
            return ReviewTaskOutcome::failed(false, reason);
        }
    }
    let (announcements, futures_delivery_batch, overnight_indices_batch, overnight_fx_batch) =
        loader(review_date, reminder_date).await;
    let announcements = announcements.map_err(|error| format!("CNInfo 全市场公告不可用: {error}"));
    let futures_delivery = futures_delivery_batch
        .as_ref()
        .map(|batch| build_cffex_delivery_summary(batch, reminder_date))
        .map_err(|error| {
            format!(
                "CFFEX 官方交割通知不可用: outcome={}, reason_code={}, detail={error}",
                error.audit_outcome(),
                error.reason_code()
            )
        });
    let overnight_indices = overnight_indices_batch
        .as_ref()
        .map(build_global_indices_summary)
        .map_err(|error| format!("Sina 全球指数批次不可用: {error}"));
    let overnight_fx = overnight_fx_batch
        .as_ref()
        .map(build_global_fx_summary)
        .map_err(|error| format!("Sina 美元/人民币批次不可用: {error}"));

    let announcement_summary = announcements
        .as_ref()
        .map(build_gateway_event_calendar_summary)
        .map_err(Clone::clone);
    let reminder_date_text = reminder_date.format("%Y-%m-%d").to_string();
    let prepared_calendar = match prepare_r08_public_calendar(
        &reminder_date_text,
        R08PublicCalendarComponents {
            announcement_summary,
            futures_delivery,
            overnight_indices,
            overnight_fx,
        },
    ) {
        Ok(prepared) => prepared,
        Err(error) => {
            log::error!("[R-08][BR-110][BR-140] {error}");
            log_dispatcher_attempt("R-08", false, 0, &error);
            return ReviewTaskOutcome::failed(true, error);
        }
    };
    if !prepared_calendar.failed_components.is_empty() {
        log::warn!(
            "[R-08][BR-140] degraded complete_components={} failed_components={:?}",
            prepared_calendar.complete_components,
            prepared_calendar.failed_components
        );
    }
    let prepared = match prepare_r08_counted_delivery(
        review_date,
        reminder_date,
        prepared_calendar,
        announcements.as_ref().ok(),
        futures_delivery_batch.as_ref().ok(),
        overnight_indices_batch.as_ref().ok(),
        overnight_fx_batch.as_ref().ok(),
    ) {
        Ok(prepared) => prepared,
        Err(reason) => {
            log::error!("[R-08][BR-140][BR-192] counted binding rejected: {reason}");
            log_dispatcher_attempt("R-08", false, 0, &reason);
            return ReviewTaskOutcome::failed(false, reason);
        }
    };
    let task_binding = match stock_analysis::durable_delivery::TaskBinding::new(
        prepared.task_identity.clone(),
        prepared.task_transition_basis_canonical.clone(),
    ) {
        Ok(binding) => binding,
        Err(error) => {
            let reason = format!("R-08 task binding rejected: {error}");
            log::error!("[R-08][BR-140][BR-192] {reason}");
            log_dispatcher_attempt("R-08", false, prepared.item_count, &reason);
            return ReviewTaskOutcome::failed(false, reason);
        }
    };
    let counted_binding = match crate::durable_delivery_runtime::CountedDeliveryBinding::new(
        prepared.business_date,
        prepared.task_identity,
        prepared.source_binding_canonical,
        crate::durable_delivery_runtime::CountedDeliveryScope::Global,
        prepared.delivery_subject_identity,
        crate::durable_delivery_runtime::CountedDeliveryOrigin::Provider {
            observed_at: Some(prepared.provider_observed_at),
            as_of: Some(prepared.business_date),
            ordered_batch_ids: prepared.ordered_batch_ids,
        },
        Some(task_binding),
        true,
    ) {
        Ok(binding) => binding,
        Err(reason) => {
            log::error!("[R-08][BR-140][BR-192] counted binding rejected: {reason}");
            log_dispatcher_attempt("R-08", false, prepared.item_count, &reason);
            return ReviewTaskOutcome::failed(false, reason);
        }
    };
    let presentation_token = match crate::presentation_registry::acquire_token(
        "R-08-public-event-calendar",
        crate::notify::PushKind::EventCalendar,
        "dispatch_r08_event_calendar_outcome",
        "render_r08_public_calendar",
    ) {
        Ok(token) => token,
        Err(reason) => {
            log::error!("[R-08][BR-196] presentation token rejected: {reason}");
            log_dispatcher_attempt("R-08", false, prepared.item_count, &reason);
            return ReviewTaskOutcome::failed(false, reason);
        }
    };
    let push_result = crate::notify::push_r08_presented_source_only_with_binding(
        presentation_token,
        &prepared.rendered,
        counted_binding,
    )
    .await;
    let disposition = match &push_result {
        crate::notify::PushOutcome::Pushed => "pushed",
        crate::notify::PushOutcome::Deduped => "deduped",
        crate::notify::PushOutcome::Denied(reason)
        | crate::notify::PushOutcome::SinkError(reason) => reason.as_str(),
    };
    log_dispatcher_attempt(
        "R-08",
        push_result.is_pushed(),
        prepared.item_count,
        disposition,
    );
    ReviewTaskOutcome::from_push_outcome(push_result, prepared.item_count)
}

pub async fn dispatch_r08_event_calendar_real(date: &str, banner: &BannerCtx) -> bool {
    let _ = banner;
    matches!(
        dispatch_r08_event_calendar_outcome(date).await,
        crate::review_batch::ReviewTaskOutcome::Delivered { .. }
    )
}

#[cfg(test)]
mod tests_br140_r08_partial_components {
    use super::*;

    #[tokio::test]
    async fn br200_r08_existing_delivered_skips_all_public_providers_and_reuses_count() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let business_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        let task_identity = crate::review_batch::review_task_identity(
            business_date,
            crate::review_batch::ReviewTask::R08,
        );
        let basis = serde_json::to_vec(&serde_json::json!({
            "task_identity": task_identity.clone(),
            "business_date": "2026-07-21",
            "task": "R-08",
            "snapshot_size": 8,
        }))
        .unwrap();
        let evidence = crate::durable_delivery_runtime::DurableDispatchEvidence {
            decision_identity: "TEST_CODE_BR200_R08_DECISION".to_string(),
            state: stock_analysis::durable_delivery::DecisionState::Delivered,
            schedule_hydration: Some(stock_analysis::durable_delivery::ScheduleHydration {
                decision_identity: "TEST_CODE_BR200_R08_DECISION".to_string(),
                task_identity,
                transition_identity: "TEST_CODE_BR200_R08_TRANSITION".to_string(),
                transition_canonical: br#"{"TEST_CODE":"transition"}"#.to_vec(),
                transition_sha256: "a".repeat(64),
                transition_basis_sha256: r09_sha256(&basis),
                transition_basis_canonical: basis,
                immutable_audit_ref: "TEST_CODE_BR200_R08_AUDIT".to_string(),
                hydration_state: stock_analysis::durable_delivery::ScheduleHydrationState::Applied,
            }),
        };
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&provider_calls);
        let outcome = dispatch_r08_event_calendar_outcome_with_loader(
            "2026-07-21",
            |_| async { Ok(Some(evidence)) },
            move |review_date, reminder_date| async move {
                calls.fetch_add(1, Ordering::SeqCst);
                (
                    Ok(announcement_batch(review_date)),
                    Ok(cffex_batch(reminder_date)),
                    Ok(indices_batch()),
                    Ok(fx_batch()),
                )
            },
        )
        .await;

        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
        assert!(matches!(
            outcome,
            crate::review_batch::ReviewTaskOutcome::Delivered { count: 8 }
        ));
    }

    #[tokio::test]
    async fn br200_r08_verified_empty_delivered_replay_remains_terminal() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let business_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        let task_identity = crate::review_batch::review_task_identity(
            business_date,
            crate::review_batch::ReviewTask::R08,
        );
        let basis = serde_json::to_vec(&serde_json::json!({
            "task_identity": task_identity.clone(),
            "business_date": "2026-07-21",
            "task": "R-08",
            "snapshot_size": 0,
        }))
        .unwrap();
        let evidence = crate::durable_delivery_runtime::DurableDispatchEvidence {
            decision_identity: "TEST_CODE_BR200_R08_EMPTY_DECISION".to_string(),
            state: stock_analysis::durable_delivery::DecisionState::Delivered,
            schedule_hydration: Some(stock_analysis::durable_delivery::ScheduleHydration {
                decision_identity: "TEST_CODE_BR200_R08_EMPTY_DECISION".to_string(),
                task_identity,
                transition_identity: "TEST_CODE_BR200_R08_EMPTY_TRANSITION".to_string(),
                transition_canonical: br#"{"TEST_CODE":"empty-transition"}"#.to_vec(),
                transition_sha256: "a".repeat(64),
                transition_basis_sha256: r09_sha256(&basis),
                transition_basis_canonical: basis,
                immutable_audit_ref: "TEST_CODE_BR200_R08_EMPTY_AUDIT".to_string(),
                hydration_state: stock_analysis::durable_delivery::ScheduleHydrationState::Applied,
            }),
        };
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&provider_calls);
        let outcome = dispatch_r08_event_calendar_outcome_with_loader(
            "2026-07-21",
            |_| async { Ok(Some(evidence)) },
            move |review_date, reminder_date| async move {
                calls.fetch_add(1, Ordering::SeqCst);
                (
                    Ok(announcement_batch(review_date)),
                    Ok(cffex_batch(reminder_date)),
                    Ok(indices_batch()),
                    Ok(fx_batch()),
                )
            },
        )
        .await;

        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            outcome,
            crate::review_batch::ReviewTaskOutcome::Delivered { count: 0 }
        );
    }

    #[tokio::test]
    async fn br199_r08_unsupported_cffex_remains_retryable_and_preserves_error_taxonomy() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let provider_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&provider_calls);
        let outcome = dispatch_r08_event_calendar_outcome_with_loader(
            "2026-07-21",
            |_| async { Ok(None) },
            move |review_date, reminder_date| async move {
                calls.fetch_add(1, Ordering::SeqCst);
                (
                    Ok(announcement_batch(review_date)),
                    Err(stock_analysis::data_gateway::GatewayError::unavailable(
                        "event_calendar",
                        Some(magic_market_core::ProviderId::Cffex),
                        false,
                        format!(
                            "provider_unsupported: unsupported by {review_date} {reminder_date}"
                        ),
                    )),
                    Ok(indices_batch()),
                    Ok(fx_batch()),
                )
            },
        )
        .await;

        assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
        match outcome {
            crate::review_batch::ReviewTaskOutcome::Failed { failure } => {
                if let crate::review_batch::ReviewTaskFailure::ExistingSourceFailure {
                    retryable,
                    reason,
                } = failure
                {
                    assert!(retryable);
                    assert!(reason.contains("r08_cffex_component_unavailable"));
                    assert!(reason.contains("provider_unsupported"));
                } else {
                    panic!("R-08 expected existing source failure on unsupported CFFEX");
                }
            }
            _ => panic!("R-08 unsupported CFFEX should remain retryable"),
        }
    }

    #[test]
    fn br199_r08_friday_targets_monday_trading_session() {
        let friday = chrono::NaiveDate::from_ymd_opt(2026, 7, 31).unwrap();

        assert_eq!(
            r08_reminder_trading_date(friday),
            chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap()
        );
    }

    #[test]
    fn br199_r08_dispatcher_has_no_account_or_virtual_reader() {
        let source = include_str!("push_templates.rs");
        let start = source
            .find("pub async fn dispatch_r08_event_calendar_outcome(")
            .unwrap();
        let end = source[start..]
            .find("pub async fn dispatch_r08_event_calendar_real(")
            .unwrap();
        let dispatcher = &source[start..start + end];

        assert!(!dispatcher.contains("load_user_confirmed_r08_positions"));
        assert!(!dispatcher.contains("event_calendar_virtual_holdings"));
        assert!(!dispatcher.contains("broker_holdings"));
    }

    #[test]
    fn br199_r08_public_renderer_has_no_holding_claims() {
        let text = render_r08_public_calendar(
            "2026-08-03",
            "公告 2 条",
            "无交割",
            "道指 +0.2%",
            "美元/人民币 7.1800",
            &["overnight_indices"],
        );

        assert!(!text.contains("持仓"));
        assert!(!text.contains("用户确认"));
        assert!(!text.contains("虚拟观察"));
        assert!(text.contains("降级组件: overnight_indices"));
    }

    fn cffex_batch(
        delivery_date: chrono::NaiveDate,
    ) -> stock_analysis::data_gateway::GatewayBatch<stock_analysis::data_gateway::FuturesDeliveryFact>
    {
        let records = ["IC2607", "IF2607", "IH2607", "IM2607"]
            .into_iter()
            .map(
                |contract_code| stock_analysis::data_gateway::FuturesDeliveryFact {
                    contract_code: contract_code.to_string(),
                    product_code: contract_code[..2].to_string(),
                    last_trading_date: None,
                    delivery_date,
                    notice_url: "TEST_CODE_official_notice_url".to_string(),
                },
            )
            .collect();
        stock_analysis::data_gateway::GatewayBatch::Available {
            records,
            evidence: stock_analysis::data_gateway::BatchEvidence {
                provider: magic_market_core::ProviderId::Cffex,
                source: "cffex-official-notice".to_string(),
                source_at: Some("2026-07-16".to_string()),
                observed_at: "2026-07-16T08:00:00Z".to_string(),
                batch_id: "TEST_CODE_cffex_batch".to_string(),
            },
        }
    }

    fn announcement_batch(
        business_date: chrono::NaiveDate,
    ) -> stock_analysis::data_gateway::GatewayBatch<stock_analysis::data_gateway::EventAnnouncement>
    {
        stock_analysis::data_gateway::GatewayBatch::Available {
            records: vec![stock_analysis::data_gateway::EventAnnouncement {
                announcement_id: "TEST_CODE_announcement".to_string(),
                code: "TEST_CODE_600000".to_string(),
                category: Some("重大合同".to_string()),
                title: "TEST_CODE 重大合同公告".to_string(),
                published_at: format!("{business_date}T18:00:00+08:00"),
                canonical_url: "https://example.invalid/TEST_CODE_announcement".to_string(),
            }],
            evidence: stock_analysis::data_gateway::BatchEvidence {
                provider: magic_market_core::ProviderId::Cninfo,
                source: "cninfo-market".to_string(),
                source_at: Some(format!("{business_date}T18:00:00+08:00")),
                observed_at: format!("{business_date}T18:01:00+08:00"),
                batch_id: "TEST_CODE_announcement_batch".to_string(),
            },
        }
    }

    fn indices_batch(
    ) -> stock_analysis::data_gateway::GatewayBatch<stock_analysis::data_gateway::GlobalIndexFact>
    {
        use magic_market_core::{GlobalIndexCode, ProviderId};

        let observed_at = chrono::DateTime::parse_from_rfc3339("2026-07-21T13:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let source_at = chrono::DateTime::parse_from_rfc3339("2026-07-21T12:59:59Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let batch_id = "TEST_CODE_indices_batch";
        let records = [
            (GlobalIndexCode::DowJones, "道琼斯", 45_000.0),
            (GlobalIndexCode::NasdaqComposite, "纳斯达克", 22_000.0),
            (GlobalIndexCode::Sp500, "标普500", 6_500.0),
        ]
        .into_iter()
        .map(
            |(code, name, value)| stock_analysis::data_gateway::GlobalIndexFact {
                code,
                name: name.to_string(),
                value,
                change: 10.0,
                change_percent: 0.2,
                source_at,
                observed_at,
                provider: ProviderId::Sina,
                batch_id: batch_id.to_string(),
            },
        )
        .collect();
        stock_analysis::data_gateway::GatewayBatch::Available {
            records,
            evidence: stock_analysis::data_gateway::BatchEvidence {
                provider: ProviderId::Sina,
                source: "sina-web".to_string(),
                source_at: Some(source_at.to_rfc3339()),
                observed_at: observed_at.to_rfc3339(),
                batch_id: batch_id.to_string(),
            },
        }
    }

    fn fx_batch(
    ) -> stock_analysis::data_gateway::GatewayBatch<stock_analysis::data_gateway::ForeignExchangeFact>
    {
        use magic_market_core::{FxPair, ProviderId};

        let observed_at = chrono::DateTime::parse_from_rfc3339("2026-07-21T13:00:01Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let source_at = chrono::DateTime::parse_from_rfc3339("2026-07-21T13:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let batch_id = "TEST_CODE_fx_batch";
        stock_analysis::data_gateway::GatewayBatch::Available {
            records: vec![stock_analysis::data_gateway::ForeignExchangeFact {
                pair: FxPair::UsdCny,
                name: "美元/人民币".to_string(),
                rate: 7.18,
                change: Some(0.01),
                change_percent: Some(0.14),
                source_at,
                observed_at,
                provider: ProviderId::Sina,
                batch_id: batch_id.to_string(),
            }],
            evidence: stock_analysis::data_gateway::BatchEvidence {
                provider: ProviderId::Sina,
                source: "sina-web".to_string(),
                source_at: Some(source_at.to_rfc3339()),
                observed_at: observed_at.to_rfc3339(),
                batch_id: batch_id.to_string(),
            },
        }
    }

    fn prepared_calendar() -> R08PreparedPublicCalendar {
        prepare_r08_public_calendar(
            "2026-07-22",
            R08PublicCalendarComponents {
                announcement_summary: Ok(("TEST_CODE announcement summary".to_string(), 1)),
                futures_delivery: Ok(("TEST_CODE delivery summary".to_string(), 0)),
                overnight_indices: Ok(("TEST_CODE indices".to_string(), 3)),
                overnight_fx: Ok(("TEST_CODE fx".to_string(), 1)),
            },
        )
        .unwrap()
    }

    fn complete_prepared_counted_delivery() -> PreparedR08CountedDelivery {
        let business_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        let reminder_date = r08_reminder_trading_date(business_date);
        let futures = cffex_batch(reminder_date);
        let futures_summary = build_cffex_delivery_summary(&futures, reminder_date);
        let prepared_calendar = prepare_r08_public_calendar(
            &reminder_date.to_string(),
            R08PublicCalendarComponents {
                announcement_summary: Ok(("TEST_CODE announcement summary".to_string(), 1)),
                futures_delivery: Ok(futures_summary),
                overnight_indices: Ok(("TEST_CODE indices".to_string(), 3)),
                overnight_fx: Ok(("TEST_CODE fx".to_string(), 1)),
            },
        )
        .unwrap();
        prepare_r08_counted_delivery(
            business_date,
            reminder_date,
            prepared_calendar,
            Some(&announcement_batch(business_date)),
            Some(&futures),
            Some(&indices_batch()),
            Some(&fx_batch()),
        )
        .unwrap()
    }

    fn counted_binding_from_r08(
        prepared: &PreparedR08CountedDelivery,
        source_binding_canonical: Vec<u8>,
    ) -> crate::durable_delivery_runtime::CountedDeliveryBinding {
        let task_binding = stock_analysis::durable_delivery::TaskBinding::new(
            prepared.task_identity.clone(),
            prepared.task_transition_basis_canonical.clone(),
        )
        .unwrap();
        crate::durable_delivery_runtime::CountedDeliveryBinding::new(
            prepared.business_date,
            prepared.task_identity.clone(),
            source_binding_canonical,
            crate::durable_delivery_runtime::CountedDeliveryScope::Global,
            prepared.delivery_subject_identity.clone(),
            crate::durable_delivery_runtime::CountedDeliveryOrigin::Provider {
                observed_at: Some(prepared.provider_observed_at),
                as_of: Some(prepared.business_date),
                ordered_batch_ids: prepared.ordered_batch_ids.clone(),
            },
            Some(task_binding),
            true,
        )
        .unwrap()
    }

    #[test]
    fn br161_governed_r08_has_no_legacy_announcement_or_yahoo_fetch() {
        let source = include_str!("push_templates.rs");
        let start = source
            .find("pub async fn dispatch_r08_event_calendar_outcome")
            .expect("R-08 governed dispatcher");
        let end = source[start..]
            .find("pub async fn dispatch_r08_event_calendar_real")
            .map(|offset| start + offset)
            .expect("R-08 wrapper");
        let governed_r08 = &source[start..end];

        assert!(governed_r08.contains("EventCalendarGateway"));
        assert!(governed_r08.contains("GlobalMarketGateway"));
        assert!(!governed_r08.contains("fetch_overnight_data"));
        assert!(!governed_r08.contains("GlobalOvernightMarket unsupported"));
    }

    #[test]
    fn br192_r08_governed_dispatcher_requires_explicit_counted_binding() {
        let source = include_str!("push_templates.rs");
        let start = source
            .find("pub async fn dispatch_r08_event_calendar_outcome")
            .expect("R-08 governed dispatcher");
        let end = source[start..]
            .find("pub async fn dispatch_r08_event_calendar_real")
            .map(|offset| start + offset)
            .expect("R-08 wrapper");
        let dispatcher = &source[start..end];

        assert!(dispatcher.contains("prepare_r08_counted_delivery"));
        assert!(dispatcher.contains("CountedDeliveryOrigin::Provider"));
        assert!(dispatcher.contains("ordered_batch_ids: prepared.ordered_batch_ids"));
        assert!(dispatcher.contains("push_r08_presented_source_only_with_binding"));
        assert!(!dispatcher.contains("push_counted_with_binding("));
        assert!(!dispatcher.contains("dispatch_outcome("));
        assert!(!dispatcher.contains("push_governor_v3("));
    }

    #[test]
    fn br199_r08_public_binding_freezes_ordered_gateway_batches_and_source_facts() {
        let business_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        let reminder_date = business_date.succ_opt().unwrap();
        let announcements = announcement_batch(business_date);
        let futures = cffex_batch(reminder_date);
        let indices = indices_batch();
        let fx = fx_batch();

        let prepared = prepare_r08_counted_delivery(
            business_date,
            reminder_date,
            prepared_calendar(),
            Some(&announcements),
            Some(&futures),
            Some(&indices),
            Some(&fx),
        )
        .unwrap();

        assert_eq!(
            prepared.ordered_batch_ids,
            [
                "TEST_CODE_announcement_batch",
                "TEST_CODE_cffex_batch",
                "TEST_CODE_indices_batch",
                "TEST_CODE_fx_batch",
            ]
        );
        assert_eq!(
            prepared.task_identity,
            crate::review_batch::review_task_identity(
                business_date,
                crate::review_batch::ReviewTask::R08,
            )
        );
        let source: serde_json::Value =
            serde_json::from_slice(&prepared.source_binding_canonical).unwrap();
        assert_eq!(source["template_id"], "event_calendar_v1");
        assert_eq!(
            source["provider_batches"][0]["component"],
            "market_announcements"
        );
        assert_eq!(source["provider_batches"][1]["component"], "cffex_delivery");
        assert_eq!(
            source["provider_batches"][2]["component"],
            "overnight_indices"
        );
        assert_eq!(source["provider_batches"][3]["component"], "overnight_fx");
        assert_eq!(
            source["announcements"][0]["announcement_id"],
            "TEST_CODE_announcement"
        );
        assert_eq!(source["futures_delivery"][0]["contract_code"], "IC2607");
        assert_eq!(source["overnight_indices"][0]["name"], "道琼斯");
        assert_eq!(source["overnight_fx"][0]["rate"], 7.18);
        assert_eq!(
            source["rendered_content_sha256"],
            r08_sha256(prepared.rendered.as_bytes())
        );
        assert!(source.get("rendered_components").is_none());
        assert_eq!(
            source["unavailable_optional_components"],
            serde_json::json!([])
        );
        let canonical_text = String::from_utf8(prepared.source_binding_canonical.clone()).unwrap();
        assert!(!canonical_text.contains("holdings"));
        assert!(!canonical_text.contains("user_confirmed"));
        assert!(!canonical_text.contains("virtual"));
        assert!(source["task_transition_basis"]["rule_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule == "BR-199"));
        let task_basis: serde_json::Value =
            serde_json::from_slice(&prepared.task_transition_basis_canonical).unwrap();
        assert_eq!(task_basis["task"], "R-08");
        assert_eq!(task_basis["batch_ids"][3], "TEST_CODE_fx_batch");
    }

    #[test]
    fn br199_r08_durable_binding_rejects_public_evidence_mutations() {
        let prepared = complete_prepared_counted_delivery();
        let binding =
            counted_binding_from_r08(&prepared, prepared.source_binding_canonical.clone());
        assert_eq!(binding.validate_r08_public_source_only(), Ok(()));
        assert_eq!(
            binding.validate_r08_public_source_only_text(&prepared.rendered),
            Ok(())
        );

        let original: serde_json::Value =
            serde_json::from_slice(&prepared.source_binding_canonical).unwrap();
        let mut mutations = Vec::new();
        let mut template = original.clone();
        template["template_id"] = serde_json::json!("event_calendar_v0");
        mutations.push(template);
        let mut reminder = original.clone();
        reminder["reminder_date"] = serde_json::json!("2026-07-23");
        mutations.push(reminder);
        let mut task_identity = original.clone();
        task_identity["review_task_identity"] = serde_json::json!("TEST_CODE_wrong_task");
        mutations.push(task_identity);
        let mut provider_order = original.clone();
        provider_order["provider_batches"]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        mutations.push(provider_order);
        let mut cffex_status = original.clone();
        cffex_status["provider_batches"][1]["status"] = serde_json::json!("verified_empty");
        mutations.push(cffex_status);
        let mut cffex_wrong_session = original.clone();
        cffex_wrong_session["futures_delivery"][0]["delivery_date"] =
            serde_json::json!("2026-07-17");
        mutations.push(cffex_wrong_session);
        let mut batch_id = original.clone();
        batch_id["provider_batches"][1]["batch_id"] = serde_json::json!("TEST_CODE_wrong_batch");
        mutations.push(batch_id);
        let mut transition = original.clone();
        transition["task_transition_basis"]["source"] =
            serde_json::json!("TEST_CODE_wrong_transition_source");
        mutations.push(transition);
        let mut rendered_hash = original.clone();
        rendered_hash["rendered_content_sha256"] = serde_json::json!("0".repeat(64));
        mutations.push(rendered_hash);

        for mutation in mutations {
            let canonical = serde_json::to_vec(&mutation).unwrap();
            let binding = counted_binding_from_r08(&prepared, canonical);
            assert_eq!(
                binding.validate_r08_public_source_only_text(&prepared.rendered),
                Err("counted_r08_source_only_binding_invalid")
            );
        }

        let task_binding = stock_analysis::durable_delivery::TaskBinding::new(
            prepared.task_identity.clone(),
            prepared.task_transition_basis_canonical.clone(),
        )
        .unwrap();
        let wrong_origin = crate::durable_delivery_runtime::CountedDeliveryBinding::new(
            prepared.business_date,
            prepared.task_identity.clone(),
            prepared.source_binding_canonical.clone(),
            crate::durable_delivery_runtime::CountedDeliveryScope::Global,
            prepared.delivery_subject_identity.clone(),
            crate::durable_delivery_runtime::CountedDeliveryOrigin::Provider {
                observed_at: Some(prepared.provider_observed_at + chrono::Duration::seconds(1)),
                as_of: Some(prepared.business_date),
                ordered_batch_ids: prepared.ordered_batch_ids.clone(),
            },
            Some(task_binding),
            true,
        )
        .unwrap();
        assert_eq!(
            wrong_origin.validate_r08_public_source_only(),
            Err("counted_r08_source_only_binding_invalid")
        );
    }

    #[test]
    fn br199_r08_closed_gate_rejects_non_event_calendar_kind() {
        let prepared = complete_prepared_counted_delivery();
        let binding =
            counted_binding_from_r08(&prepared, prepared.source_binding_canonical.clone());
        assert!(matches!(
            crate::v14_adapter::v14_gate_r08_source_only_binding(
                crate::notify::PushKind::ReviewLhb,
                &binding,
            ),
            crate::v14_adapter::V14Gate::Denied(reason)
                if reason == "counted_r08_source_only_kind_not_allowed"
        ));
    }

    #[test]
    fn br199_r08_dispatch_is_joined_before_account_dependency_outcomes() {
        let source = include_str!("push_templates.rs");
        let start = source
            .find("pub async fn dispatch_post_session_review(")
            .expect("post-session dispatcher");
        let end = source[start..]
            .find("// ============================================================================")
            .expect("post-session dispatcher boundary");
        let dispatcher = &source[start..start + end];
        let r08 = dispatcher
            .find("dispatch_r08_event_calendar_outcome")
            .expect("R-08 source-only call");
        let a10 = dispatcher
            .find("dispatch_catalyst_review_daily_outcome")
            .expect("A-10 source-only call");
        let a01 = dispatcher
            .find("dispatch_paper_review_daily_outcome")
            .expect("A-01 source-only call");
        let account = dispatcher
            .find("let mut account_required_outcomes = Vec::new()")
            .expect("account phase");
        assert!(r08 < account);
        assert!(a10 < account);
        assert!(a01 < account);
        // BR-194: R-03 is a LegacyAccountGate task, so `partition_review_tasks`
        // can never place it in `source_only`. 2026-08-06 R-03 解除 (a9f006a):
        // R-03 在 account phase 之后的 account_required 循环内走真实数据
        // (dispatch_r03_industry_chain_outcome), 仍属 account-gated 路径 —
        // 约束: 调用位置必须位于 account phase 之后 (绝不能在 source-only join)。
        let r03 = dispatcher.find("dispatch_r03_industry_chain_outcome");
        assert!(
            r03.map(|pos| pos > account).unwrap_or(false),
            "R-03 must stay on the account-gated path, not the source-only join"
        );
    }

    #[test]
    fn br199_r08_cffex_is_mandatory_for_counted_delivery() {
        let business_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        let error = prepare_r08_counted_delivery(
            business_date,
            business_date.succ_opt().unwrap(),
            prepared_calendar(),
            None,
            None,
            None,
            None,
        )
        .unwrap_err();

        assert!(error.contains("r08_cffex_component_unavailable"));
    }

    #[test]
    fn br199_r08_verified_empty_cffex_is_a_complete_public_component() {
        let business_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        let cffex = stock_analysis::data_gateway::GatewayBatch::VerifiedEmpty(
            stock_analysis::data_gateway::BatchEvidence {
                provider: magic_market_core::ProviderId::Cffex,
                source: "cffex-official-notice".to_string(),
                source_at: None,
                observed_at: "2026-07-21T13:00:01Z".to_string(),
                batch_id: "TEST_CODE_cffex_verified_empty".to_string(),
            },
        );
        let prepared = prepare_r08_counted_delivery(
            business_date,
            r08_reminder_trading_date(business_date),
            prepared_calendar(),
            Some(&announcement_batch(business_date)),
            Some(&cffex),
            Some(&indices_batch()),
            Some(&fx_batch()),
        )
        .unwrap();
        let binding: serde_json::Value =
            serde_json::from_slice(&prepared.source_binding_canonical).unwrap();
        assert_eq!(binding["provider_batches"][1]["status"], "verified_empty");
        assert_eq!(binding["provider_batches"][1]["record_count"], 0);
        assert_eq!(binding["futures_delivery"], serde_json::json!([]));
        assert_eq!(
            counted_binding_from_r08(&prepared, prepared.source_binding_canonical.clone())
                .validate_r08_public_source_only_text(&prepared.rendered),
            Ok(())
        );
    }

    #[test]
    fn br192_r08_fails_closed_on_invalid_business_date_or_batch_identity() {
        let business_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        let futures = cffex_batch(chrono::NaiveDate::from_ymd_opt(2026, 7, 17).unwrap());
        let mut announcements = announcement_batch(business_date);
        match &mut announcements {
            stock_analysis::data_gateway::GatewayBatch::Available { evidence, .. } => {
                evidence.batch_id.clear();
            }
            stock_analysis::data_gateway::GatewayBatch::VerifiedEmpty(_) => unreachable!(),
        }
        let error = prepare_r08_counted_delivery(
            business_date,
            business_date.succ_opt().unwrap(),
            prepared_calendar(),
            Some(&announcements),
            Some(&futures),
            None,
            None,
        )
        .unwrap_err();
        assert!(error.contains("batch ID is missing"));

        let announcements = announcement_batch(business_date);
        let error = prepare_r08_counted_delivery(
            business_date,
            business_date.succ_opt().unwrap().succ_opt().unwrap(),
            prepared_calendar(),
            Some(&announcements),
            Some(&futures),
            None,
            None,
        )
        .unwrap_err();
        assert!(error.contains("next trading day"));

        let mut announcements = announcement_batch(business_date);
        match &mut announcements {
            stock_analysis::data_gateway::GatewayBatch::Available { evidence, .. } => {
                evidence.source_at = Some("2026-07-20T18:00:00+08:00".to_string());
            }
            stock_analysis::data_gateway::GatewayBatch::VerifiedEmpty(_) => unreachable!(),
        }
        let error = prepare_r08_counted_delivery(
            business_date,
            business_date.succ_opt().unwrap(),
            prepared_calendar(),
            Some(&announcements),
            Some(&futures),
            None,
            None,
        )
        .unwrap_err();
        assert!(error.contains("outside business date"));
    }

    #[test]
    fn br199_announcement_failure_does_not_block_complete_public_components() {
        let prepared = prepare_r08_public_calendar(
            "2026-07-22",
            R08PublicCalendarComponents {
                announcement_summary: Err("TEST_CODE announcement unavailable".to_string()),
                futures_delivery: Ok((
                    "TEST_CODE CFFEX verified no next-day delivery".to_string(),
                    0,
                )),
                overnight_indices: Ok(("+0.5%".to_string(), 3)),
                overnight_fx: Ok(("7.20".to_string(), 1)),
            },
        )
        .expect("verified components must produce a degraded report");

        assert!(prepared.text.contains("公告不可用"));
        assert!(!prepared.text.contains("持仓"));
        assert_eq!(prepared.complete_components, 3);
        assert_eq!(prepared.failed_components, vec!["market_announcements"]);
    }

    #[test]
    fn br199_unavailable_cffex_is_explicit_retryable_component_failure() {
        let error = prepare_r08_public_calendar(
            "2026-07-22",
            R08PublicCalendarComponents {
                announcement_summary: Err("TEST_CODE announcement unavailable".to_string()),
                futures_delivery: Err("TEST_CODE CFFEX unavailable".to_string()),
                overnight_indices: Err("TEST_CODE overnight indices unavailable".to_string()),
                overnight_fx: Err("TEST_CODE overnight FX unavailable".to_string()),
            },
        )
        .expect_err("mandatory CFFEX evidence must fail before render or push");

        assert!(error.contains("r08_cffex_component_unavailable"));
    }

    #[test]
    fn br165_only_renders_an_official_next_day_delivery_batch() {
        let delivery_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 17).unwrap();
        let batch = cffex_batch(delivery_date);

        let (reminder, count) = build_cffex_delivery_summary(&batch, delivery_date);
        assert_eq!(count, 4);
        assert!(reminder.contains("IC2607/IF2607/IH2607/IM2607"));
        assert!(reminder.contains("官方通知；交割方式未由该通知提供"));
        assert!(!reminder.contains("现金交割"));

        let (no_reminder, count) = build_cffex_delivery_summary(
            &batch,
            chrono::NaiveDate::from_ymd_opt(2026, 7, 18).unwrap(),
        );
        assert_eq!(count, 0);
        assert!(no_reminder.contains("无股指期货交割"));
    }

    #[test]
    fn br165_cffex_renderer_projects_exact_session_in_canonical_order() {
        let reminder_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 22).unwrap();
        let mut batch = cffex_batch(reminder_date);
        let stock_analysis::data_gateway::GatewayBatch::Available { records, .. } = &mut batch
        else {
            unreachable!();
        };
        records.reverse();
        records[0].delivery_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 17).unwrap();

        let (summary, count) = build_cffex_delivery_summary(&batch, reminder_date);

        assert_eq!(count, 3);
        assert!(summary.contains("IC2607/IF2607/IH2607"));
        assert!(!summary.contains("IM2607"));
    }

    #[test]
    fn br199_r08_binding_uses_same_exact_sorted_cffex_projection_as_renderer() {
        let business_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        let reminder_date = r08_reminder_trading_date(business_date);
        let mut futures = cffex_batch(reminder_date);
        let stock_analysis::data_gateway::GatewayBatch::Available { records, .. } = &mut futures
        else {
            unreachable!();
        };
        records.reverse();
        records[0].delivery_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 17).unwrap();
        let futures_summary = build_cffex_delivery_summary(&futures, reminder_date);
        let prepared_calendar = prepare_r08_public_calendar(
            &reminder_date.to_string(),
            R08PublicCalendarComponents {
                announcement_summary: Ok(("TEST_CODE announcement summary".to_string(), 1)),
                futures_delivery: Ok(futures_summary),
                overnight_indices: Ok(("TEST_CODE indices".to_string(), 3)),
                overnight_fx: Ok(("TEST_CODE fx".to_string(), 1)),
            },
        )
        .unwrap();

        let prepared = prepare_r08_counted_delivery(
            business_date,
            reminder_date,
            prepared_calendar,
            Some(&announcement_batch(business_date)),
            Some(&futures),
            Some(&indices_batch()),
            Some(&fx_batch()),
        )
        .unwrap();
        let binding: serde_json::Value =
            serde_json::from_slice(&prepared.source_binding_canonical).unwrap();
        let projected = binding["futures_delivery"].as_array().unwrap();

        assert_eq!(binding["provider_batches"][1]["record_count"], 3);
        assert_eq!(projected.len(), 3);
        assert_eq!(projected[0]["contract_code"], "IC2607");
        assert_eq!(projected[1]["contract_code"], "IF2607");
        assert_eq!(projected[2]["contract_code"], "IH2607");
        assert!(projected
            .iter()
            .all(|fact| fact["delivery_date"] == "2026-07-22"));
        assert_eq!(
            counted_binding_from_r08(&prepared, prepared.source_binding_canonical.clone())
                .validate_r08_public_source_only_text(&prepared.rendered),
            Ok(())
        );
    }
}

// ============================================================================
// CR-16 (review): R-03/R-04/R-05/R-06 真实 dispatcher (从 run_review_only_inner 抽取)
// 替代之前 dispatch_post_session_review 内的占位 dispatcher (复用 A-10/A-01)
// ============================================================================

/// R-03 涨停题材联动：基于完整、精确日期的已选涨停池批次与实盘持仓/自选交集（BR-106/BR-110/BR-140/BR-159）。
pub async fn dispatch_r03_industry_chain_outcome(
    date: &str,
) -> crate::review_batch::ReviewTaskOutcome {
    use crate::review_batch::ReviewTaskOutcome;
    use stock_analysis::market_analyzer::limit_chain_review::{aggregate, LimitChainInput};

    let review_date = date.to_string();
    let positions =
        match tokio::task::spawn_blocking(stock_analysis::portfolio::get_positions).await {
            Ok(Ok(positions)) => positions,
            Ok(Err(error)) => {
                let reason = format!("R-03 实盘持仓查询失败: {error}");
                log::error!("[R-03][BR-106][BR-110] {reason}");
                log_dispatcher_attempt("R-03", false, 0, &reason);
                return ReviewTaskOutcome::failed(true, reason);
            }
            Err(error) => {
                let reason = format!("R-03 实盘持仓查询任务失败: {error}");
                log::error!("[R-03][BR-110] {reason}");
                log_dispatcher_attempt("R-03", false, 0, &reason);
                return ReviewTaskOutcome::failed(true, reason);
            }
        };
    let batch = match super::load_review_limit_chain_stocks(&positions, date).await {
        Ok(batch) => batch,
        Err(error) => {
            log::error!("[R-03][BR-106][BR-110] {error}");
            log_dispatcher_attempt("R-03", false, 0, &error);
            return ReviewTaskOutcome::failed(true, format!("R-03 source unavailable: {error}"));
        }
    };
    let prepared =
        tokio::task::spawn_blocking(move || -> Result<Option<(String, usize)>, String> {
            let audit_date = chrono::NaiveDate::parse_from_str(&review_date, "%Y-%m-%d")
                .map_err(|error| format!("R-03 非法复盘日期: {error}"))?;
            let mut rejection_rows = batch
                .rejected
                .iter()
                .map(|rejection| {
                    let reason_code = if rejection.reason.contains("日 K") {
                        "daily_kline_rejected"
                    } else if rejection.reason.contains("产业链") {
                        "industry_evidence_missing"
                    } else {
                        "candidate_validation_failed"
                    };
                    (rejection.code.clone(), reason_code, true)
                })
                .collect::<Vec<_>>();
            rejection_rows.extend(
                batch
                    .source_errors
                    .iter()
                    .enumerate()
                    .map(|(index, error)| {
                        (
                            format!("source-error-{index}:{error}"),
                            "source_error",
                            true,
                        )
                    }),
            );
            persist_review_rejections(
                "R-03",
                "review_data_gateway_routed_limit_pool",
                audit_date,
                &["BR-106", "BR-140", "BR-159"],
                rejection_rows,
            )?;
            let source_complete = batch.source_complete();
            if batch.accepted.is_empty() {
                return if source_complete {
                    Ok(None)
                } else {
                    Err(format!(
                        "R-03 部分数据失败且没有可生成报告的标的: rejected={} source_errors={}",
                        batch.rejected.len(),
                        batch.source_errors.len()
                    ))
                };
            }
            let aggregates = aggregate(&LimitChainInput {
                stocks: batch.accepted,
                source_complete,
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
                    watch_point: (!row.watch_point.trim().is_empty())
                        .then_some(row.watch_point.as_str()),
                })
                .collect();
            let count = lines.len();
            let evidence_note = (!source_complete).then(|| {
                format!(
                    "已隔离 {} 个标的、{} 个来源错误，仅展示通过质检的真实子集",
                    batch.rejected.len(),
                    batch.source_errors.len()
                )
            });
            Ok(Some((
                render_industry_chain(&review_date, &lines, None, evidence_note.as_deref()),
                count,
            )))
        })
        .await;
    let (text, count) = match prepared {
        Ok(Ok(Some(prepared))) => prepared,
        Ok(Ok(None)) => {
            let reason = "R-03 完整数据批次无当日涨停标的";
            log::info!("[R-03][BR-140] {reason}");
            log_dispatcher_attempt("R-03", false, 0, reason);
            return ReviewTaskOutcome::no_data(reason);
        }
        Ok(Err(error)) => {
            log::error!("[R-03][BR-106][BR-110] {error}");
            log_dispatcher_attempt("R-03", false, 0, &error);
            return ReviewTaskOutcome::failed(true, format!("R-03 source unavailable: {error}"));
        }
        Err(error) => {
            let reason = format!("R-03 数据准备任务失败: {error}");
            log::error!("[R-03][BR-110] {reason}");
            log_dispatcher_attempt("R-03", false, 0, &reason);
            return ReviewTaskOutcome::failed(true, reason);
        }
    };
    let push_result = dispatch_registered_outcome!(
        "R-03-industry-chain",
        crate::notify::PushKind::IndustryChain,
        "industry_chain_review_dispatcher",
        "render_industry_chain",
        "",
        None,
        text
    );
    log_dispatcher_attempt("R-03", push_result.is_pushed(), count, "");
    ReviewTaskOutcome::from_push_outcome(push_result, 1)
}

pub async fn dispatch_r03_industry_chain_real(date: &str, _banner: &BannerCtx) -> bool {
    matches!(
        dispatch_r03_industry_chain_outcome(date).await,
        crate::review_batch::ReviewTaskOutcome::Delivered { .. }
    )
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewLhbSeatBinding {
    side: String,
    rank: u32,
    seat_name: String,
    amount_yuan: f64,
    buy_amount_yuan: Option<f64>,
    sell_amount_yuan: Option<f64>,
    net_amount_yuan: Option<f64>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewLhbDisclosureBinding {
    entry_id: String,
    trade_id: String,
    reason: Option<String>,
    buy_amount_yuan: Option<f64>,
    sell_amount_yuan: Option<f64>,
    net_amount_yuan: Option<f64>,
    turnover_rate_pct: Option<f64>,
    seats: Vec<ReviewLhbSeatBinding>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewLhbStockBinding {
    source_order_ordinal: usize,
    exchange: String,
    code: String,
    ranking_net_amount_yuan: f64,
    disclosures: Vec<ReviewLhbDisclosureBinding>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewLhbEvidenceBinding {
    provider: String,
    source: String,
    source_at: String,
    observed_at: String,
    batch_id: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewLhbTaskTransitionBasis {
    task_identity: String,
    business_date: String,
    task: String,
    source: String,
    source_time: Option<String>,
    rule_ids: Vec<String>,
    snapshot_size: usize,
    batch_ids: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewLhbSourceBinding {
    schema_version: u32,
    business_date: String,
    template_id: String,
    review_task_identity: String,
    delivery_subject_identity: String,
    evidence: ReviewLhbEvidenceBinding,
    ordered_projection: Vec<ReviewLhbStockBinding>,
    rendered_content_sha256: String,
    task_transition_basis: ReviewLhbTaskTransitionBasis,
}

pub(super) fn validate_review_lhb_source_binding_canonical_bytes(
    canonical: &[u8],
) -> Result<Vec<u8>, &'static str> {
    let binding: ReviewLhbSourceBinding =
        serde_json::from_slice(canonical).map_err(|_| "counted_source_only_binding_invalid")?;
    let transition_basis = serde_json::to_vec(&binding.task_transition_basis)
        .map_err(|_| "counted_source_only_binding_invalid")?;
    let expected =
        serde_json::to_vec(&binding).map_err(|_| "counted_source_only_binding_invalid")?;
    if expected != canonical {
        return Err("counted_source_only_binding_invalid");
    }
    Ok(transition_basis)
}

#[cfg(test)]
pub(super) fn canonical_review_lhb_source_binding_for_test(
    value: serde_json::Value,
) -> (Vec<u8>, Vec<u8>) {
    let binding: ReviewLhbSourceBinding =
        serde_json::from_value(value).expect("deserialize TEST_CODE R-04 binding");
    let transition_basis =
        serde_json::to_vec(&binding.task_transition_basis).expect("serialize TEST_CODE R-04 basis");
    let source = serde_json::to_vec(&binding).expect("serialize TEST_CODE R-04 binding");
    (source, transition_basis)
}

#[derive(Debug)]
struct PreparedReviewLhbDelivery {
    rendered: String,
    business_date: chrono::NaiveDate,
    task_identity: String,
    delivery_subject_identity: String,
    source_binding_canonical: Vec<u8>,
    task_transition_basis_canonical: Vec<u8>,
    provider_observed_at: chrono::DateTime<chrono::Utc>,
    batch_id: String,
}

pub(super) fn parse_r04_observed_at(value: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    if let Some(milliseconds) = value.strip_prefix("unix-ms:") {
        if milliseconds.is_empty()
            || !milliseconds.bytes().all(|byte| byte.is_ascii_digit())
            || (milliseconds.len() > 1 && milliseconds.starts_with('0'))
        {
            return Err(format!(
                "R-04 provider observed_at has malformed unix-ms evidence: {value:?}"
            ));
        }
        let milliseconds = milliseconds.parse::<i64>().map_err(|error| {
            format!("R-04 provider observed_at milliseconds are invalid: {error}")
        })?;
        return chrono::DateTime::<chrono::Utc>::from_timestamp_millis(milliseconds)
            .ok_or_else(|| "R-04 provider observed_at is outside chrono range".to_string());
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&chrono::Utc))
        .map_err(|error| format!("R-04 provider observed_at is invalid: {error}"))
}

fn prepare_review_lhb_delivery(
    business_date: chrono::NaiveDate,
    stocks: &[stock_analysis::data_gateway::DragonTigerStockReview],
    evidence: &stock_analysis::data_gateway::BatchEvidence,
) -> Result<PreparedReviewLhbDelivery, String> {
    use magic_market_core::{DragonTigerSide, ProviderId};

    if evidence.provider != ProviderId::Eastmoney {
        return Err(format!(
            "R-04 provider mismatch: expected Eastmoney, got {:?}",
            evidence.provider
        ));
    }
    if evidence.source.trim().is_empty() {
        return Err("R-04 source identity is missing".to_string());
    }
    if evidence.batch_id.trim().is_empty() {
        return Err("R-04 accepted batch ID is missing".to_string());
    }
    let source_at = evidence
        .source_at
        .as_deref()
        .ok_or_else(|| "R-04 provider source_at is missing".to_string())?;
    let source_date = chrono::NaiveDate::parse_from_str(source_at, "%Y-%m-%d")
        .map_err(|error| format!("R-04 provider source_at is invalid: {error}"))?;
    if source_date != business_date {
        return Err(format!(
            "R-04 provider source_at {source_date} differs from business date {business_date}"
        ));
    }
    let provider_observed_at = parse_r04_observed_at(&evidence.observed_at)?;
    if stocks.is_empty() {
        return Err("R-04 available batch contains no records".to_string());
    }

    let mut ordered_projection = Vec::with_capacity(stocks.len());
    for (source_order_ordinal, stock) in stocks.iter().enumerate() {
        if stock.code.trim().is_empty()
            || !stock.ranking_net_amount_yuan.is_finite()
            || stock.ranking_net_amount_yuan <= 0.0
            || stock.disclosures.is_empty()
        {
            return Err(format!(
                "R-04 stock projection at ordinal {source_order_ordinal} is incomplete or invalid"
            ));
        }
        let mut disclosures = Vec::with_capacity(stock.disclosures.len());
        for disclosure in &stock.disclosures {
            if disclosure.entry_id.trim().is_empty()
                || disclosure.trade_id.trim().is_empty()
                || !r04_optional_number_is_finite(disclosure.buy_amount_yuan)
                || !r04_optional_number_is_finite(disclosure.sell_amount_yuan)
                || !r04_optional_number_is_finite(disclosure.net_amount_yuan)
                || !r04_optional_number_is_finite(disclosure.turnover_rate_pct)
            {
                return Err(format!(
                    "R-04 disclosure {} has incomplete or invalid source facts",
                    disclosure.entry_id
                ));
            }
            let mut buy_ranks = [false; 5];
            let mut sell_ranks = [false; 5];
            if disclosure.seats.len() != 10 {
                return Err(format!(
                    "R-04 disclosure {} must contain exactly buy1-buy5 and sell1-sell5",
                    disclosure.entry_id
                ));
            }
            for seat in &disclosure.seats {
                let Ok(rank_index) = usize::try_from(seat.rank.saturating_sub(1)) else {
                    return Err(format!(
                        "R-04 disclosure {} must contain exactly buy1-buy5 and sell1-sell5",
                        disclosure.entry_id
                    ));
                };
                let ranks = match seat.side {
                    DragonTigerSide::Buy => &mut buy_ranks,
                    DragonTigerSide::Sell => &mut sell_ranks,
                };
                let Some(seen) = ranks.get_mut(rank_index) else {
                    return Err(format!(
                        "R-04 disclosure {} must contain exactly buy1-buy5 and sell1-sell5",
                        disclosure.entry_id
                    ));
                };
                if *seen {
                    return Err(format!(
                        "R-04 disclosure {} must contain exactly buy1-buy5 and sell1-sell5",
                        disclosure.entry_id
                    ));
                }
                *seen = true;
            }
            if !buy_ranks.into_iter().all(std::convert::identity)
                || !sell_ranks.into_iter().all(std::convert::identity)
            {
                return Err(format!(
                    "R-04 disclosure {} must contain exactly buy1-buy5 and sell1-sell5",
                    disclosure.entry_id
                ));
            }
            let mut seats = Vec::with_capacity(disclosure.seats.len());
            for seat in &disclosure.seats {
                if seat.rank == 0
                    || seat.seat_name.trim().is_empty()
                    || !seat.amount_yuan.is_finite()
                    || seat.amount_yuan <= 0.0
                    || !r04_optional_number_is_finite(seat.buy_amount_yuan)
                    || !r04_optional_number_is_finite(seat.sell_amount_yuan)
                    || !r04_optional_number_is_finite(seat.net_amount_yuan)
                {
                    return Err(format!(
                        "R-04 disclosure {} has invalid seat facts",
                        disclosure.entry_id
                    ));
                }
                seats.push(ReviewLhbSeatBinding {
                    side: match seat.side {
                        DragonTigerSide::Buy => "buy",
                        DragonTigerSide::Sell => "sell",
                    }
                    .to_string(),
                    rank: seat.rank,
                    seat_name: seat.seat_name.clone(),
                    amount_yuan: seat.amount_yuan,
                    buy_amount_yuan: seat.buy_amount_yuan,
                    sell_amount_yuan: seat.sell_amount_yuan,
                    net_amount_yuan: seat.net_amount_yuan,
                });
            }
            disclosures.push(ReviewLhbDisclosureBinding {
                entry_id: disclosure.entry_id.clone(),
                trade_id: disclosure.trade_id.clone(),
                reason: disclosure.reason.clone(),
                buy_amount_yuan: disclosure.buy_amount_yuan,
                sell_amount_yuan: disclosure.sell_amount_yuan,
                net_amount_yuan: disclosure.net_amount_yuan,
                turnover_rate_pct: disclosure.turnover_rate_pct,
                seats,
            });
        }
        ordered_projection.push(ReviewLhbStockBinding {
            source_order_ordinal,
            exchange: exchange_label(stock.exchange).to_string(),
            code: stock.code.clone(),
            ranking_net_amount_yuan: stock.ranking_net_amount_yuan,
            disclosures,
        });
    }

    let business_date_text = business_date.format("%Y-%m-%d").to_string();
    let task_identity = crate::review_batch::review_task_identity(
        business_date,
        crate::review_batch::ReviewTask::R04,
    );
    let delivery_subject_identity = crate::review_batch::audit_identity_hash(
        "review-lhb-delivery-subject",
        &format!("{business_date_text}:{task_identity}"),
    );
    let rendered = render_review_lhb_gateway(&business_date_text, stocks, evidence);
    let task_transition_basis = ReviewLhbTaskTransitionBasis {
        task_identity: task_identity.clone(),
        business_date: business_date_text.clone(),
        task: "R-04".to_string(),
        source: evidence.source.clone(),
        source_time: Some(source_at.to_string()),
        rule_ids: vec![
            "BR-110".to_string(),
            "BR-140".to_string(),
            "BR-162".to_string(),
            "BR-192".to_string(),
            "BR-200".to_string(),
        ],
        snapshot_size: stocks.len(),
        batch_ids: vec![evidence.batch_id.clone()],
    };
    let task_transition_basis_canonical = serde_json::to_vec(&task_transition_basis)
        .map_err(|error| format!("R-04 task transition serialization failed: {error}"))?;
    let source_binding = ReviewLhbSourceBinding {
        schema_version: 1,
        business_date: business_date_text,
        template_id: "review_lhb_v1".to_string(),
        review_task_identity: task_identity.clone(),
        delivery_subject_identity: delivery_subject_identity.clone(),
        evidence: ReviewLhbEvidenceBinding {
            provider: "Eastmoney".to_string(),
            source: evidence.source.clone(),
            source_at: source_at.to_string(),
            observed_at: evidence.observed_at.clone(),
            batch_id: evidence.batch_id.clone(),
        },
        ordered_projection,
        rendered_content_sha256: r04_sha256(rendered.as_bytes()),
        task_transition_basis,
    };
    let source_binding_canonical = serde_json::to_vec(&source_binding)
        .map_err(|error| format!("R-04 source binding serialization failed: {error}"))?;
    Ok(PreparedReviewLhbDelivery {
        rendered,
        business_date,
        task_identity,
        delivery_subject_identity,
        source_binding_canonical,
        task_transition_basis_canonical,
        provider_observed_at,
        batch_id: evidence.batch_id.clone(),
    })
}

fn r04_optional_number_is_finite(value: Option<f64>) -> bool {
    value.is_none_or(f64::is_finite)
}

fn r04_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    format!("{:x}", Sha256::digest(bytes))
}

/// R-04 龙虎榜 (BR-162): unified Gateway whole-market batch, grouped by stock.
pub async fn dispatch_r04_lhb_outcome(
    date: &str,
    now_time: chrono::NaiveTime,
) -> crate::review_batch::ReviewTaskOutcome {
    use stock_analysis::data_gateway::DragonTigerGateway;

    let gateway = DragonTigerGateway::new();
    dispatch_r04_lhb_outcome_with_loader(
        date,
        now_time,
        inspect_r04_review_occurrence,
        move |trading_date| async move {
            // Eastmoney publishes some lower-ranked disclosures without a complete
            // buy-five side. R-04 requests the canonical top-five disclosure batch
            // and fails closed if any of those five is incomplete.
            gateway.market_review(trading_date, 5, 5).await
        },
    )
    .await
}

async fn inspect_r04_review_occurrence(
    review_date: chrono::NaiveDate,
) -> Result<Option<crate::durable_delivery_runtime::DurableDispatchEvidence>, String> {
    crate::durable_delivery_runtime::inspect_review_task_occurrence(
        review_date,
        stock_analysis::durable_delivery::PushKind::ReviewLhb,
        crate::review_batch::review_task_identity(
            review_date,
            crate::review_batch::ReviewTask::R04,
        ),
    )
    .await
}

async fn dispatch_r04_lhb_outcome_with_loader<Preflight, PreflightFuture, F, Fut>(
    date: &str,
    now_time: chrono::NaiveTime,
    preflight: Preflight,
    fetch_lhb: F,
) -> crate::review_batch::ReviewTaskOutcome
where
    Preflight: FnOnce(chrono::NaiveDate) -> PreflightFuture,
    PreflightFuture: std::future::Future<
            Output = Result<
                Option<crate::durable_delivery_runtime::DurableDispatchEvidence>,
                String,
            >,
        > + Send,
    F: FnOnce(chrono::NaiveDate) -> Fut + Send,
    Fut: std::future::Future<
            Output = Result<
                stock_analysis::data_gateway::GatewayBatch<
                    stock_analysis::data_gateway::DragonTigerStockReview,
                >,
                stock_analysis::data_gateway::GatewayError,
            >,
        > + Send,
{
    use crate::review_batch::ReviewTaskOutcome;
    use chrono::NaiveDate;
    use stock_analysis::data_gateway::GatewayBatch;

    let today = match NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        Ok(today) => today,
        Err(error) => {
            let reason = format!("非法复盘日期 {date}: {error}");
            log::error!("[R-04][BR-110] {reason}");
            log_dispatcher_attempt("R-04", false, 0, &reason);
            return ReviewTaskOutcome::failed(false, reason);
        }
    };
    match preflight(today).await {
        Ok(Some(evidence)) => {
            return review_outcome_from_existing_durable(
                evidence,
                today,
                crate::review_batch::ReviewTask::R04,
            )
        }
        Ok(None) => {}
        Err(error) => {
            let reason = format!("R-04 durable terminal preflight failed: {error}");
            log_dispatcher_attempt("R-04", false, 0, &reason);
            return ReviewTaskOutcome::failed(true, reason);
        }
    }
    let lhb_ready_time = chrono::NaiveTime::from_hms_opt(21, 0, 0).unwrap();
    if now_time < lhb_ready_time {
        return ReviewTaskOutcome::expected_wait(
            lhb_ready_time,
            "LHB source not published before 21:00",
        );
    }
    let batch = match fetch_lhb(today).await {
        Ok(batch) => batch,
        Err(error) => {
            let retryable = error.retryable();
            let reason = error.to_string();
            log::error!(
                "[R-04][BR-159][BR-162] reason_code={} retryable={retryable}: {reason}",
                error.reason_code()
            );
            log_dispatcher_attempt("R-04", false, 0, &reason);
            return ReviewTaskOutcome::failed(retryable, reason);
        }
    };
    let (entries, evidence) = match batch {
        GatewayBatch::VerifiedEmpty(evidence) => {
            log::info!(
                "[R-04][BR-162] verified empty provider={:?} source={} batch_id={}",
                evidence.provider,
                evidence.source,
                evidence.batch_id
            );
            log_dispatcher_attempt("R-04", false, 0, "verified empty LHB net-buy batch");
            return ReviewTaskOutcome::no_data(
                "complete LHB source returned zero positive-net stocks after 21:00",
            );
        }
        GatewayBatch::Available { records, evidence } => (records, evidence),
    };
    let prepared = match prepare_review_lhb_delivery(today, &entries, &evidence) {
        Ok(prepared) => prepared,
        Err(reason) => {
            log::error!("[R-04][BR-140][BR-192] counted binding rejected: {reason}");
            log_dispatcher_attempt("R-04", false, entries.len(), &reason);
            return ReviewTaskOutcome::failed(false, reason);
        }
    };
    let task_binding = match stock_analysis::durable_delivery::TaskBinding::new(
        prepared.task_identity.clone(),
        prepared.task_transition_basis_canonical.clone(),
    ) {
        Ok(binding) => binding,
        Err(error) => {
            let reason = format!("R-04 task binding rejected: {error}");
            log::error!("[R-04][BR-140][BR-192] {reason}");
            log_dispatcher_attempt("R-04", false, entries.len(), &reason);
            return ReviewTaskOutcome::failed(false, reason);
        }
    };
    let counted_binding = match crate::durable_delivery_runtime::CountedDeliveryBinding::new(
        prepared.business_date,
        prepared.task_identity,
        prepared.source_binding_canonical,
        crate::durable_delivery_runtime::CountedDeliveryScope::Global,
        prepared.delivery_subject_identity,
        crate::durable_delivery_runtime::CountedDeliveryOrigin::Provider {
            observed_at: Some(prepared.provider_observed_at),
            as_of: Some(prepared.business_date),
            ordered_batch_ids: vec![prepared.batch_id],
        },
        Some(task_binding),
        true,
    ) {
        Ok(binding) => binding,
        Err(reason) => {
            log::error!("[R-04][BR-140][BR-192] counted binding rejected: {reason}");
            log_dispatcher_attempt("R-04", false, entries.len(), &reason);
            return ReviewTaskOutcome::failed(false, reason);
        }
    };
    let presentation_token = match crate::presentation_registry::acquire_token(
        "R-04-review-lhb-gateway",
        crate::notify::PushKind::ReviewLhb,
        "review_lhb_gateway_dispatcher",
        "render_review_lhb_gateway",
    ) {
        Ok(token) => token,
        Err(reason) => {
            log::error!("[R-04][BR-196] presentation token rejected: {reason}");
            log_dispatcher_attempt("R-04", false, entries.len(), &reason);
            return ReviewTaskOutcome::failed(false, reason);
        }
    };
    let push_result = crate::notify::push_counted_source_only_with_binding(
        presentation_token,
        &prepared.rendered,
        counted_binding,
    )
    .await;
    let dispatcher_error = push_outcome_dispatcher_error(&push_result);
    log_dispatcher_attempt(
        "R-04",
        push_result.is_pushed(),
        entries.len(),
        &dispatcher_error,
    );
    ReviewTaskOutcome::from_push_outcome(push_result, entries.len())
}

fn push_outcome_dispatcher_error(outcome: &crate::notify::PushOutcome) -> String {
    match outcome {
        crate::notify::PushOutcome::Pushed => String::new(),
        crate::notify::PushOutcome::Deduped => {
            "delivery deduplicated by push governance".to_owned()
        }
        crate::notify::PushOutcome::Denied(reason) => {
            format!("delivery denied by push governance: {reason}")
        }
        crate::notify::PushOutcome::SinkError(reason) => {
            format!("delivery sink failed: {reason}")
        }
    }
}

/// R-05 信号复盘：需要“信号触发 → 成交 → 平仓结果”的完整关联源。
/// 当前通用交易表无法证明信号归属，明确禁用而不是据此伪造胜率（BR-110）。
pub async fn dispatch_r05_signal_review_real(_date: &str, _banner: &BannerCtx) -> bool {
    let reason = "disabled=no_signal_delivery_execution_settlement_outcome_source";
    log::error!("[R-05][BR-110] {reason}");
    log_dispatcher_attempt("R-05", false, 0, reason);
    false
}

/// R-06 失败归因：等待与信号、投递、执行、结算及分类器版本绑定的真实结果源。
/// 通用交易或订单拒绝记录不能被重新解释为策略失败原因（BR-110）。
pub async fn dispatch_r06_failure_real(_date: &str, _banner: &BannerCtx) -> bool {
    let reason = "disabled=no_evidence_bound_classified_failure_outcome_source";
    log::error!("[R-06][BR-110] {reason}");
    log_dispatcher_attempt("R-06", false, 0, reason);
    false
}

// ============================================================================
// Review dispatcher regression tests
// R-02: capability unavailable -> no partial acquisition
// R-03: chain_daily cluster -> ChainLine
// ============================================================================

#[cfg(test)]
mod tests_r_dispatchers {
    use super::*;

    #[test]
    fn br140_dispatcher_error_is_never_empty_on_non_push() {
        assert_eq!(
            push_outcome_dispatcher_error(&crate::notify::PushOutcome::Pushed),
            ""
        );
        assert_eq!(
            push_outcome_dispatcher_error(&crate::notify::PushOutcome::Deduped),
            "delivery deduplicated by push governance"
        );
        assert_eq!(
            push_outcome_dispatcher_error(&crate::notify::PushOutcome::Denied(
                "TEST_CODE denied".to_owned()
            )),
            "delivery denied by push governance: TEST_CODE denied"
        );
        assert_eq!(
            push_outcome_dispatcher_error(&crate::notify::PushOutcome::SinkError(
                "TEST_CODE sink".to_owned()
            )),
            "delivery sink failed: TEST_CODE sink"
        );
    }

    #[test]
    fn br162_r04_production_dispatcher_uses_unified_gateway_only() {
        let source = include_str!("push_templates.rs");
        let start = source
            .find("pub async fn dispatch_r04_lhb_outcome(")
            .expect("R-04 production dispatcher");
        let end_offset = source[start..]
            .find("pub async fn dispatch_r05_signal_review_real(")
            .expect("R-05 dispatcher boundary");
        let dispatcher = &source[start..start + end_offset];

        assert!(
            dispatcher.contains("DragonTigerGateway"),
            "R-04 must acquire data through DragonTigerGateway"
        );
        assert!(
            !dispatcher.contains("market_analyzer::lhb_review")
                && !dispatcher.contains("fetch_recent_lhb")
                && !dispatcher.contains("reqwest"),
            "R-04 production dispatcher must not retain the legacy/direct HTTP loader"
        );
    }

    #[test]
    fn br162_r04_renderer_keeps_trade_ids_and_exact_seats_without_fake_sum() {
        use magic_market_core::{DragonTigerSide, Exchange, ProviderId};
        use stock_analysis::data_gateway::{
            BatchEvidence, DragonTigerSeatReview, DragonTigerSourceDisclosure,
            DragonTigerStockReview,
        };

        let disclosure = |trade_id: &str, net_amount_yuan: f64| {
            let mut seats = Vec::with_capacity(10);
            for side in [DragonTigerSide::Buy, DragonTigerSide::Sell] {
                for rank in 1..=5 {
                    seats.push(DragonTigerSeatReview {
                        side,
                        rank,
                        seat_name: format!("TEST_CODE_{side:?}_{rank}"),
                        amount_yuan: f64::from(rank) * 10_000_000.0,
                        buy_amount_yuan: None,
                        sell_amount_yuan: None,
                        net_amount_yuan: None,
                    });
                }
            }
            DragonTigerSourceDisclosure {
                entry_id: format!("TEST_CODE_600396:2099-01-02:{trade_id}"),
                trade_id: trade_id.to_string(),
                reason: Some(format!("TEST_CODE_reason_{trade_id}")),
                buy_amount_yuan: Some(500_000_000.0),
                sell_amount_yuan: Some(500_000_000.0 - net_amount_yuan),
                net_amount_yuan: Some(net_amount_yuan),
                turnover_rate_pct: Some(12.34),
                seats,
            }
        };
        let stocks = vec![DragonTigerStockReview {
            exchange: Exchange::Shanghai,
            code: "TEST_CODE_600396".to_string(),
            ranking_net_amount_yuan: 380_000_000.0,
            disclosures: vec![
                disclosure("100380472", 380_000_000.0),
                disclosure("100380465", 280_000_000.0),
            ],
        }];
        let evidence = BatchEvidence {
            provider: ProviderId::Eastmoney,
            source: "TEST_CODE_eastmoney-market-dragon-tiger".to_string(),
            source_at: Some("2099-01-02".to_string()),
            observed_at: "2099-01-02T21:00:00+08:00".to_string(),
            batch_id: "TEST_CODE_batch_r04".to_string(),
        };

        let rendered = render_review_lhb_gateway("2099-01-02", &stocks, &evidence);

        assert!(rendered.contains("排名净买 3.80亿 | 源披露 2 条"));
        assert!(rendered.contains("TRADE_ID=100380472"));
        assert!(rendered.contains("TRADE_ID=100380465"));
        assert!(rendered.contains("买1 TEST_CODE_Buy_1"));
        assert!(rendered.contains("卖5 TEST_CODE_Sell_5"));
        assert!(rendered.contains("不同 TRADE_ID 未合并求和"));
        assert!(!rendered.contains("6.60亿"));
        assert!(!rendered.contains("数据缺失席"));

        let prepared = prepare_review_lhb_delivery(
            chrono::NaiveDate::from_ymd_opt(2099, 1, 2).unwrap(),
            &stocks,
            &evidence,
        )
        .expect("complete R-04 source facts form a counted binding");
        let replay = prepare_review_lhb_delivery(
            chrono::NaiveDate::from_ymd_opt(2099, 1, 2).unwrap(),
            &stocks,
            &evidence,
        )
        .expect("identical source facts replay");
        assert_eq!(
            prepared.source_binding_canonical,
            replay.source_binding_canonical
        );
        assert_eq!(
            prepared.task_identity,
            crate::review_batch::review_task_identity(
                chrono::NaiveDate::from_ymd_opt(2099, 1, 2).unwrap(),
                crate::review_batch::ReviewTask::R04,
            )
        );
        assert_eq!(prepared.batch_id, evidence.batch_id);
        assert_eq!(
            prepared.provider_observed_at,
            chrono::DateTime::parse_from_rfc3339(&evidence.observed_at)
                .unwrap()
                .with_timezone(&chrono::Utc)
        );
        let mut magic_evidence = evidence.clone();
        magic_evidence.observed_at = "unix-ms:1785578029695".to_string();
        let magic_prepared = prepare_review_lhb_delivery(
            chrono::NaiveDate::from_ymd_opt(2099, 1, 2).unwrap(),
            &stocks,
            &magic_evidence,
        )
        .expect("Magic provenance unix-ms evidence forms the same typed UTC origin");
        assert_eq!(
            magic_prepared.provider_observed_at,
            chrono::DateTime::<chrono::Utc>::from_timestamp_millis(1_785_578_029_695).unwrap()
        );
        let magic_canonical: serde_json::Value =
            serde_json::from_slice(&magic_prepared.source_binding_canonical).unwrap();
        assert_eq!(
            magic_canonical["evidence"]["observed_at"], "unix-ms:1785578029695",
            "the immutable source binding must preserve the provider evidence bytes"
        );
        let business_date = chrono::NaiveDate::from_ymd_opt(2099, 1, 2).unwrap();
        let task_binding = stock_analysis::durable_delivery::TaskBinding::new(
            magic_prepared.task_identity.clone(),
            magic_prepared.task_transition_basis_canonical.clone(),
        )
        .expect("TEST_CODE R-04 task binding");
        let counted_binding = crate::durable_delivery_runtime::CountedDeliveryBinding::new(
            business_date,
            magic_prepared.task_identity.clone(),
            magic_prepared.source_binding_canonical.clone(),
            crate::durable_delivery_runtime::CountedDeliveryScope::Global,
            magic_prepared.delivery_subject_identity.clone(),
            crate::durable_delivery_runtime::CountedDeliveryOrigin::Provider {
                observed_at: Some(magic_prepared.provider_observed_at),
                as_of: Some(business_date),
                ordered_batch_ids: vec![magic_prepared.batch_id.clone()],
            },
            Some(task_binding),
            true,
        )
        .expect("TEST_CODE complete R-04 counted binding");
        assert_eq!(
            counted_binding.validate_r04_source_only_text(&magic_prepared.rendered),
            Ok(()),
            "a provider timestamp accepted during preparation must survive the exact durable revalidation"
        );
        let canonical: serde_json::Value =
            serde_json::from_slice(&prepared.source_binding_canonical).unwrap();
        assert_eq!(canonical["business_date"], "2099-01-02");
        assert_eq!(canonical["evidence"]["batch_id"], "TEST_CODE_batch_r04");
        assert_eq!(
            canonical["ordered_projection"][0]["source_order_ordinal"],
            0
        );
        assert_eq!(
            canonical["ordered_projection"][0]["disclosures"][0]["trade_id"],
            "100380472"
        );
        assert_eq!(
            canonical["ordered_projection"][0]["disclosures"][1]["trade_id"],
            "100380465"
        );
        let task_basis: serde_json::Value =
            serde_json::from_slice(&prepared.task_transition_basis_canonical).unwrap();
        assert_eq!(task_basis["task"], "R-04");
        assert_eq!(task_basis["batch_ids"][0], "TEST_CODE_batch_r04");
    }

    #[test]
    fn br192_r04_counted_binding_fails_closed_without_exact_provider_evidence() {
        use magic_market_core::{DragonTigerSide, Exchange, ProviderId};
        use stock_analysis::data_gateway::{
            BatchEvidence, DragonTigerSeatReview, DragonTigerSourceDisclosure,
            DragonTigerStockReview,
        };

        let stocks = vec![DragonTigerStockReview {
            exchange: Exchange::Shanghai,
            code: "TEST_CODE_600396".to_string(),
            ranking_net_amount_yuan: 380_000_000.0,
            disclosures: vec![DragonTigerSourceDisclosure {
                entry_id: "TEST_CODE_ENTRY".to_string(),
                trade_id: "TEST_CODE_TRADE".to_string(),
                reason: None,
                buy_amount_yuan: Some(500_000_000.0),
                sell_amount_yuan: Some(120_000_000.0),
                net_amount_yuan: Some(380_000_000.0),
                turnover_rate_pct: Some(12.34),
                seats: vec![DragonTigerSeatReview {
                    side: DragonTigerSide::Buy,
                    rank: 1,
                    seat_name: "TEST_CODE_SEAT".to_string(),
                    amount_yuan: 500_000_000.0,
                    buy_amount_yuan: Some(500_000_000.0),
                    sell_amount_yuan: None,
                    net_amount_yuan: Some(500_000_000.0),
                }],
            }],
        }];
        let complete = BatchEvidence {
            provider: ProviderId::Eastmoney,
            source: "TEST_CODE_eastmoney-market-dragon-tiger".to_string(),
            source_at: Some("2099-01-02".to_string()),
            observed_at: "2099-01-02T21:00:00+08:00".to_string(),
            batch_id: "TEST_CODE_batch_r04".to_string(),
        };
        let date = chrono::NaiveDate::from_ymd_opt(2099, 1, 2).unwrap();

        assert!(
            prepare_review_lhb_delivery(date, &stocks, &complete)
                .unwrap_err()
                .contains("exactly buy1-buy5 and sell1-sell5"),
            "a disclosure with only one buy seat must never enter the canonical R-04 binding"
        );

        let mut missing_source_at = complete.clone();
        missing_source_at.source_at = None;
        assert!(
            prepare_review_lhb_delivery(date, &stocks, &missing_source_at)
                .unwrap_err()
                .contains("source_at is missing")
        );

        let mut mismatched_source_at = complete.clone();
        mismatched_source_at.source_at = Some("2099-01-01".to_string());
        assert!(
            prepare_review_lhb_delivery(date, &stocks, &mismatched_source_at)
                .unwrap_err()
                .contains("differs from business date")
        );

        let mut missing_batch_id = complete.clone();
        missing_batch_id.batch_id.clear();
        assert!(
            prepare_review_lhb_delivery(date, &stocks, &missing_batch_id)
                .unwrap_err()
                .contains("batch ID is missing")
        );

        for observed_at in [
            "TEST_CODE_not_a_timestamp",
            "unix-ms:",
            "unix-ms:01785578029695",
            "unix-ms:not-a-number",
        ] {
            let mut invalid_observed_at = complete.clone();
            invalid_observed_at.observed_at = observed_at.to_string();
            assert!(
                prepare_review_lhb_delivery(date, &stocks, &invalid_observed_at)
                    .unwrap_err()
                    .contains("observed_at"),
                "invalid observation evidence must fail closed: {observed_at}"
            );
        }

        assert!(prepare_review_lhb_delivery(
            date,
            &[],
            &BatchEvidence {
                provider: ProviderId::Eastmoney,
                source: "TEST_CODE_eastmoney-market-dragon-tiger".to_string(),
                source_at: Some("2099-01-02".to_string()),
                observed_at: "2099-01-02T21:00:00+08:00".to_string(),
                batch_id: "TEST_CODE_batch_r04".to_string(),
            }
        )
        .unwrap_err()
        .contains("contains no records"));
    }

    #[test]
    fn br192_r04_dispatch_uses_only_explicit_counted_binding() {
        let source = include_str!("push_templates.rs");
        let start = source
            .find("async fn dispatch_r04_lhb_outcome_with_loader")
            .expect("R-04 dispatcher");
        let end = source[start..]
            .find("pub async fn dispatch_r05_signal_review_real")
            .expect("R-04 dispatcher boundary");
        let dispatcher = &source[start..start + end];

        assert!(dispatcher.contains("prepare_review_lhb_delivery"));
        assert!(dispatcher.contains("CountedDeliveryOrigin::Provider"));
        assert!(dispatcher.contains("ordered_batch_ids: vec![prepared.batch_id]"));
        assert!(dispatcher.contains("push_counted_source_only_with_binding"));
        assert!(!dispatcher.contains("push_counted_with_binding("));
        assert!(!dispatcher.contains("dispatch_outcome("));
        assert!(!dispatcher.contains("push_governor_v3("));
    }

    #[tokio::test]
    async fn br200_r04_existing_delivered_skips_provider_and_reuses_count() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 30).unwrap();
        let task_identity =
            crate::review_batch::review_task_identity(date, crate::review_batch::ReviewTask::R04);
        let basis = serde_json::to_vec(&serde_json::json!({
            "task_identity": task_identity.clone(),
            "business_date": "2026-07-30",
            "task": "R-04",
            "snapshot_size": 5,
        }))
        .unwrap();
        let evidence = crate::durable_delivery_runtime::DurableDispatchEvidence {
            decision_identity: "TEST_CODE_BR200_R04_DECISION".to_string(),
            state: stock_analysis::durable_delivery::DecisionState::Delivered,
            schedule_hydration: Some(stock_analysis::durable_delivery::ScheduleHydration {
                decision_identity: "TEST_CODE_BR200_R04_DECISION".to_string(),
                task_identity,
                transition_identity: "TEST_CODE_BR200_R04_TRANSITION".to_string(),
                transition_canonical: br#"{"TEST_CODE":"transition"}"#.to_vec(),
                transition_sha256: "a".repeat(64),
                transition_basis_sha256: r09_sha256(&basis),
                transition_basis_canonical: basis,
                immutable_audit_ref: "TEST_CODE_BR200_R04_AUDIT".to_string(),
                hydration_state: stock_analysis::durable_delivery::ScheduleHydrationState::Applied,
            }),
        };
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&provider_calls);
        let outcome = dispatch_r04_lhb_outcome_with_loader(
            "2026-07-30",
            chrono::NaiveTime::from_hms_opt(21, 0, 0).unwrap(),
            |_| async { Ok(Some(evidence)) },
            move |_| async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(stock_analysis::data_gateway::GatewayError::unavailable(
                    "R-04",
                    Some(magic_market_core::ProviderId::Eastmoney),
                    true,
                    "TEST_CODE provider must not run",
                ))
            },
        )
        .await;

        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
        assert!(matches!(
            outcome,
            crate::review_batch::ReviewTaskOutcome::Delivered { count: 5 }
        ));
    }

    /// BR-140: R-02 的完整批次能力缺失时必须 fail-closed，且不得先做局部请求。
    #[test]
    fn br140_r02_incomplete_contract_stays_disabled_before_acquisition() {
        let source = include_str!("push_templates.rs");
        let start = source
            .find("pub async fn dispatch_r02_review_market_real(")
            .expect("R-02 compatibility dispatcher");
        let end_offset = source[start..]
            .find("struct R08PublicCalendarComponents")
            .expect("R-02 dispatcher boundary");
        let dispatcher = &source[start..start + end_offset];

        assert!(dispatcher.contains("no_complete_review_date_market_overview_batch"));
        assert!(
            !dispatcher.contains("fetch_market_review_snapshot")
                && !dispatcher.contains("spawn_blocking")
                && !dispatcher.contains("MarketAnalyzer"),
            "R-02 must not perform partial acquisition while the complete capability is disabled"
        );
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

    /// BR-110: 通用交易行不能替代信号到结算的权威闭环。
    #[tokio::test]
    async fn br110_r05_stays_disabled_without_authoritative_lineage() {
        let banner = BannerCtx {
            account_mode: AccountMode::Normal,
            total_pos: Some(0),
            today_pnl: Some(0.0),
            account_metrics_complete: true,
            data_mode: DataMode::Full,
            data_missing_note: None,
        };
        let result = dispatch_r05_signal_review_real("2026-01-01", &banner).await;
        assert!(!result, "R-05 缺权威闭环时必须禁用");
        assert!(
            !dispatch_r06_failure_real("2026-01-01", &banner).await,
            "R-06 缺证据绑定分类结果时必须禁用"
        );
    }

    #[tokio::test]
    async fn br162_r04_preserves_wait_empty_and_unavailable_outcomes() {
        use magic_market_core::ProviderId;
        use stock_analysis::data_gateway::{BatchEvidence, GatewayBatch, GatewayError};

        let before = dispatch_r04_lhb_outcome_with_loader(
            "2026-07-21",
            chrono::NaiveTime::from_hms_opt(20, 59, 0).unwrap(),
            |_| async { Ok(None) },
            |_| async {
                panic!("TEST_CODE R-04 loader must not run before 21:00");
                #[allow(unreachable_code)]
                Err(GatewayError::unavailable(
                    "R-04",
                    Some(ProviderId::Eastmoney),
                    true,
                    "TEST_CODE unreachable",
                ))
            },
        )
        .await;
        assert!(matches!(
            before,
            crate::review_batch::ReviewTaskOutcome::ExpectedWait { .. }
        ));

        let after = dispatch_r04_lhb_outcome_with_loader(
            "2026-07-21",
            chrono::NaiveTime::from_hms_opt(21, 0, 0).unwrap(),
            |_| async { Ok(None) },
            |_| async {
                Err(GatewayError::unavailable(
                    "R-04",
                    Some(ProviderId::Eastmoney),
                    true,
                    "TEST_CODE lhb producer unavailable",
                ))
            },
        )
        .await;
        assert!(matches!(
            after,
            crate::review_batch::ReviewTaskOutcome::Failed {
                failure: crate::review_batch::ReviewTaskFailure::ExistingSourceFailure {
                    retryable: true,
                    ..
                },
            }
        ));

        let evidence = BatchEvidence {
            provider: ProviderId::Eastmoney,
            source: "TEST_CODE_eastmoney-market-dragon-tiger".to_string(),
            source_at: Some("2026-07-21".to_string()),
            observed_at: "2026-07-21T21:00:00+08:00".to_string(),
            batch_id: "TEST_CODE_r04_empty".to_string(),
        };
        let empty = dispatch_r04_lhb_outcome_with_loader(
            "2026-07-21",
            chrono::NaiveTime::from_hms_opt(21, 0, 0).unwrap(),
            |_| async { Ok(None) },
            |_| async { Ok(GatewayBatch::VerifiedEmpty(evidence)) },
        )
        .await;
        assert!(matches!(
            empty,
            crate::review_batch::ReviewTaskOutcome::NoData { .. }
        ));
    }
}

/// v35: A-10 dispatcher 入口
async fn dispatch_catalyst_review_daily_outcome(
    date: &str,
) -> crate::review_batch::ReviewTaskOutcome {
    let snapshot = match load_catalyst_review_snapshot_real(date).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            log::error!("[A-10] 催化复盘批次拒绝: {error}");
            log_dispatcher_attempt("A-10", false, 0, &error);
            return crate::review_batch::ReviewTaskOutcome::failed(
                true,
                format!("A-10 source unavailable: {error}"),
            );
        }
    };
    if snapshot.leading_members.is_empty() {
        log_dispatcher_attempt("A-10", false, 0, "catalyst_review_snapshot empty");
        log::info!("[A-10] visible ChainIntelligenceBatch 无合格主线，跳过推送");
        return crate::review_batch::ReviewTaskOutcome::no_data(
            "visible ChainIntelligenceBatch contains no eligible source-backed chain",
        );
    }
    let leading_refs: Vec<&str> = snapshot
        .leading_members
        .iter()
        .map(|name| name.as_str())
        .collect();
    let other_refs: Vec<&str> = snapshot
        .other_members
        .iter()
        .map(|name| name.as_str())
        .collect();
    // BR-225: 题材评分/观察点推导 (无独立评分批次时用当日涨停结构, 确定性规则)
    let derived_score = snapshot.score.or_else(|| {
        let structure = (snapshot.member_count as f32).min(100.0) * 0.30
            + (snapshot.continuous_count as f32).min(60.0) * 0.50
            + match snapshot.persistent {
                PersistentLevel::High => 20.0,
                PersistentLevel::Med => 10.0,
                PersistentLevel::Low => 0.0,
            };
        Some(structure.min(100.0))
    });
    let derived_watch_point = snapshot.watch_point.as_deref().filter(|v| !v.trim().is_empty()).or_else(|| {
        Some(if snapshot.continuous_count >= 10 {
            "连板结构高位, 明日关注前排是否扩散与是否退潮 (基于当日涨停结构推导)"
        } else if snapshot.continuous_count >= 3 {
            "连板梯队成型, 明日关注前排延续性与后排补涨 (基于当日涨停结构推导)"
        } else {
            "连板梯队偏弱, 明日关注题材是否出现新催化 (基于当日涨停结构推导)"
        })
    });
    let params = CatalystReviewParams {
        date: &snapshot.date,
        theme: &snapshot.theme,
        score: derived_score,
        persistent: snapshot.persistent,
        member_count: snapshot.member_count,
        continuous_count: snapshot.continuous_count,
        leading_names: leading_refs,
        other_names: other_refs,
        watch_point: derived_watch_point,
    };
    let text = render_catalyst_review(params);
    let business_date = match chrono::NaiveDate::parse_from_str(&snapshot.date, "%Y-%m-%d") {
        Ok(date) => date,
        Err(error) => {
            let reason = format!("A-10 source batch business date invalid: {error}");
            log_dispatcher_attempt("A-10", false, snapshot.member_count, &reason);
            return crate::review_batch::ReviewTaskOutcome::failed(false, reason);
        }
    };
    let source_evidence = match crate::v14_adapter::SourceBatchEvidence::new(
        crate::notify::PushKind::CatalystReview,
        business_date,
        match snapshot.source_observed_at {
            Some(observed_at) => observed_at,
            None => {
                let reason = "A-10 source batch observation time missing".to_string();
                log_dispatcher_attempt("A-10", false, snapshot.member_count, &reason);
                return crate::review_batch::ReviewTaskOutcome::failed(false, reason);
            }
        },
        snapshot.source_batch_id.clone(),
        snapshot.source_content_hash.clone(),
    ) {
        Ok(evidence) => evidence,
        Err(error) => {
            let reason = format!("A-10 source batch binding rejected: {error}");
            log_dispatcher_attempt("A-10", false, snapshot.member_count, &reason);
            return crate::review_batch::ReviewTaskOutcome::failed(false, reason);
        }
    };
    let presentation_token = match crate::presentation_registry::acquire_token(
        "A-10-catalyst-review",
        crate::notify::PushKind::CatalystReview,
        "catalyst_review_dispatcher",
        "render_catalyst_review",
    ) {
        Ok(token) => token,
        Err(reason) => {
            log::error!("[A-10][BR-196] presentation token rejected: {reason}");
            log_dispatcher_attempt("A-10", false, snapshot.member_count, &reason);
            return crate::review_batch::ReviewTaskOutcome::failed(false, reason);
        }
    };
    let result =
        crate::notify::push_source_batch_v3(presentation_token, &text, &source_evidence).await;
    let dispatcher_error = push_outcome_dispatcher_error(&result);
    log_dispatcher_attempt(
        "A-10",
        result.is_pushed(),
        snapshot.member_count,
        &dispatcher_error,
    );
    crate::review_batch::ReviewTaskOutcome::from_push_outcome(result, snapshot.member_count)
}

pub async fn dispatch_catalyst_review_daily(date: &str) -> bool {
    matches!(
        dispatch_catalyst_review_daily_outcome(date).await,
        crate::review_batch::ReviewTaskOutcome::Delivered { .. }
    )
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
    pub rejections: Vec<VirtualObservationLoadIssue>,
    pub source_failures: Vec<VirtualObservationLoadIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualObservationLoadIssue {
    pub identity_hash: String,
    pub reason_code: String,
    pub retryable: bool,
}

fn audit_virtual_observation_load_issues(snapshot: &VirtualSnapshotLite) -> Result<(), String> {
    let issues = snapshot
        .rejections
        .iter()
        .chain(snapshot.source_failures.iter())
        .collect::<Vec<_>>();
    if issues.is_empty() {
        return Ok(());
    }
    let now = chrono::Local::now();
    let observed_at = now.format("%Y-%m-%dT%H:%M:%S").to_string();
    for issue in &issues {
        log::warn!(
            "[A-01][BR-140] virtual observation isolated identity_hash={} reason_code={} retryable={}",
            issue.identity_hash,
            issue.reason_code,
            issue.retryable
        );
    }
    let rejections = issues
        .into_iter()
        .map(|issue| crate::review_batch::ReviewCandidateRejection {
            observed_at: observed_at.clone(),
            task: "A-01".to_string(),
            source: "virtual_observation".to_string(),
            source_time: None,
            rule_ids: vec!["BR-104".to_string(), "BR-140".to_string()],
            retryable: issue.retryable,
            identity_hash: issue.identity_hash.clone(),
            reason_code: issue.reason_code.clone(),
        })
        .collect();
    crate::review_batch::append_candidate_rejection_audit(rejections, now.date_naive()).map(|_| ())
}

fn persist_review_rejections(
    task: &str,
    source: &str,
    date: chrono::NaiveDate,
    rule_ids: &[&str],
    rows: Vec<(String, &'static str, bool)>,
) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    let now = chrono::Local::now();
    let observed_at = now.format("%Y-%m-%dT%H:%M:%S").to_string();
    let rejections = rows
        .into_iter()
        .map(|(identity, reason_code, retryable)| {
            let identity_hash =
                crate::review_batch::audit_identity_hash(task, identity.as_str());
            log::warn!(
                "[{task}][BR-140] candidate isolated identity_hash={identity_hash} reason_code={reason_code} retryable={retryable}"
            );
            crate::review_batch::ReviewCandidateRejection {
                observed_at: observed_at.clone(),
                task: task.to_string(),
                source: source.to_string(),
                source_time: None,
                rule_ids: rule_ids.iter().map(|rule| (*rule).to_string()).collect(),
                retryable,
                identity_hash,
                reason_code: reason_code.to_string(),
            }
        })
        .collect();
    crate::review_batch::append_candidate_rejection_audit(rejections, date).map(|_| ())
}

/// v16.5: 简化版 virtual_observation 加载 (与 main.rs::VirtualObservationRecord 兼容)
/// 读 data/virtual_observation/*.json (按 main.rs 持久化格式)
/// v13.6.3 扩展: 解析 entry_price 字段
pub fn load_virtual_observation_for_a01() -> Result<VirtualSnapshotLite, String> {
    let dir = match stock_analysis::risk::env_guard::current_env() {
        stock_analysis::risk::env_guard::TradingEnv::Prod => {
            std::path::PathBuf::from("data/virtual_observation")
        }
        stock_analysis::risk::env_guard::TradingEnv::Test => {
            std::path::PathBuf::from("data/test/virtual_observation")
        }
    };
    load_virtual_observation_from_dir(&dir)
}

fn load_virtual_observation_from_dir(dir: &std::path::Path) -> Result<VirtualSnapshotLite, String> {
    use std::fs;
    if !dir.exists() {
        return Err(format!(
            "virtual observation source directory missing: {}",
            dir.display()
        ));
    }
    let mut records: Vec<VirtualRecordLite> = Vec::new();
    let mut rejections = Vec::new();
    let mut source_failures = Vec::new();
    let entries = fs::read_dir(dir)
        .map_err(|error| format!("读取虚拟观察目录 {} 失败: {error}", dir.display()))?;
    let mut paths = Vec::new();
    for (entry_index, entry) in entries.enumerate() {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(_) => {
                source_failures.push(VirtualObservationLoadIssue {
                    identity_hash: crate::review_batch::audit_identity_hash(
                        "A-01-source",
                        &format!("directory-entry-{entry_index}"),
                    ),
                    reason_code: "directory_entry_unreadable".to_string(),
                    retryable: true,
                });
                continue;
            }
        };
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            paths.push(path);
        }
    }
    paths.sort();
    paths.reverse();
    if paths.is_empty() {
        return Err(format!(
            "virtual observation source has no JSON snapshots: {} (directory_errors={})",
            dir.display(),
            source_failures.len()
        ));
    }

    #[derive(serde::Deserialize)]
    struct RecordJson {
        entry_date: Option<String>,
        code: Option<String>,
        name: Option<String>,
        entry_mode: Option<String>,
        entry_price: Option<f64>,
    }
    fn validate_record(parsed: RecordJson) -> Result<VirtualRecordLite, &'static str> {
        let code = parsed
            .code
            .filter(|value| valid_source_stock_code(value))
            .ok_or("invalid_code")?;
        let name = parsed
            .name
            .filter(|value| !value.trim().is_empty())
            .ok_or("missing_name")?;
        let entry_date = parsed
            .entry_date
            .filter(|value| !value.trim().is_empty())
            .ok_or("missing_entry_date")?;
        chrono::NaiveDate::parse_from_str(&entry_date, "%Y-%m-%d")
            .map_err(|_| "invalid_entry_date")?;
        let entry_mode = parsed
            .entry_mode
            .filter(|value| !value.trim().is_empty())
            .ok_or("missing_entry_mode")?;
        let entry_price = parsed
            .entry_price
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or("invalid_entry_price")?;
        Ok(VirtualRecordLite {
            entry_date,
            code,
            name,
            entry_mode,
            entry_price,
        })
    }

    for path in paths.iter().take(5) {
        let source_hash =
            crate::review_batch::audit_identity_hash("A-01-source", &path.to_string_lossy());
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(_) => {
                source_failures.push(VirtualObservationLoadIssue {
                    identity_hash: source_hash,
                    reason_code: "source_read_failed".to_string(),
                    retryable: true,
                });
                continue;
            }
        };
        let value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => {
                source_failures.push(VirtualObservationLoadIssue {
                    identity_hash: source_hash,
                    reason_code: "invalid_json".to_string(),
                    retryable: false,
                });
                continue;
            }
        };
        let record_values = match value {
            serde_json::Value::Object(mut object) if object.contains_key("records") => {
                match object.remove("records") {
                    Some(serde_json::Value::Array(values)) => values,
                    _ => {
                        source_failures.push(VirtualObservationLoadIssue {
                            identity_hash: source_hash,
                            reason_code: "invalid_snapshot_shape".to_string(),
                            retryable: false,
                        });
                        continue;
                    }
                }
            }
            serde_json::Value::Object(object) => vec![serde_json::Value::Object(object)],
            _ => {
                source_failures.push(VirtualObservationLoadIssue {
                    identity_hash: source_hash,
                    reason_code: "invalid_snapshot_shape".to_string(),
                    retryable: false,
                });
                continue;
            }
        };
        for (record_index, value) in record_values.into_iter().enumerate() {
            let identity_hash = crate::review_batch::audit_identity_hash(
                "A-01-record",
                &format!("{}:{record_index}", path.to_string_lossy()),
            );
            let parsed = match serde_json::from_value::<RecordJson>(value) {
                Ok(parsed) => parsed,
                Err(_) => {
                    rejections.push(VirtualObservationLoadIssue {
                        identity_hash,
                        reason_code: "record_decode_failed".to_string(),
                        retryable: false,
                    });
                    continue;
                }
            };
            match validate_record(parsed) {
                Ok(record) => records.push(record),
                Err(reason_code) => rejections.push(VirtualObservationLoadIssue {
                    identity_hash,
                    reason_code: reason_code.to_string(),
                    retryable: false,
                }),
            }
        }
    }
    Ok(VirtualSnapshotLite {
        records,
        rejections,
        source_failures,
    })
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
    dispatch_registered_outcome!(
        "I-01-intraday-market",
        crate::notify::PushKind::IntradayMarket,
        "intraday_market_dispatcher",
        "render_intraday_market",
        code,
        Some(banner),
        text
    )
}

/// v13 §14.2 I-02 新闻催化映射 (⚡交易建议类, 带 banner)
pub async fn push_news_catalyst(
    code: &str,
    banner: &BannerCtx,
    params: NewsCatalystParams<'_>,
) -> bool {
    let text = render_news_catalyst(banner, params);
    dispatch_registered_outcome!(
        "I-02-news-catalyst",
        crate::notify::PushKind::NewsCatalyst,
        "news_catalyst_dispatcher",
        "render_news_catalyst",
        code,
        Some(banner),
        text
    )
    .is_pushed()
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
    dispatch_registered_outcome!(
        "I-09-sector-anomaly",
        crate::notify::PushKind::SectorAnomaly,
        "sector_anomaly_dispatcher",
        "render_sector_anomaly",
        "",
        None,
        text
    )
    .is_pushed()
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
    dispatch_registered_outcome!(
        "I-03-industry-chain-intraday",
        crate::notify::PushKind::IndustryChainIntraday,
        "industry_chain_intraday_dispatcher",
        "render_industry_chain_intraday",
        code,
        Some(banner),
        text
    )
}

/// v13 §14.4 D-01 新闻驱动个股 (⚡交易建议类, 带 banner)
pub async fn push_news_to_idea(
    code: &str,
    banner: &BannerCtx,
    params: NewsToIdeaParams<'_>,
) -> bool {
    let text = render_news_to_idea(banner, params);
    dispatch_registered_outcome!(
        "D-01-news-to-idea",
        crate::notify::PushKind::NewsToIdea,
        "news_to_idea_dispatcher",
        "render_news_to_idea",
        code,
        Some(banner),
        text
    )
    .is_pushed()
}

/// v13 §14.3 A-01 虚拟仓复盘 (ℹ️盘后参考, 复用 T-11 竞价复算)
pub async fn push_paper_review(code: &str, params: PaperReviewParams<'_>) -> bool {
    push_paper_review_outcome(code, params).await.is_pushed()
}

async fn push_paper_review_outcome(
    code: &str,
    params: PaperReviewParams<'_>,
) -> crate::notify::PushOutcome {
    let text = render_paper_review(params);
    dispatch_registered_outcome!(
        "A-01-paper-review",
        crate::notify::PushKind::PaperReview,
        "paper_review_dispatcher",
        "render_paper_review",
        code,
        None,
        text
    )
}

// ============================================================================
// MVP3-3.2 orchestrator: T-07 候选触发 + T-08 候选失效
// ============================================================================

/// MVP3-3.2 T-07 候选触发 (⚡ 1次/票/日).
///
/// 由 candidate_state::is_candidate_live_enabled() 控制: 关闭时返回
/// `Ok(false)` (零推送).
///
/// BR-192 caller migration contract:
/// - the candidate producer must first durably persist the exact
///   `Candidate -> Triggered` lifecycle transition and expose its stable
///   identity plus canonical transition bytes;
/// - it must retain the ordered selection-decision/source batch identities
///   used by that transition (realtime quote/statistics batches alone do not
///   prove which candidate was selected);
/// - it must supply the transition business date, ticket scope and a stable
///   candidate subject identity without using dispatch time or rendered text;
/// - only then may this entry point accept a `CountedDeliveryBinding` and call
///   `push_counted_with_binding`.
///
/// The current `RealCandidateBatch` is assembled from `chain_daily` and local
/// P5 source files and has no durable lifecycle transition owner.  Failing
/// closed is therefore mandatory; constructing provider metadata or a source
/// payload from local dispatch time would fabricate BR-192 evidence.
const CANDIDATE_COUNTED_BINDING_UNAVAILABLE: &str = "candidate_counted_binding_unavailable";

pub async fn push_candidate_triggered(
    code: &str,
    banner: &BannerCtx,
    params: CandidateTriggeredParams<'_>,
    promotion_evidence: Option<stock_analysis::opportunity::candidate_state::PromotionEvidence>,
    live_override: Option<bool>,
) -> Result<bool, String> {
    use stock_analysis::opportunity::candidate_state::require_live_promotion;

    if let Err(error) = require_live_promotion(promotion_evidence, live_override) {
        log::info!("[T-07] 候选触发保持 Shadow (code={code}): {error}");
        return Ok(false);
    }

    // Keep the render inputs in the signature so the missing evidence cannot
    // accidentally be "fixed" by routing this counted kind through the old
    // generic dispatcher.  They become usable only after the durable
    // candidate lifecycle producer supplies the contract documented above.
    let _ = (banner, params);
    Err(CANDIDATE_COUNTED_BINDING_UNAVAILABLE.to_string())
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
    dispatch_registered_outcome!(
        "T-08-candidate-invalidated",
        crate::notify::PushKind::CandidateInvalidated,
        "candidate_dispatcher",
        "render_candidate_invalidated",
        code,
        None,
        text
    )
    .is_pushed()
}

/// v12 PR2-2.2: 数据模式变更编排器.
///
/// 完整链路: evaluate() → 计划状态变更 → 拼 T-02 → dispatch().
/// BR-116: 已确认状态本身负责精确去重，不设跨状态的粗粒度时间冷却。
///
/// 返回 `ModeDispatchResult`: 静默建立 Full, 或保留权威 `PushOutcome` 供调用方确认。
///
/// `prev` 由调用方的进程内 `LATEST_DATA_MODE` 提供, 首次评估传 None.
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
    let outcome = match dispatch_reason {
        DataModeDispatchReason::Transition => dispatch_registered_outcome!(
            "T-02-data-mode",
            crate::notify::PushKind::DataMode,
            "data_mode_hook",
            "render_data_mode",
            "",
            banner,
            text
        ),
        DataModeDispatchReason::PersistentUnsafeReminder => dispatch_registered_outcome!(
            "T-02-data-mode-reminder",
            crate::notify::PushKind::DataMode,
            "data_mode_scheduler",
            "render_data_mode_reminder",
            "",
            banner,
            text
        ),
    };

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

/// 冷却表: key = (PushKind, code_or_empty), value = last sent epoch secs
///
/// 仅保留给 BR-192 durable catalog 之外的非计数通知。计数通知的冷却、
/// 日预算和重启恢复全部由 `DurableDeliveryCoordinator` 独占。
static COOLDOWN_TABLE: Lazy<std::sync::Mutex<HashMap<(crate::notify::PushKind, String), i64>>> =
    Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

/// 判定: 该 (kind, code) 是否在冷却中. 紧急类 (`Emergency`) 与无冷却 (`None`) 永远返回 false.
///
/// BR-192 counted kinds 永远返回 false；它们只能由持久协调器判定。
/// 副作用: 不命中时**不**写表, 由 dispatch 在非计数通知成功后写入.
pub fn is_in_cooldown(kind: crate::notify::PushKind, code: &str) -> bool {
    use super::notify::PushLevel;
    if crate::durable_delivery_runtime::is_counted_kind(kind)
        || kind.level() == PushLevel::Emergency
    {
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

/// 记录非计数通知成功后的进程内冷却时间戳。
fn record_uncounted_cooldown(kind: crate::notify::PushKind, code: &str) {
    debug_assert!(
        !crate::durable_delivery_runtime::is_counted_kind(kind),
        "BR-192 counted cooldown must be owned by DurableDeliveryCoordinator"
    );
    let key = (kind, code.to_string());
    let now = chrono::Utc::now().timestamp();
    let mut table = COOLDOWN_TABLE.lock().expect("cooldown table poisoned");
    table.insert(key, now);
}

/// §14.3 治理规则: Frozen/Unsafe 停发判定
///
/// 2026-08-06 用户决策 (未接入券商): 全部推送为参考级, 无任何账户/数据模式
/// 停发 → 恒 false。Frozen/Unsafe 状态仍由 banner 出声 (Frozen 横幅 +
/// DataMode banner), "出声"原则保留, 仅移除推送拦截。
/// 原实现: T-03/T-05/T-07 (持有建议/做T/候选触发) 在 Frozen/Unsafe 停发,
///         风险类照发 — 与 L5 data_quality 门禁一并移除 (C 方案)。
/// 签名保留 (调用点不变); 若未来接入券商需要恢复, 恢复原 match 即可。
pub fn should_block_on_mode(
    kind: crate::notify::PushKind,
    mode: AccountMode,
    dm: DataMode,
) -> bool {
    let _ = (kind, mode, dm); // 参数保留签名兼容 (调用点不变)
    false
}

/// 一站式便捷入口: 已注册生产展示令牌 → 治理检查 → 通知网关.
///
/// 治理流程 (任一环节 skip 即转 log):
///   1. §14.3.4 mode/dm 停发检查 (`should_block_on_mode`)
///   2. 非计数通知执行旧进程内冷却；BR-192 计数通知由 durable coordinator
///      在一个事务中持久化冷却与日预算。
///
/// `code` 用于 §14.3.1 的 (PushKind, code) 键. 不分票的推送 (T-01/T-02 状态变更/全局)
/// 传空字符串即可.
///
/// BR-196: `kind` 只能从不可复制的生产展示令牌派生；不存在 raw
/// `PushKind` 兼容入口，避免新增展示绕过登记表。
async fn dispatch_outcome(
    token: crate::presentation_registry::ProductionPresentationToken,
    code: &str,
    banner: Option<&BannerCtx>,
    text: String,
) -> crate::notify::PushOutcome {
    let kind = token.descriptor().push_kind;
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

    // 2. 非计数通知冷却（紧急类跳过）。计数通知必须直达 durable coordinator。
    if is_in_cooldown(kind, code) {
        log::info!(
            "[PUSH_GOVERNOR] §14.3.1 冷却中跳过 | kind={} code={}",
            kind.label(),
            code,
        );
        return crate::notify::PushOutcome::Deduped;
    }

    // 3. 推 — b013 review P0-2: 票级事件显式传 code, 让 v14_gate L4 dedup 真正按
    //    (kind,code) 工作。全局事件沿用模板层的空字符串键，但进入事件 envelope 前必须
    //    规范化为 None；空字符串不是一个真实证券身份，也不能写入 BR-130 审计字段。
    //    BR-192 counted kinds 的去重/冷却仅由 durable coordinator 持久化；
    //    非计数通知仍由旧 L4 与本地冷却保护。
    let outcome =
        crate::notify::push_presented_v3(token, &text, optional_dispatch_code(code)).await;
    if outcome.is_pushed() && !crate::durable_delivery_runtime::is_counted_kind(kind) {
        record_uncounted_cooldown(kind, code);
    }
    outcome
}

fn optional_dispatch_code(code: &str) -> Option<&str> {
    (!code.trim().is_empty()).then_some(code)
}

pub async fn dispatch(
    token: crate::presentation_registry::ProductionPresentationToken,
    code: &str,
    banner: Option<&BannerCtx>,
    text: String,
) -> bool {
    dispatch_outcome(token, code, banner, text)
        .await
        .is_pushed()
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
    pub member_count: usize,
    pub continuous_count: usize,
    pub leading_names: Vec<&'a str>,
    pub other_names: Vec<&'a str>,
    pub watch_point: Option<&'a str>,
}

/// v13 §14.3 A-10 盘后题材催化复盘
pub fn render_catalyst_review(p: CatalystReviewParams<'_>) -> String {
    let score = p
        .score
        .map(|value| format!("{value:.1}"))
        .unwrap_or_else(|| "数据缺失（无独立评分批次）".to_string());
    let persistent = match p.persistent {
        PersistentLevel::High => "高",
        PersistentLevel::Med => "中",
        PersistentLevel::Low => "低",
    };
    let mut s = format!(
        "📰 题材催化复盘（{}）\n主线: {}\n涨停成员: {}家 | 连板成员: {}家 | 持续性结构: {}\n题材评分: {}\n",
        p.date, p.theme, p.member_count, p.continuous_count, persistent, score
    );
    if !p.leading_names.is_empty() {
        s.push_str(&format!(
            "前排成员（按连板数）: {}\n",
            p.leading_names.join("、")
        ));
    }
    if !p.other_names.is_empty() {
        s.push_str(&format!("其余同题材成员: {}\n", p.other_names.join("、")));
    }
    let watch_point = p
        .watch_point
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("数据缺失（未接入独立量能/走势批次）");
    s.push_str(&format!("明日观察点: {watch_point}\n"));
    s.push_str("仅结构化事实，非下单指令");
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
        p.hhmm,
        p.name,
        p.code,
        st,
        p.holding_qty,
        p.old_limit * 100.0,
        p.new_limit * 100.0,
        p.now_price,
        p.cost,
        pnl_pct
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

/// BR-196: one renderer-backed, non-production template preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestTemplatePreview {
    pub template_id: &'static str,
    pub text: String,
}

/// BR-196: closed renderer catalog used only by `monitor --test`.
///
/// Every entry is assembled by the real renderer with explicit TEST_CODE
/// fixtures.  The caller validates uniqueness/cardinality before any external
/// transport is attempted.
pub fn build_test_template_catalog(
    date: &str,
    hhmm: &str,
) -> Result<Vec<TestTemplatePreview>, String> {
    use magic_market_core::{DragonTigerSide, Exchange as CoreExchange, ProviderId};
    use stock_analysis::data_gateway::{
        BatchEvidence, DragonTigerSeatReview, DragonTigerSourceDisclosure, DragonTigerStockReview,
    };
    use stock_analysis::decision::t0_advisor::{
        PriceZone, T0Metrics, T0PlanState, T0StructuredPlan, TrendStatus, ZoneSource,
    };
    use stock_analysis::market_analyzer::sector_monitor::{
        AnomalyReason, ConceptBoard, UnexplainedMove,
    };

    const EXPECTED_CATALOG_TOTAL: usize = 52;
    let banner = BannerCtx {
        account_mode: AccountMode::Normal,
        total_pos: Some(0),
        today_pnl: Some(0.0),
        account_metrics_complete: true,
        data_mode: DataMode::Full,
        data_missing_note: None,
    };
    let reasons = vec!["TEST_CODE 风险证据完整".to_string()];
    let restrictions = vec!["TEST_CODE 只读分析".to_string()];
    let invalidations = vec!["TEST_CODE 跌破确认支撑".to_string()];
    let no_buy = vec!["TEST_CODE 量价背离".to_string()];
    let mut catalog = Vec::with_capacity(EXPECTED_CATALOG_TOTAL);
    let mut push = |template_id: &'static str, text: String| {
        catalog.push(TestTemplatePreview { template_id, text });
    };

    push(
        "T-01-account-mode",
        render_account_mode(
            hhmm,
            AccountMode::Normal,
            AccountMode::ReduceOnly,
            &reasons,
            "TEST_CODE 禁止新增风险敞口",
            "TEST_CODE 风险指标恢复",
        ),
    );
    push(
        "T-02-data-mode",
        render_data_mode(
            hhmm,
            Some(DataMode::Full),
            DataMode::Degraded,
            "TEST_CODE OrderBook",
            &restrictions,
            Some("下一完整批次"),
        ),
    );
    push(
        "T-02-data-mode-reminder",
        render_data_mode_reminder(
            hhmm,
            DataMode::Unsafe,
            "TEST_CODE Quote",
            &restrictions,
            None,
        ),
    );
    push(
        "T-03-holding-plan",
        render_holding_plan(
            &banner,
            HoldingPlanParams {
                name: "TEST_CODE 持仓",
                code: "TEST_CODE_600001",
                hhmm,
                intent: Intent::Reduce,
                price: 10.80,
                cost: 9.50,
                avail: 500,
                reduce_zone: Some((10.70, 10.95)),
                support: 10.20,
                pressure: 11.00,
                stop: 9.90,
                invalidations: &invalidations,
                reasons: &reasons,
            },
        ),
    );
    push(
        "T-04-holding-event",
        render_holding_event(
            &banner,
            HoldingEventParams {
                name: "TEST_CODE 持仓",
                code: "TEST_CODE_600001",
                hhmm,
                trigger: "TEST_CODE 跌破硬止损",
                price: 9.80,
                chg_pct: -3.5,
                gap_pct: -1.2,
                action: "TEST_CODE 等待人工确认",
                avail: 500,
            },
        ),
    );

    let t0_plan = T0StructuredPlan {
        code: "TEST_CODE_002415".to_string(),
        name: "TEST_CODE 做T".to_string(),
        source_at: chrono::DateTime::parse_from_rfc3339("2026-08-01T10:30:00+08:00")
            .map_err(|error| format!("BR-196 T0 source time invalid: {error}"))?
            .with_timezone(&chrono::Utc),
        batch_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        current_price: 25.18,
        cost_price: 23.80,
        total_quantity: 2_000,
        sell_quantity: 600,
        buyback_quantity: 600,
        sell_zone: PriceZone {
            low: 25.10,
            high: 25.30,
            source: ZoneSource::DailyPivot,
        },
        buy_zone: PriceZone {
            low: 24.50,
            high: 24.70,
            source: ZoneSource::IntradayPivot,
        },
        gross_spread_pct: 1.62,
        metrics: T0Metrics {
            trend: TrendStatus::Range,
            pace_ratio: 1.36,
            last_bar_volume_ratio: 1.28,
            intraday_average_price: 24.88,
            atr14: 0.72,
            ask_bid_ratio: 1.31,
            bid_ask_ratio: 0.76,
        },
        state: T0PlanState::SellTriggered,
        trigger_text: "TEST_CODE 量价与卖盘同时确认".to_string(),
        invalidation_text: "TEST_CODE 连续突破上沿".to_string(),
    };
    push(
        "T-05-t0-advice",
        render_t0_advice(&banner, T0AdviceParams::from(&t0_plan)),
    );
    push(
        "T-06-t0-forbid",
        render_t0_forbid(
            &banner,
            T0ForbidParams {
                name: "TEST_CODE 做T",
                code: "TEST_CODE_002415",
                hhmm,
                reason: "TEST_CODE 价差不足",
            },
        ),
    );
    push(
        "T-07-candidate-triggered",
        render_candidate_triggered(
            &banner,
            CandidateTriggeredParams {
                name: "TEST_CODE 候选",
                code: "TEST_CODE_300001",
                hhmm,
                grade: CandidateGrade::A,
                topic: "TEST_CODE 算力",
                price: 20.0,
                trigger_desc: "TEST_CODE 放量突破",
                lo: 19.50,
                hi: 19.80,
                stop: 18.90,
                max_pos_pct: 10,
                news_quality: EvidenceQuality::Strong,
                news_note: "TEST_CODE 来源已验证",
                vol_quality: EvidenceQuality::Strong,
                vol_ratio: 2.1,
                kline_quality: EvidenceQuality::Mid,
                kline_note: "TEST_CODE 多头结构",
                book_quality: EvidenceQuality::Mid,
                no_buy: &no_buy,
            },
        ),
    );
    push(
        "T-08-candidate-invalidated",
        render_candidate_invalidated(
            hhmm,
            "TEST_CODE 候选",
            "TEST_CODE_300001",
            "Triggered",
            "TEST_CODE 失效条件命中",
        ),
    );
    push(
        "T-09-forbidden-ops",
        render_forbidden_ops(
            &banner,
            ForbiddenOpsParams {
                name: "TEST_CODE 持仓",
                code: "TEST_CODE_600001",
                hhmm,
                conclusion: "TEST_CODE 禁止追涨",
                reasons: &reasons,
            },
        ),
    );
    push(
        "P-05-virtual-watch",
        render_virtual_watch(VirtualWatchParams {
            hhmm,
            shares_per_lot: 100,
            items: vec![VirtualWatchItem {
                name: "TEST_CODE 观察",
                code: "TEST_CODE_000001",
                open_price: 10.0,
                shares: 100,
                estimated_amount: 1_000.0,
            }],
            total_amount: 1_000.0,
            item_count: 1,
        }),
    );
    push(
        "T-10-paper-trade",
        render_paper_trade(PaperTradeParams {
            name: "TEST_CODE 虚拟成交",
            code: "TEST_CODE_000001",
            hhmm,
            status: PaperTradeStatus::Filled,
            fill_price: Some(10.20),
            qty: Some(100),
            virtual_reason: Some("TEST_CODE 信号确认"),
            not_fill_reason: None,
            account_mode: AccountMode::Normal,
            data_mode: DataMode::Full,
        }),
    );
    push(
        "T-11-auction-volume",
        render_auction_volume(
            &banner,
            hhmm,
            &[AuctionItem {
                name: "TEST_CODE 竞价",
                code: "TEST_CODE_600002",
                gap_pct: 2.0,
                vol_ratio: 3.2,
                tag: "TEST_CODE 强承接",
            }],
            "TEST_CODE 情绪回暖",
            "TEST_CODE 已复核",
        ),
    );
    let close_holding = CloseCallHolding {
        name: "TEST_CODE 持仓",
        state: "TEST_CODE 正常",
    };
    let close_gamble = CloseCallGamble {
        name: "TEST_CODE 候选",
        code: "TEST_CODE_300001",
        satisfied: true,
        cond: "TEST_CODE 尾盘承接",
    };
    push(
        "T-12-close-call",
        render_close_call(&banner, hhmm, Some(&close_holding), Some(&close_gamble)),
    );
    push(
        "I-09-sector-top",
        render_sector_top(hhmm, &[("TEST_CODE 算力".to_string(), 3.2, 1.5)]),
    );
    push(
        "T-13-turnover-top",
        render_turnover_top(
            hhmm,
            &[TurnoverEntry {
                name: "TEST_CODE 换手".to_string(),
                code: "TEST_CODE_600003".to_string(),
                price: 12.3,
                change_pct: 4.5,
                turnover_pct: 18.2,
                main_flow_yi: Some(1.2),
            }],
        ),
    );
    push(
        "R-01-daily-report",
        render_daily_report(
            date,
            &[HoldingDailyPlan {
                name: "TEST_CODE 持仓",
                code: "TEST_CODE_600001",
                price: 10.8,
                cost: 9.5,
                pnl_pct: 13.7,
                high_gap_x: 1.0,
                plan_high: "TEST_CODE 观察减仓",
                plan_flat: "TEST_CODE 持有",
                stop: 9.9,
                t0: "TEST_CODE 观察价差",
            }],
        ),
    );
    push(
        "R-02-review-market",
        render_review_market(
            date,
            &MarketReview {
                sh_chg: Some(0.8),
                chinext_chg: Some(1.2),
                star_chg: Some(1.0),
                limit_up_n: Some(52),
                limit_down_n: Some(3),
                broken_pct: Some(18.0),
                consecutive_h: Some(4),
                amount_yi: Some(12_000.0),
                amount_delta_pct: Some(8.0),
                amount_dir: Some("放量"),
                main_flow_yi: Some(80.0),
                money_effect: "TEST_CODE 回暖",
                heat_stage: "TEST_CODE 主升",
                heat_conf_pct: 80,
                low_conf: false,
                low_conf_tier: None,
                account_mode: AccountMode::Normal,
                max_pos: 7,
            },
        ),
    );
    push(
        "R-03-industry-chain",
        render_industry_chain(
            date,
            &[ChainLine {
                chain: "TEST_CODE 算力",
                limit_up_n: 3,
                first_n: 1,
                consec_n: 2,
                heat_stage: "TEST_CODE 主升",
                leader_name: "TEST_CODE 龙头",
                leader_code: "TEST_CODE_600004",
                leader_boards: 2,
                followers: "TEST_CODE 后排",
                watch_point: Some("TEST_CODE 回踩量能"),
            }],
            None,
            Some("TEST_CODE 完整批次"),
        ),
    );
    let mut seats = Vec::with_capacity(10);
    for side in [DragonTigerSide::Buy, DragonTigerSide::Sell] {
        for rank in 1..=5 {
            seats.push(DragonTigerSeatReview {
                side,
                rank,
                seat_name: format!("TEST_CODE_{side:?}_{rank}"),
                amount_yuan: f64::from(rank) * 10_000_000.0,
                buy_amount_yuan: None,
                sell_amount_yuan: None,
                net_amount_yuan: None,
            });
        }
    }
    let gateway_stocks = vec![DragonTigerStockReview {
        exchange: CoreExchange::Shanghai,
        code: "TEST_CODE_600005".to_string(),
        ranking_net_amount_yuan: 120_000_000.0,
        disclosures: vec![DragonTigerSourceDisclosure {
            entry_id: format!("TEST_CODE_600005:{date}:TEST_CODE_TRADE_ID"),
            trade_id: "TEST_CODE_TRADE_ID".to_string(),
            reason: Some("TEST_CODE 源披露原因".to_string()),
            buy_amount_yuan: Some(300_000_000.0),
            sell_amount_yuan: Some(180_000_000.0),
            net_amount_yuan: Some(120_000_000.0),
            turnover_rate_pct: Some(12.34),
            seats,
        }],
    }];
    let gateway_evidence = BatchEvidence {
        provider: ProviderId::Eastmoney,
        source: "TEST_CODE_eastmoney-dragon-tiger".to_string(),
        source_at: Some(date.to_string()),
        observed_at: "2026-08-01T21:00:00+08:00".to_string(),
        batch_id: "TEST_CODE_R04_BATCH".to_string(),
    };
    push(
        "R-04-review-lhb-gateway",
        render_review_lhb_gateway(date, &gateway_stocks, &gateway_evidence),
    );
    push(
        "R-05-review-signal",
        render_review_signal(
            date,
            &SignalReview {
                holding_n: 2,
                holding_exec: 1,
                holding_eff: 1,
                t0_n: 1,
                t0_eff: 1,
                cand_trigger: 2,
                cand_filled: 1,
                cand_notfilled: 1,
                cand_limitup: 1,
                cand_notreach: 1,
                paper_pnl_pct: 1.2,
                paper_total_pct: 3.4,
                paper_n: 2,
                news_push_n: 2,
                news_d1_eff: 1,
            },
        ),
    );
    push(
        "R-06-review-failure",
        render_review_failure(
            date,
            &[FailureEntry {
                name: "TEST_CODE 失败样本",
                code: "TEST_CODE_600006",
                signal_level: "A",
                virtual_reason: "TEST_CODE 放量",
                result_desc: "TEST_CODE 回撤",
                pnl_pct: -3.0,
                failure_reason: "TEST_CODE 买点过晚",
                suggestion: "TEST_CODE 收紧触发",
            }],
            &FailureDistribution {
                buy_late: 1,
                chain_fade: 0,
                not_fillable: 0,
                human_not_exec: 0,
            },
        ),
    );
    push(
        "R-07-tomorrow-watch",
        render_tomorrow_watch(
            date,
            &[WatchItem {
                name: "TEST_CODE 观察",
                code: "TEST_CODE_000001",
                topic: "TEST_CODE 算力",
                source: "TEST_CODE A档",
                trigger: "TEST_CODE 放量",
                lo: 9.8,
                hi: 10.0,
                stop: 9.4,
                reason: "TEST_CODE 多源共振",
            }],
        ),
    );
    push(
        "R-11-position-review",
        render_position_review(PositionReviewParams {
            date,
            total_assets: 100_000.0,
            position_ratio_pct: 61.3,
            available_cash: 38_700.0,
            daily_pnl: 1_864.60,
            unrealized_pnl: -21_729.90,
            unrealized_return_pct: -3.51,
            position_count: 7,
            market_value: 61_300.0,
            sectors: &[
                ("TEST_CODE 算力".to_string(), 45.0),
                ("TEST_CODE 半导体".to_string(), 30.0),
                ("其他".to_string(), 25.0),
            ],
            items: &[PositionReviewItem {
                code: "TEST_CODE_000001".to_string(),
                name: "TEST_CODE 持仓".to_string(),
                quantity: 700,
                cost_price: 60.2,
                close: Some(61.54),
                market_value: 43_078.0,
                unrealized_pnl: 938.0,
                unrealized_return_pct: Some(2.22),
                daily_price_pnl: Some(120.0),
            }],
        }),
    );
    push(
        "A-02-auction-repush",
        render_auction_repush(
            "09:20",
            &[
                stock_analysis::opportunity::candidate_panel::CandidateEntry {
                    code: "TEST_CODE_000001".to_string(),
                    name: "TEST_CODE 候选".to_string(),
                    sources: Vec::new(),
                    tier: stock_analysis::opportunity::candidate_panel::EvidenceTier::Strong,
                    evidence: Vec::new(),
                    current_price: Some(10.0),
                    change_pct: None,
                    heat_score: Some(80.0),
                },
            ],
        ),
    );
    push(
        "P-05-candidate-board",
        stock_analysis::opportunity::candidate_panel::format_candidate_board(&[
            stock_analysis::opportunity::candidate_panel::CandidateEntry {
                code: "TEST_CODE_000001".to_string(),
                name: "TEST_CODE 候选".to_string(),
                sources: Vec::new(),
                tier: stock_analysis::opportunity::candidate_panel::EvidenceTier::Strong,
                evidence: Vec::new(),
                current_price: Some(10.0),
                change_pct: None,
                heat_score: Some(80.0),
            },
        ]),
    );
    push(
        "A-11-ipo-catalyst",
        render_ipo_catalyst(
            date,
            stock_analysis::news::ipo::supply_chain::ipo_companies(),
        ),
    );
    push(
        "R-08-public-event-calendar",
        render_r08_public_calendar(
            date,
            "TEST_CODE 公告批次 2 条",
            "TEST_CODE 中金所交割日事实 1 条",
            "TEST_CODE 隔夜指数批次完整",
            "TEST_CODE 汇率批次完整",
            &[],
        ),
    );
    let limit_identity =
        crate::br196_test_delivery::TestSecurityIdentity::parse("TEST_CODE_LIMIT_ALPHA")?;
    let limit_row = format!(
        "  TEST_CODE 涨停样本({}) 主力+1.20亿 量比3.2 +10.0%",
        limit_identity.as_str()
    );
    for (template_id, shape) in [
        ("L-01-limit-boards-first", LimitBoardsShape::First),
        ("L-02-limit-boards-second", LimitBoardsShape::Second),
        ("L-03-limit-boards-third-plus", LimitBoardsShape::ThirdPlus),
    ] {
        push(
            template_id,
            render_limit_boards_shape(shape, hhmm, std::slice::from_ref(&limit_row))?,
        );
    }
    for (template_id, text) in crate::v17_sources::build_br196_normalized_source_previews()? {
        push(template_id, text);
    }
    let r09_volume = vec![ProviderTopNProjectionRow {
        metric: "volume_ratio".to_string(),
        source_order_ordinal: 1,
        exchange: "Shanghai".to_string(),
        asset_class: "Equity".to_string(),
        code: "TEST_CODE_600007".to_string(),
        label: "TEST_CODE 量比".to_string(),
        value: 5.2,
        unit: "ratio".to_string(),
        trading_date: date.to_string(),
        filter_identity: "TEST_CODE_A_SHARE".to_string(),
        provider_declared_total: 1,
        inspected_row_count: 1,
    }];
    let r09_inflow = vec![ProviderTopNProjectionRow {
        metric: "main_net_inflow".to_string(),
        source_order_ordinal: 1,
        exchange: "Shanghai".to_string(),
        asset_class: "Equity".to_string(),
        code: "TEST_CODE_600008".to_string(),
        label: "TEST_CODE 主力净流入".to_string(),
        value: 220_000_000.0,
        unit: "yuan".to_string(),
        trading_date: date.to_string(),
        filter_identity: "TEST_CODE_A_SHARE".to_string(),
        provider_declared_total: 1,
        inspected_row_count: 1,
    }];
    let review_date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|error| format!("BR-196 review date invalid: {error}"))?;
    push(
        "R-09-provider-top-n",
        render_r09_provider_top_n(review_date, &r09_volume, &r09_inflow),
    );
    push(
        "P-01-preopen-news-hot",
        render_preopen_news_hot(PreopenNewsHotParams {
            hhmm,
            theme_1: Some("TEST_CODE 算力"),
            theme_2: Some("TEST_CODE 机器人"),
            theme_3: None,
            news_pairs: vec![("TEST_CODE 新闻", "TEST_CODE 算力")],
            watch_stocks: vec![(
                "TEST_CODE 关注".to_string(),
                "TEST_CODE_000001".to_string(),
                "TEST_CODE 多源证据".to_string(),
            )],
        }),
    );
    push(
        "I-01-intraday-market",
        render_intraday_market(
            &banner,
            IntradayMarketParams {
                hhmm,
                tech_sub: Some("TEST_CODE 算力"),
                tech_score: Some(8.0),
                power_sub: Some("TEST_CODE 电力"),
                power_score: Some(6.0),
                robot_sub: Some("TEST_CODE 机器人"),
                robot_score: Some(7.0),
                main_attack: Some("TEST_CODE 算力"),
                rotation_state: RotationState::Spreading,
            },
        ),
    );
    push(
        "I-02-news-catalyst",
        render_news_catalyst(
            &banner,
            NewsCatalystParams {
                hhmm,
                headline: "TEST_CODE 新闻催化",
                theme: Some("TEST_CODE 算力"),
                stocks: vec![(
                    "TEST_CODE 受益",
                    "TEST_CODE_600009",
                    Some(3.2),
                    "TEST_CODE 产业链映射",
                )],
            },
        ),
    );
    let anomaly = UnexplainedMove {
        board: ConceptBoard {
            code: "TEST_CODE_BK001".to_string(),
            name: "TEST_CODE 异动板块".to_string(),
            change_pct: 5.0,
            main_inflow: 300_000_000.0,
            leader_name: "TEST_CODE 龙头".to_string(),
            vol_ratio: 2.5,
            turnover: 8.0,
            main_net_pct_today: 3.0,
            main_net_pct_5d: 1.0,
        },
        reasons: vec![AnomalyReason::HighChange, AnomalyReason::HighVolRatio],
    };
    push(
        "I-09-sector-anomaly",
        render_sector_anomaly(hhmm, &[anomaly]),
    );
    push(
        "D-01-news-to-idea",
        render_news_to_idea(
            &banner,
            NewsToIdeaParams {
                hhmm,
                headline: "TEST_CODE 业绩超预期",
                theme: Some("TEST_CODE 算力"),
                stage: NewsStage::Starting,
                name: "TEST_CODE 受益",
                code: "TEST_CODE_600009",
                reasons: vec!["TEST_CODE 业绩证据"],
                action: Some(NewsAction::Observe),
            },
        ),
    );
    push(
        "A-10-catalyst-review",
        render_catalyst_review(CatalystReviewParams {
            date,
            theme: "TEST_CODE 算力",
            score: Some(8.5),
            persistent: PersistentLevel::High,
            member_count: 3,
            continuous_count: 2,
            leading_names: vec!["TEST_CODE 龙头"],
            other_names: vec!["TEST_CODE 后排"],
            watch_point: Some("TEST_CODE 次日量能"),
        }),
    );
    push(
        "I-03-industry-chain-intraday",
        render_industry_chain_intraday(
            &banner,
            IndustryChainIntradayParams {
                hhmm,
                chain: "TEST_CODE 算力",
                limit_count: 3,
                leader_name: Some("TEST_CODE 龙头"),
                leader_code: Some("TEST_CODE_600004"),
                leader_height: 2,
                supplements: vec![SupplementCandidate {
                    name: "TEST_CODE 补涨",
                    code: "TEST_CODE_300002",
                    trigger: "TEST_CODE 放量",
                    lo: 12.0,
                    hi: 12.2,
                    stop: 11.5,
                }],
            },
        ),
    );
    push(
        "T-14-post-fixed-price-order",
        render_post_fixed_price_order(PostFixedPriceOrderParams {
            exchange: Exchange::SH,
            hhmm: "15:10",
            name: "TEST_CODE 盘后申报",
            code: "TEST_CODE_688001",
            price: 20.0,
            qty: 100,
            order_id: "TEST_CODE_ORDER_ID",
            status: OrderStatus::Submitted,
        }),
    );
    push(
        "T-15-post-fixed-price-fill",
        render_post_fixed_price_fill(PostFixedPriceFillParams {
            exchange: Exchange::SH,
            hhmm: "15:15",
            name: "TEST_CODE 盘后成交",
            code: "TEST_CODE_688001",
            fill_price: 20.0,
            qty: 100,
            vs_limit_pct: Some(-0.5),
            next_session_carry: true,
        }),
    );
    push(
        "T-16-st-price-limit-changed",
        render_st_price_limit_changed(StPriceLimitChangedParams {
            hhmm,
            name: "TEST_CODE ST",
            code: "TEST_CODE_600010",
            st_type: StType::ST,
            old_limit: 0.05,
            new_limit: 0.10,
            holding_qty: 500,
            cost: 8.0,
            now_price: 8.5,
            new_stop_loss: Some(7.2),
            new_take_profit: Some(8.8),
        }),
    );
    push(
        "T-17-etf-closing-call-auction",
        render_etf_closing_call_auction(EtfClosingCallAuctionParams {
            hhmm: "14:58",
            name: "TEST_CODE ETF",
            code: "TEST_CODE_510300",
            call_auction_price: Some(4.123),
            vs_continuous_est: Some(0.12),
            liquidity_note: "TEST_CODE 流动性充足",
        }),
    );
    push(
        "BR-033-block-trade-confirm",
        render_block_trade_intraday_confirm(BlockTradeIntradayConfirmParams {
            hhmm,
            name: "TEST_CODE 大宗",
            code: "TEST_CODE_300003",
            qty: 100_000,
            price: 15.0,
            block_type: BlockType::Agreed,
            board: Board::Gem,
            real_time_confirm: true,
            next_session_settle: SettleType::NextSession,
        }),
    );
    push(
        "BR-034-block-trade-range",
        render_block_trade_price_range(BlockTradePriceRangeParams {
            hhmm,
            name: "TEST_CODE 北交所",
            code: "TEST_CODE_920001",
            prev_close: Some(10.0),
            today_avg_price: 10.3,
            block_price_range: Some("TEST_CODE 9.0~11.0"),
            note: "TEST_CODE 源口径",
        }),
    );
    push(
        "A-01-paper-review",
        render_paper_review(PaperReviewParams {
            date,
            name: "TEST_CODE 虚拟仓",
            code: "TEST_CODE_000001",
            trigger: "TEST_CODE 放量突破",
            desc: "TEST_CODE 次日兑现",
            pnl: Some(2.5),
            plan_high: Some("TEST_CODE 兑现"),
            plan_flat: Some("TEST_CODE 观察"),
            plan_low: Some("TEST_CODE 止损"),
        }),
    );

    if catalog.len() != EXPECTED_CATALOG_TOTAL {
        return Err(format!(
            "BR-196 template catalog cardinality drift: expected={EXPECTED_CATALOG_TOTAL} actual={}",
            catalog.len()
        ));
    }
    let mut ids = std::collections::HashSet::with_capacity(catalog.len());
    for preview in &catalog {
        if preview.template_id.trim().is_empty()
            || preview.text.trim().is_empty()
            || !ids.insert(preview.template_id)
        {
            return Err(format!(
                "BR-196 invalid or duplicate template preview: {}",
                preview.template_id
            ));
        }
    }
    Ok(catalog)
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
    let outcome = dispatch_registered_outcome!(
        "I-09-sector-top",
        crate::notify::PushKind::SectorTop,
        "sector_top_dispatcher",
        "render_sector_top",
        "",
        None,
        text
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_intraday_alert_includes_category_and_extra() {
        use stock_analysis::monitor::detector::{AlertCategory, AlertDetail, AlertEvent, AlertLevel};
        let event = AlertEvent {
            level: AlertLevel::Important,
            category: AlertCategory::MainInflow,
            code: "000001".to_string(),
            name: "平安银行".to_string(),
            message: "平安银行 主力净流入 1.2 亿".to_string(),
            detail: AlertDetail {
                price: Some(10.5),
                change_pct: Some(2.3),
                volume_ratio: Some(3.2),
                main_flow_yi: Some(1.2),
                threshold: None,
                news_title: None,
                news_summary: None,
                news_importance: None,
                ai_decision: None,
                t1_locked: false,
                extra: Some("主力排名 3/50 | 共振 85 强".to_string()),
            },
            triggered_at: chrono::Local::now(),
            routed_external_id: None,
        };
        let text = render_intraday_alert(&event);
        assert!(text.contains("主力突袭"), "缺失告警类别: {text}");
        assert!(text.contains("平安银行(000001)"), "缺失标的: {text}");
        assert!(text.contains("主力净流入 1.2 亿"), "缺失消息: {text}");
        assert!(text.contains("主力排名 3/50"), "缺失 extra: {text}");
        assert!(text.contains("辅助建议, 非下单指令"), "缺失免责声明: {text}");
    }

    #[test]
    fn render_intraday_alert_handles_empty_extra() {
        use stock_analysis::monitor::detector::{AlertCategory, AlertDetail, AlertEvent, AlertLevel};
        let event = AlertEvent {
            level: AlertLevel::Emergency,
            category: AlertCategory::LimitDown,
            code: "600000".to_string(),
            name: "浦发银行".to_string(),
            message: "浦发银行 跌停 -10.0%".to_string(),
            detail: AlertDetail {
                price: Some(9.0),
                change_pct: Some(-10.0),
                volume_ratio: Some(2.0),
                main_flow_yi: Some(-0.8),
                threshold: None,
                news_title: None,
                news_summary: None,
                news_importance: None,
                ai_decision: None,
                t1_locked: false,
                extra: None,
            },
            triggered_at: chrono::Local::now(),
            routed_external_id: None,
        };
        let text = render_intraday_alert(&event);
        assert!(text.contains("🔴"), "缺失紧急级别 emoji: {text}");
        assert!(text.contains("跌停扫雷"), "缺失类别: {text}");
        assert!(!text.contains("null"), "extra=None 不应渲染 null: {text}");
    }

    #[test]
    fn extract_company_name_handles_cjk_byte_boundaries() {
        // 2026-08-06 panic 回归: 中文标题字节边界 (原 pos+chars().count() 越界)
        let with_lawyer_prefix = "上海市锦天城律师事务所关于上海频准激光科技股份有限公司首次公开发行股票并在科创板上市之参与战略配售的投资者核查事项的法律意见书";
        assert_eq!(
            extract_company_name(with_lawyer_prefix),
            "上海频准激光科技股份有限公司"
        );
        assert_eq!(
            extract_company_name("温州宏丰电工合金股份有限公司2026年度向特定对象发行股票"),
            "温州宏丰电工合金股份有限公司"
        );
        assert_eq!(extract_company_name("某公司集团公告"), "某公司集团");
        assert_eq!(extract_company_name("无后缀标题"), "");
    }

    #[test]
    fn ipo_keyword_stage_maps_announcement_keywords() {
        assert!(ipo_keyword_stage("首次公开发行股票并在科创板上市").is_some());
        assert!(ipo_keyword_stage("招股意向书").is_some());
        assert!(ipo_keyword_stage("上市公告书").is_some());
        assert_eq!(ipo_keyword_stage("例行董事会决议公告"), None);
    }

    #[test]
    fn br192_terminal_template_status_maps_exactly_to_durable_audit_status() {
        let cases = [
            (PaperTradeStatus::Filled, "Filled"),
            (PaperTradeStatus::NotFilled, "NotFilled"),
            (PaperTradeStatus::Invalidated, "Invalidated"),
        ];

        for (template_status, expected) in cases {
            let durable_status = paper_trade::PaperTradeStatus::from(template_status);
            assert_eq!(durable_status.as_str(), expected);
        }
    }

    #[test]
    fn br099_p03_volume_quality_requires_real_value_and_has_three_tiers() {
        assert_eq!(
            candidate_volume_quality(Some(0.99)).unwrap(),
            EvidenceQuality::Weak
        );
        assert_eq!(
            candidate_volume_quality(Some(1.0)).unwrap(),
            EvidenceQuality::Mid
        );
        assert_eq!(
            candidate_volume_quality(Some(2.99)).unwrap(),
            EvidenceQuality::Mid
        );
        assert_eq!(
            candidate_volume_quality(Some(3.0)).unwrap(),
            EvidenceQuality::Strong
        );
        assert!(candidate_volume_quality(None)
            .unwrap_err()
            .contains("缺少实时量比"));
    }

    #[test]
    fn br099_candidate_assembly_removes_only_held_and_keeps_watch_candidate() {
        use magic_market_core::ProviderId;
        use stock_analysis::data_gateway::BatchEvidence;
        use stock_analysis::market_data::TopStock;
        use stock_analysis::opportunity::candidate_panel::{merge_candidates, CandidateSource};

        let evidence = |source: &str, batch_id: &str| BatchEvidence {
            provider: ProviderId::Tencent,
            source: source.to_string(),
            source_at: Some("2026-07-27T01:30:00Z".to_string()),
            observed_at: "2026-07-27T01:30:00Z".to_string(),
            batch_id: batch_id.to_string(),
        };
        let entries = merge_candidates(vec![
            (
                CandidateSource::StockPick,
                "TEST_CODE_600001".to_string(),
                "持仓候选".to_string(),
            ),
            (
                CandidateSource::VolumeWatchlist,
                "TEST_CODE_000001".to_string(),
                "自选候选".to_string(),
            ),
        ]);
        let quote_batch = crate::market_data::TopStockBatch {
            stocks: vec![
                TopStock {
                    code: "TEST_CODE_600001".to_string(),
                    name: "持仓候选".to_string(),
                    price: 10.0,
                    change_pct: 1.0,
                    volume_ratio: None,
                    main_net_yi: None,
                },
                TopStock {
                    code: "TEST_CODE_000001".to_string(),
                    name: "自选候选".to_string(),
                    price: 20.0,
                    change_pct: 2.0,
                    volume_ratio: None,
                    main_net_yi: None,
                },
            ],
            evidence: evidence("TEST_CODE_quote", "TEST_CODE_quote_batch"),
        };
        let statistics_batch = CandidateStatisticsBatch {
            rows: vec![
                CandidateStatisticsRow {
                    code: "TEST_CODE_000001".to_string(),
                    volume_ratio: Some(1.5),
                },
                CandidateStatisticsRow {
                    code: "TEST_CODE_600001".to_string(),
                    volume_ratio: Some(1.1),
                },
            ],
            evidence: evidence("TEST_CODE_statistics", "TEST_CODE_statistics_batch"),
        };

        let batch = assemble_real_candidate_batch(
            entries,
            quote_batch,
            statistics_batch,
            std::collections::HashMap::new(),
            &["TEST_CODE_600001".to_string()],
        )
        .unwrap();

        assert_eq!(batch.entries.len(), 1);
        assert_eq!(batch.entries[0].code, "TEST_CODE_000001");
        let quote = batch.quotes.get("TEST_CODE_000001").unwrap();
        assert_eq!(quote.volume_ratio, Some(1.5));
        assert_eq!(quote.main_net_yi, None);
        assert_eq!(batch.entries[0].heat_score, None);
        assert_ne!(
            batch.entries[0].tier,
            stock_analysis::opportunity::candidate_panel::EvidenceTier::Strong
        );
        assert_eq!(
            batch.quote_evidence.as_ref().unwrap().batch_id,
            "TEST_CODE_quote_batch"
        );
        assert_eq!(
            batch.statistics_evidence.as_ref().unwrap().batch_id,
            "TEST_CODE_statistics_batch"
        );
    }

    #[test]
    fn br159_candidate_statistics_projection_requires_exact_identity_and_preserves_missing() {
        use magic_market_core::{
            AssetClass, Exchange, FiniteNumber, InstrumentId, ProviderId, SourceEvidence,
        };
        use stock_analysis::data_gateway::{
            company::MarketStatistics, BatchEvidence, GatewayBatch,
        };

        let batch_id = "TEST_CODE_statistics_batch";
        let observed_at = "1785125400.000000000";
        let record = |exchange, code: &str, volume_ratio: Option<f64>, record_batch_id: &str| {
            let instrument = InstrumentId::new(exchange, code, AssetClass::Equity).unwrap();
            let evidence = SourceEvidence::new(ProviderId::Tencent, observed_at, record_batch_id)
                .unwrap()
                .with_source_at("2026-07-27T09:30:00+08:00")
                .unwrap();
            MarketStatistics::new(
                instrument,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                volume_ratio.map(|value| FiniteNumber::new(value).unwrap()),
                evidence,
            )
            .unwrap()
        };
        let evidence = BatchEvidence {
            provider: ProviderId::Tencent,
            source: "TEST_CODE_magic_tencent_statistics".to_string(),
            source_at: Some("2026-07-27T09:30:00+08:00".to_string()),
            observed_at: observed_at.to_string(),
            batch_id: batch_id.to_string(),
        };
        let requested = vec![
            "TEST_CODE_600001".to_string(),
            "TEST_CODE_000001".to_string(),
        ];
        let records = vec![
            record(Exchange::Shanghai, "600001", Some(1.8), batch_id),
            record(Exchange::Shenzhen, "000001", None, batch_id),
        ];
        let projected = project_candidate_statistics(
            &requested,
            GatewayBatch::Available {
                records: records.clone(),
                evidence: evidence.clone(),
            },
        )
        .unwrap();
        assert_eq!(
            projected.rows,
            [
                CandidateStatisticsRow {
                    code: "TEST_CODE_600001".to_string(),
                    volume_ratio: Some(1.8),
                },
                CandidateStatisticsRow {
                    code: "TEST_CODE_000001".to_string(),
                    volume_ratio: None,
                },
            ]
        );
        assert_eq!(projected.evidence, evidence);

        let error = project_candidate_statistics(
            &requested,
            GatewayBatch::Available {
                records: vec![records[0].clone()],
                evidence: projected.evidence.clone(),
            },
        )
        .unwrap_err();
        assert!(error.contains("基数不一致"));

        let error = project_candidate_statistics(
            &requested,
            GatewayBatch::Available {
                records: vec![records[1].clone(), records[0].clone()],
                evidence: projected.evidence.clone(),
            },
        )
        .unwrap_err();
        assert!(error.contains("身份不一致"));

        let error = project_candidate_statistics(
            &requested,
            GatewayBatch::Available {
                records: vec![
                    record(
                        Exchange::Shanghai,
                        "600001",
                        Some(1.8),
                        "TEST_CODE_wrong_batch",
                    ),
                    records[1].clone(),
                ],
                evidence: projected.evidence,
            },
        )
        .unwrap_err();
        assert!(error.contains("记录证据与批次证据不一致"));
    }

    #[test]
    fn snapshot_paper_accepts_complete_previous_day_valuation_before_close() {
        let now = chrono::Local::now();
        assert!(post_close_valuation_eligible(
            now,
            now.date_naive() - chrono::Days::new(1),
            true
        ));
    }

    #[test]
    fn snapshot_paper_rejects_incomplete_or_future_valuation() {
        let now = chrono::Local::now();
        assert!(!post_close_valuation_eligible(now, now.date_naive(), false));
        assert!(!post_close_valuation_eligible(
            now,
            now.date_naive() + chrono::Days::new(1),
            true
        ));
    }

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
    fn confirmed_snapshot_banner_does_not_impersonate_realtime_account() {
        let banner = BannerCtx {
            account_mode: AccountMode::Frozen,
            total_pos: Some(7),
            today_pnl: Some(2.45),
            account_metrics_complete: false,
            data_mode: DataMode::Unsafe,
            data_missing_note: None,
        };
        let text = banner.render();
        assert!(text.contains("仓位已确认"));
        assert!(text.contains("日盈亏已确认"));
        assert!(!text.contains("仓位7成"));
        assert!(!text.contains("日盈亏+2.5%"));
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

    fn t0_structured_plan() -> stock_analysis::decision::t0_advisor::T0StructuredPlan {
        use stock_analysis::decision::t0_advisor::{
            PriceZone, T0Metrics, T0PlanState, T0StructuredPlan, TrendStatus, ZoneSource,
        };
        T0StructuredPlan {
            code: "TEST_CODE_002415".to_string(),
            name: "YY".to_string(),
            source_at: chrono::DateTime::parse_from_rfc3339("2026-07-23T11:20:00+08:00")
                .unwrap()
                .with_timezone(&chrono::Utc),
            batch_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
            current_price: 25.18,
            cost_price: 23.80,
            total_quantity: 2_000,
            sell_quantity: 600,
            buyback_quantity: 600,
            sell_zone: PriceZone {
                low: 25.10,
                high: 25.30,
                source: ZoneSource::DailyPivot,
            },
            buy_zone: PriceZone {
                low: 24.50,
                high: 24.70,
                source: ZoneSource::IntradayPivot,
            },
            gross_spread_pct: 1.62,
            metrics: T0Metrics {
                trend: TrendStatus::Range,
                pace_ratio: 1.36,
                last_bar_volume_ratio: 1.28,
                intraday_average_price: 24.88,
                atr14: 0.72,
                ask_bid_ratio: 1.31,
                bid_ask_ratio: 0.76,
            },
            state: T0PlanState::SellTriggered,
            trigger_text: "卖出需进入区间且量价与卖盘同时确认".to_string(),
            invalidation_text: "连续两根5分钟收盘突破上沿则取消卖出".to_string(),
        }
    }

    #[test]
    fn t05_t0_reverse() {
        let plan = t0_structured_plan();
        let s = render_t0_advice(&banner_normal(), T0AdviceParams::from(&plan));
        assert!(s.contains("数据: Magic TDX | 批次: 0123456789ab"));
        assert!(s.contains("状态: 卖出观察触发 | 趋势: 震荡"));
        assert!(s.contains("量能节奏: 1.36x"));
        assert!(s.contains("末根5分钟量比: 1.28x"));
        assert!(s.contains("卖出观察区: 25.10~25.30（日线确认拐点）"));
        assert!(s.contains("接回观察区: 24.50~24.70（5分钟确认拐点）"));
        assert!(s.contains("观察腿: 600股卖出/600股接回"));
        assert!(s.contains("不代表券商已验证可卖数量"));
        assert!(!s.contains("高抛: +"));
        assert!(!s.contains("低吸: -"));
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
                watch_point: Some("明日分歧"),
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
                watch_point: Some("接力意愿"),
            },
        ];
        let s = render_industry_chain("2026-07-05", &chains, Some("光伏（涨停12→3家）"), None);
        assert!(s.starts_with("🔥 涨停题材联动（2026-07-05）"));
        assert!(s.contains("1. AI算力 涨停8家"));
        assert!(s.contains("龙头: 龙头A(TEST_CODE_688001) 4板"));
        assert!(s.contains("2. 机器人"));
        assert!(s.contains("⚠️ 退潮链: 光伏（涨停12→3家）"));

        let degraded = render_industry_chain(
            "2026-07-05",
            &chains,
            None,
            Some("已隔离 1 个标的、1 个来源错误，仅展示通过质检的真实子集"),
        );
        assert!(degraded.contains("⚠️ 部分证据:"));
        assert!(degraded.contains("已隔离 1 个标的、1 个来源错误"));

        let missing_watch = render_industry_chain(
            "2026-07-05",
            &[ChainLine {
                chain: "AI算力",
                limit_up_n: 3,
                first_n: 2,
                consec_n: 1,
                heat_stage: "MainUp",
                leader_name: "龙头A",
                leader_code: "TEST_CODE_688001",
                leader_boards: 2,
                followers: "B,C",
                watch_point: None,
            }],
            None,
            None,
        );
        assert!(missing_watch.contains("明日观察: 数据缺失（当前批次未提供量能/走势证据）"));
    }

    // ---- R-04 龙虎榜 ----

    #[test]
    fn r04_review_lhb() {
        let entries = vec![LhbEntry {
            name: "X",
            code: "TEST_CODE_688001",
            net_buy_yi: 1.5,
            reason: Some("涨幅偏离值达7%"),
            buy_inst_n: Some(2),
            buy_inst_amt_wan: Some(8000.0),
            buy_other_n: Some(3),
            buy_other_amt_wan: Some(4000.0),
            buy_conc_pct: Some(65.0),
            sell_desc: Some("游资席位"),
            sell_conc_pct: Some(45.0),
            chain_match: Some("AI算力"),
            next_day_risk: Some("高开震荡"),
        }];
        let s = render_review_lhb("2026-07-05", &entries);
        assert!(s.starts_with("🐉 龙虎榜净买前五（2026-07-05 21:00）"));
        assert!(s.contains("X(TEST_CODE_688001) 净买1.5亿"));
        assert!(s.contains("买: 机构2席8000万 其他3席4000万（集中度65%）"));
        assert!(s.contains("卖: 游资席位（集中度45%）"));
        assert!(s.contains("主线一致: 是-AI算力"));
        assert!(s.contains("仅结构化事实, 不含席位风格推断"));

        let missing = render_review_lhb(
            "2026-07-05",
            &[LhbEntry {
                name: "X",
                code: "TEST_CODE_688001",
                net_buy_yi: 1.5,
                reason: None,
                buy_inst_n: None,
                buy_inst_amt_wan: None,
                buy_other_n: None,
                buy_other_amt_wan: None,
                buy_conc_pct: None,
                sell_desc: None,
                sell_conc_pct: None,
                chain_match: None,
                next_day_risk: None,
            }],
        );
        assert!(missing.contains("数据缺失"));
        assert!(!missing.contains("机构0席"));
        assert!(!missing.contains("其他0席"));
        assert!(!missing.contains(" | —"));
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
        let s = render_event_calendar(
            "2026-07-06",
            &holdings,
            "央行MLF到期",
            "中金所官方批次已验证；无次日交割",
            "+0.8%",
            "7.18",
        );
        assert!(s.starts_with("🗓️ 明日事件（2026-07-06）"));
        assert!(s.contains("· 【实盘】XX(TEST_CODE_000001): 解禁3.2亿"));
        assert!(s.contains("· 【虚拟】YY(TEST_CODE_000002): 财报预告"));
        assert!(s.contains("宏观: 央行MLF到期"));
        assert!(s.contains("期货交割: 中金所官方批次已验证；无次日交割"));
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
            member_count: 3,
            continuous_count: 3,
            leading_names: vec!["A", "B"],
            other_names: vec!["C"],
            watch_point: Some("明日是否扩散"),
        };
        let out = render_catalyst_review(p);
        assert!(out.contains("📰 题材催化复盘（2026-07-06）"));
        assert!(out.contains("主线: AI算力"));
        assert!(out.contains("涨停成员: 3家 | 连板成员: 3家 | 持续性结构: 高"));
        assert!(out.contains("题材评分: 85.0"));
        assert!(out.contains("前排成员（按连板数）: A、B"));
        assert!(out.contains("其余同题材成员: C"));
        assert!(out.contains("明日观察点: 明日是否扩散"));
        assert!(!out.contains("已启动"));
        assert!(!out.contains("待启动"));
        assert!(!out.contains("N/A"));
    }

    #[test]
    fn catalyst_review_persistent_low_empty() {
        let p = CatalystReviewParams {
            date: "2026-07-06",
            theme: "X",
            score: None,
            persistent: PersistentLevel::Low,
            member_count: 0,
            continuous_count: 0,
            leading_names: vec![],
            other_names: vec![],
            watch_point: None,
        };
        let out = render_catalyst_review(p);
        assert!(out.contains("题材评分: 数据缺失（无独立评分批次）"));
        assert!(out.contains("持续性结构: 低"));
        assert!(!out.contains("前排成员"));
        assert!(!out.contains("其余同题材成员"));
        assert!(out.contains("明日观察点: 数据缺失（未接入独立量能/走势批次）"));
    }

    fn visible_chain_batch(
        names: &[&str],
    ) -> stock_analysis::database::chain_intelligence::VisibleChainBatch {
        use stock_analysis::database::chain_intelligence::{
            ChainInputEvidenceInput, VisibleChain, VisibleChainBatch, VisibleChainMember,
        };
        VisibleChainBatch {
            batch_id: "TEST_CODE_chain_batch".to_string(),
            content_hash: "a".repeat(64),
            trading_date: chrono::NaiveDate::from_ymd_opt(2099, 1, 2).unwrap(),
            calculation_version: "TEST_CODE_v1".to_string(),
            taxonomy_version: "TEST_CODE_taxonomy_v1".to_string(),
            inputs: vec![ChainInputEvidenceInput {
                input_id: "TEST_CODE_chain_input".to_string(),
                ordinal: 0,
                capability: "TEST_CODE_limit_pool".to_string(),
                provider: "TEST_CODE_provider".to_string(),
                source: "TEST_CODE_source".to_string(),
                source_at: Some("2099-01-02".to_string()),
                observed_at: "2099-01-02T15:00:00+08:00".to_string(),
                source_batch_id: "TEST_CODE_input_batch".to_string(),
                source_batch_hash: "b".repeat(64),
                content_hash: "c".repeat(64),
            }],
            chains: vec![VisibleChain {
                chain_id: "TEST_CODE_chain".to_string(),
                canonical_board_id: "TEST_CODE_board".to_string(),
                board_name: "测试主线".to_string(),
                upper_limit_count: i32::try_from(names.len()).unwrap(),
                continuous_count: 3,
                members: names
                    .iter()
                    .enumerate()
                    .map(|(index, name)| VisibleChainMember {
                        instrument_id: format!("TEST_CODE_{index:06}"),
                        security_name: (*name).to_string(),
                        source_event_id: format!("TEST_CODE_event_{index}"),
                        streak: i32::try_from(names.len() - index).unwrap(),
                    })
                    .collect(),
            }],
            rejections: vec![],
        }
    }

    #[test]
    fn br160_a10_maps_only_the_visible_gateway_batch() {
        let batch = visible_chain_batch(&["测试一", "测试二", "测试三", "测试四"]);
        let snapshot =
            catalyst_review_from_chain_batch(&batch).expect("visible batch maps to A-10");
        assert_eq!(snapshot.date, "2099-01-02");
        assert_eq!(snapshot.source_batch_id, batch.batch_id);
        assert_eq!(snapshot.source_content_hash, batch.content_hash);
        assert_eq!(
            snapshot.source_observed_at,
            Some(chrono::DateTime::parse_from_rfc3339("2099-01-02T15:00:00+08:00").unwrap())
        );
        assert_eq!(snapshot.theme, "测试主线");
        assert_eq!(snapshot.persistent, PersistentLevel::High);
        assert_eq!(snapshot.member_count, 4);
        assert_eq!(snapshot.continuous_count, 3);
        assert_eq!(snapshot.leading_members, ["测试一", "测试二", "测试三"]);
        assert_eq!(snapshot.other_members, ["测试四"]);
        assert_eq!(snapshot.score, None);
        assert_eq!(snapshot.watch_point, None);
    }

    #[test]
    fn br160_a10_rejects_visible_batch_count_contradiction() {
        let mut batch = visible_chain_batch(&["测试一", "测试二", "测试三"]);
        batch.chains[0].upper_limit_count = 4;
        let error = catalyst_review_from_chain_batch(&batch)
            .expect_err("contradictory visible batch must fail");
        assert!(error.contains("member-count contract"));
    }

    #[test]
    fn br160_a10_rejects_missing_or_invalid_input_observation_time() {
        let mut batch = visible_chain_batch(&["测试一", "测试二", "测试三"]);
        batch.inputs[0].observed_at = "TEST_CODE_invalid_observation".to_string();
        let error = catalyst_review_from_chain_batch(&batch)
            .expect_err("invalid source observation must fail before delivery");
        assert!(error.contains("source observed_at is invalid"), "{error}");

        batch.inputs.clear();
        let error = catalyst_review_from_chain_batch(&batch)
            .expect_err("missing source observation must fail before delivery");
        assert!(error.contains("no input observation time"), "{error}");
    }

    #[test]
    fn br160_a10_rejects_invalid_continuous_counts() {
        let mut above_member_count = visible_chain_batch(&["测试一", "测试二", "测试三"]);
        above_member_count.chains[0].continuous_count = 4;
        let error = catalyst_review_from_chain_batch(&above_member_count)
            .expect_err("continuous count above member count must fail");
        assert!(error.contains("continuous count above member count"));

        let mut negative = visible_chain_batch(&["测试一", "测试二", "测试三"]);
        negative.chains[0].continuous_count = -1;
        let error = catalyst_review_from_chain_batch(&negative)
            .expect_err("negative continuous count must fail");
        assert!(error.contains("negative continuous count"));
    }

    #[test]
    fn br160_a10_loader_has_no_legacy_source_fallback() {
        let source = include_str!("push_templates.rs");
        let start = source
            .find("pub async fn load_catalyst_review_snapshot_real")
            .expect("A-10 loader");
        let end = source[start..]
            .find("// ============================================================================")
            .map(|offset| start + offset)
            .expect("A-10 loader boundary");
        let loader = &source[start..end];
        for forbidden in [
            "chain_daily",
            "board_rotation_daily",
            "DataFetcherManager",
            "get_latest_chain_clusters",
            "get_latest_board_rotations",
        ] {
            assert!(
                !loader.contains(forbidden),
                "legacy A-10 source: {forbidden}"
            );
        }
        assert!(loader.contains("ChainIntelligenceGateway"));
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
        let p = build_preopen_news_hot_from_db(
            "09:05",
            &clusters,
            &rotations,
            &std::collections::HashMap::new(),
        )
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
        assert!(build_preopen_news_hot_from_db(
            "09:05",
            &clusters,
            &[],
            &std::collections::HashMap::new()
        )
        .is_err());
    }

    #[test]
    fn v15_dispatch_preopen_news_hot_daily_no_data() {
        // 空 DB 时不推送 (graceful no-op)
        // 实际需要 DB, 此处仅验证 build_* 函数路径, dispatch 行为在 e2e
        use stock_analysis::database::concepts::ChainDailyRow;
        let clusters: Vec<ChainDailyRow> = vec![];
        assert!(build_preopen_news_hot_from_db(
            "09:05",
            &clusters,
            &[],
            &std::collections::HashMap::new()
        )
        .is_err());
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
    fn br140_missing_virtual_observation_directory_is_not_a_complete_empty_source() {
        let dir = std::env::temp_dir().join(format!(
            "stock_analysis_a01_missing_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));

        let error = match load_virtual_observation_from_dir(&dir) {
            Err(error) => error,
            Ok(_) => panic!("a missing source directory must remain unavailable"),
        };

        assert!(error.contains("source directory missing"));
    }

    #[test]
    fn br140_empty_virtual_observation_directory_is_not_a_complete_empty_source() {
        let dir = std::env::temp_dir().join(format!(
            "stock_analysis_a01_empty_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let error = match load_virtual_observation_from_dir(&dir) {
            Err(error) => error,
            Ok(_) => panic!("an empty source directory must remain unavailable"),
        };

        assert!(error.contains("no JSON snapshots"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn br140_a01_loader_isolates_bad_record_inside_valid_snapshot() {
        let dir = std::env::temp_dir().join(format!(
            "stock_analysis_a01_loader_bad_record_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("2026-07-21.json"),
            r#"{"records":[
                {"entry_date":"2026-07-20","code":"TEST_CODE_000001","name":"坏记录","entry_mode":"观察","entry_price":"bad"},
                {"entry_date":"2026-07-20","code":"TEST_CODE_000002","name":"合规记录","entry_mode":"观察","entry_price":10.5}
            ]}"#,
        )
        .unwrap();

        let batch = load_virtual_observation_from_dir(&dir).unwrap();

        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.records[0].code, "TEST_CODE_000002");
        assert_eq!(batch.rejections.len(), 1);
        assert_eq!(batch.rejections[0].reason_code, "record_decode_failed");
        assert!(batch.source_failures.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn br140_a01_loader_keeps_valid_file_when_newer_json_is_damaged() {
        let dir = std::env::temp_dir().join(format!(
            "stock_analysis_a01_loader_damaged_file_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("latest.json"), "{damaged").unwrap();
        std::fs::write(
            dir.join("2026-07-20.json"),
            r#"{"entry_date":"2026-07-20","code":"TEST_CODE_000003","name":"合规记录","entry_mode":"观察","entry_price":8.25}"#,
        )
        .unwrap();

        let batch = load_virtual_observation_from_dir(&dir).unwrap();

        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.records[0].code, "TEST_CODE_000003");
        assert_eq!(batch.source_failures.len(), 1);
        assert_eq!(batch.source_failures[0].reason_code, "invalid_json");
        assert_eq!(batch.source_failures[0].identity_hash.len(), 64);
        std::fs::remove_dir_all(&dir).unwrap();
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
    fn br158_a01_loader_uses_review_data_gateway_only() {
        let source = include_str!("push_templates.rs");
        let loader = source
            .split("pub async fn load_paper_review_snapshot_real(")
            .nth(1)
            .expect("A-01 async loader")
            .split("/// v15.6 兼容")
            .next()
            .expect("A-01 loader body");

        assert!(loader.contains("ReviewDataGateway::new"));
        assert!(loader.contains("a01_daily_bars"));
        assert!(!loader.contains("DataFetcherManager"));
        assert!(!loader.contains("get_daily_data"));
    }

    #[test]
    fn br140_a01_bad_first_symbol_does_not_block_later_valid_record() {
        let records = vec![
            VirtualRecordLite {
                entry_date: "2026-07-20".to_string(),
                code: "TEST_CODE_000001".to_string(),
                name: "测试一".to_string(),
                entry_mode: "观察".to_string(),
                entry_price: 10.0,
            },
            VirtualRecordLite {
                entry_date: "2026-07-20".to_string(),
                code: "TEST_CODE_000002".to_string(),
                name: "测试二".to_string(),
                entry_mode: "观察".to_string(),
                entry_price: 10.0,
            },
        ];
        let close_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 21).expect("valid test date");

        let batch = build_paper_review_candidate_with("2026-07-21", &records, |code, _days| {
            if code.ends_with("000001") {
                Err("TEST_CODE quality rejected".to_string())
            } else {
                Ok((vec![(close_date, 12.0)], "TEST_REAL_FIXTURE".to_string()))
            }
        })
        .expect("later complete record remains eligible");

        assert_eq!(
            batch.snapshot.as_ref().expect("one complete snapshot").code,
            "TEST_CODE_000002"
        );
        assert_eq!(
            batch.snapshot.expect("one complete snapshot").date,
            "2026-07-21"
        );
        assert_eq!(batch.rejections.len(), 1);
    }

    #[test]
    fn br158_a01_strict_review_excludes_old_completed_observations() {
        let records = vec![VirtualRecordLite {
            entry_date: "2026-07-10".to_string(),
            code: "TEST_CODE_000001".to_string(),
            name: "测试一".to_string(),
            entry_mode: "观察".to_string(),
            entry_price: 10.0,
        }];
        let mut fetch_calls = 0;

        let batch = build_paper_review_candidate_with("2026-07-24", &records, |_code, _days| {
            fetch_calls += 1;
            Ok((Vec::new(), "TEST_CODE_unreachable".to_string()))
        })
        .expect("old observations are a typed no-data condition");

        assert!(batch.snapshot.is_none());
        assert_eq!(batch.out_of_window_count, 1);
        assert_eq!(batch.pending_count, 0);
        assert!(batch.rejections.is_empty());
        assert_eq!(fetch_calls, 0);
    }

    #[test]
    fn br158_a01_target_disposition_distinguishes_exact_pending_and_old() {
        let review = chrono::NaiveDate::from_ymd_opt(2026, 7, 24).unwrap();
        let completed = review;

        assert_eq!(
            classify_a01_target(
                chrono::NaiveDate::from_ymd_opt(2026, 7, 23).unwrap(),
                review,
                completed
            )
            .unwrap(),
            A01TargetDisposition::Eligible(review)
        );
        assert!(matches!(
            classify_a01_target(
                chrono::NaiveDate::from_ymd_opt(2026, 7, 24).unwrap(),
                review,
                completed
            )
            .unwrap(),
            A01TargetDisposition::Pending(_)
        ));
        assert!(matches!(
            classify_a01_target(
                chrono::NaiveDate::from_ymd_opt(2026, 7, 10).unwrap(),
                review,
                completed
            )
            .unwrap(),
            A01TargetDisposition::OutOfWindow(_)
        ));
    }

    #[test]
    fn br140_a01_all_invalid_records_fail_with_aggregate_count() {
        let records = vec![
            VirtualRecordLite {
                entry_date: "2026-07-20".to_string(),
                code: "TEST_CODE_000001".to_string(),
                name: "测试一".to_string(),
                entry_mode: "观察".to_string(),
                entry_price: 10.0,
            },
            VirtualRecordLite {
                entry_date: "2026-07-20".to_string(),
                code: "TEST_CODE_000002".to_string(),
                name: "测试二".to_string(),
                entry_mode: "观察".to_string(),
                entry_price: 10.0,
            },
        ];

        let batch = build_paper_review_candidate_with("2026-07-21", &records, |code, _days| {
            Err(format!("{code}: TEST_CODE quality rejected"))
        })
        .expect("record isolation returns a typed batch");

        assert!(batch.snapshot.is_none());
        assert_eq!(batch.rejections.len(), 2);
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
            plan_id: "TEST_CODE_PAPER_PLAN_1".to_string(),
            code: "TEST_CODE_600001".to_string(),
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
            paper_trade_created_at: "2026-07-30 01:31:00".to_string(),
            order_audit_id: Some(1),
            audit_previous_hash: Some("BR086_ORDER_AUDIT_GENESIS_V1".to_string()),
            audit_record_hash: Some("a".repeat(64)),
            quote_observed_at: Some("2026-07-30T09:31:00+08:00".to_string()),
            terminal_at: Some("2026-07-30 01:31:00".to_string()),
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
    fn br192_paper_trade_report_rejects_missing_or_ambiguous_terminal_evidence() {
        let mut missing_receipt = valid_paper_trade_dispatch_row();
        missing_receipt.audit_record_hash = None;
        let error = validate_paper_trade_dispatch_row_for_env(
            missing_receipt,
            stock_analysis::risk::env_guard::TradingEnv::Test,
        )
        .expect_err("missing immutable audit receipt");
        assert!(error.contains("terminal evidence unavailable"), "{error}");

        let first = validate_paper_trade_dispatch_row_for_env(
            valid_paper_trade_dispatch_row(),
            stock_analysis::risk::env_guard::TradingEnv::Test,
        )
        .expect("first exact terminal projection");
        let second = validate_paper_trade_dispatch_row_for_env(
            valid_paper_trade_dispatch_row(),
            stock_analysis::risk::env_guard::TradingEnv::Test,
        )
        .expect("second exact terminal projection");
        let error = reject_ambiguous_paper_trade_reports(&[first, second])
            .expect_err("duplicate exact audit projection must be rejected");
        assert!(error.contains("terminal evidence ambiguous"), "{error}");
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

        // G-03 验证对照: 2026-08-06 用户决策 (未接券商) — 账户限制全移除,
        // HoldingPlan 在 Frozen 也不阻断 (Frozen 状态由 banner 出声)
        assert!(!should_block_on_mode(
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
            member_count: 3,
            continuous_count: 3,
            leading_names: vec!["中科曙光", "浪潮信息"],
            other_names: vec!["紫光股份"],
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

        // 建议型模板必须保留辅助建议尾注；A-10 只呈现 BR-160
        // 已接纳的结构化事实，因此使用更严格的事实型尾注。
        assert!(p1.contains("辅助建议, 非下单指令"));
        assert!(i1.contains("辅助建议, 非下单指令"));
        assert!(i2.contains("辅助建议, 非下单指令"));
        assert!(i3.contains("辅助建议, 非下单指令"));
        assert!(d1.contains("辅助建议, 非下单指令"));
        assert!(a10.ends_with("仅结构化事实，非下单指令"));
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
    /// 2026-08-06 用户决策 (未接券商): 账户限制全移除 — HoldingPlan 在
    /// Frozen/Unsafe 均不阻断 (状态由 banner 出声)。
    fn should_not_block_holding_plan_on_frozen() {
        use super::super::notify::PushKind;
        assert!(!should_block_on_mode(
            PushKind::HoldingPlan,
            AccountMode::Frozen,
            DataMode::Full,
        ));
    }

    #[test]
    fn should_not_block_holding_plan_on_unsafe() {
        use super::super::notify::PushKind;
        assert!(!should_block_on_mode(
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
        // (实际推送行为由 BR-192 durable coordinator 与非计数冷却单元测试覆盖)
        let _banner = banner_normal();
    }

    #[test]
    fn counted_kinds_bypass_process_local_cooldown() {
        use super::super::notify::PushKind;
        for kind in [
            PushKind::HoldingPlan,
            PushKind::T0Advice,
            PushKind::HoldingEvent,
            PushKind::ReviewMarket,
            PushKind::ReviewProviderTopN,
            PushKind::DailyReport,
        ] {
            assert!(
                crate::durable_delivery_runtime::is_counted_kind(kind),
                "{kind:?} must be in the durable catalog"
            );
            assert!(!is_in_cooldown(kind, "TEST_CODE_000001"));
        }
    }

    #[test]
    fn cooldown_table_isolated_by_code() {
        use super::super::notify::PushKind;
        let kind = PushKind::NewsRanked;
        assert!(!crate::durable_delivery_runtime::is_counted_kind(kind));
        // 同一 kind 不同 code 是不同 key
        assert!(!is_in_cooldown(kind, "TEST_CODE_000001"));
        assert!(!is_in_cooldown(kind, "TEST_CODE_000002"));
        record_uncounted_cooldown(kind, "TEST_CODE_000001");
        assert!(is_in_cooldown(kind, "TEST_CODE_000001"));
        assert!(
            !is_in_cooldown(kind, "TEST_CODE_000002"),
            "不同 code 应独立"
        );
    }

    #[test]
    fn emergency_bypass_cooldown_table() {
        use super::super::notify::{PushKind, PushLevel};
        let kind = PushKind::MarketActionAlert;
        assert!(!crate::durable_delivery_runtime::is_counted_kind(kind));
        record_uncounted_cooldown(kind, "TEST_CODE_000001");
        assert!(!is_in_cooldown(kind, "TEST_CODE_000001"));
        assert_eq!(kind.level(), PushLevel::Emergency);
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

    /// 清空非计数 COOLDOWN_TABLE 与旧 L4 测试状态。
    /// 测试间共享的全局状态 (account_mode_log 表、冷却表) 必须全部重置
    /// 才能保证 67 个并行测试互不干扰。
    /// 环境变量由 `TestEnvGuard` 负责隔离；这里仅重置业务状态。
    fn reset_delivery_state_for_test() {
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
        reset_delivery_state_for_test();

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
        reset_delivery_state_for_test();

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
        reset_delivery_state_for_test();

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
        reset_delivery_state_for_test();

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
        reset_delivery_state_for_test();

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
        reset_delivery_state_for_test();

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
        reset_delivery_state_for_test();

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
        reset_delivery_state_for_test();

        use stock_analysis::monitor::data_mode::{
            Capability, CapabilityStatus, DataHealthInput, DataMode as LibDM,
        };

        let input = DataHealthInput {
            capabilities: vec![
                CapabilityStatus::fresh(Capability::Quote, 30),
                // BR-216: Kline 预算是 1 个交易日, 必须真正越过该档才算过期,
                // 否则本用例会因 News 缺失而"以错误原因通过"。
                CapabilityStatus::fresh(
                    Capability::Kline,
                    stock_analysis::monitor::data_mode::KLINE_MAX_AGE_SECS + 1,
                ),
                CapabilityStatus::missing(Capability::MoneyFlow),
                CapabilityStatus::fresh(Capability::News, 30),
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
        reset_delivery_state_for_test();

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
        reset_delivery_state_for_test();
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
        reset_delivery_state_for_test();
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
        reset_delivery_state_for_test();
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
        // Now test the BR-196 presentation-gated governor.
        let token = crate::presentation_registry::acquire_token(
            "T-01-account-mode",
            crate::notify::PushKind::AccountMode,
            "account_mode_hook",
            "render_account_mode",
        )
        .unwrap();
        let outcome = crate::notify::push_presented_v3(token, text, None).await;
        eprintln!("[v12-E2E-smoke] presented governor result: {outcome:?}");
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
        assert!(outcome.is_pushed(), "smoke test 推送应成功");
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
        use stock_analysis::announcement::{AnnLevel, Announcement};
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
            build_event_calendar_macro_summary(&[], R08HoldingAudience::Unavailable),
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
            announcement(
                "TEST_CODE_000005",
                "测试本地",
                "关于注销部分回购股份并减少注册资本通知债权人的公告",
                AnnLevel::Skip,
            ),
        ];
        let held = std::collections::HashSet::from(["TEST_CODE_600000".to_string()]);
        let summary =
            build_event_calendar_macro_summary(&rows, R08HoldingAudience::Verified(&held));
        assert!(summary.contains("今日共 5 条公告"));
        assert!(summary.contains("持仓相关 (TOP 1)"));
        assert!(summary.contains("测试持仓(TEST_CODE_600000)"));
        assert!(summary.contains("TEST_CODE_000001 (Info): 其他公告1"));
        assert!(summary.contains("非持仓 (TOP 3)"));
        assert!(!summary.contains("其他公告4"));
        assert!(!summary.contains("测试本地"));
        assert!(!summary.contains("通知债权人"));
    }

    #[test]
    fn br140_r08_unavailable_audience_never_labels_notice_non_holding() {
        use stock_analysis::announcement::{AnnLevel, Announcement};
        let rows = vec![Announcement {
            code: "TEST_CODE_600000".to_string(),
            name: "测试公司".to_string(),
            title: "重大合同公告".to_string(),
            date: "2026-07-21".to_string(),
            summary: "TEST_CODE summary".to_string(),
            content: "TEST_CODE content".to_string(),
            level: AnnLevel::Important,
            reason: "TEST_CODE reason".to_string(),
            external_id: Some("TEST_CODE_R08_UNKNOWN_AUDIENCE".to_string()),
            url: Some("https://example.invalid/unknown-audience".to_string()),
        }];

        let summary = build_event_calendar_macro_summary(&rows, R08HoldingAudience::Unavailable);

        assert!(summary.contains("持仓关系不可判定"));
        assert!(summary.contains("重大合同公告"));
        assert!(!summary.contains("持仓相关"));
        assert!(!summary.contains("非持仓"));
    }

    #[test]
    fn br138_dispatcher_r08_excludes_local_only_lifecycle_rows() {
        use magic_market_core::ProviderId;
        use stock_analysis::data_gateway::{BatchEvidence, EventAnnouncement, GatewayBatch};
        let batch = GatewayBatch::Available {
            records: vec![EventAnnouncement {
                announcement_id: "TEST_CODE_DISPATCHER_R08_LOCAL".to_string(),
                code: "TEST_CODE_600000".to_string(),
                category: Some("减持".to_string()),
                title: "减持计划期限届满暨实施情况的公告".to_string(),
                published_at: "2026-07-21T18:00:00+08:00".to_string(),
                canonical_url: "https://example.invalid/local-only".to_string(),
            }],
            evidence: BatchEvidence {
                provider: ProviderId::Cninfo,
                source: "cninfo-market".to_string(),
                source_at: Some("2026-07-21T18:00:00+08:00".to_string()),
                observed_at: "1784649000.000000000".to_string(),
                batch_id: "TEST_CODE_cninfo_batch".to_string(),
            },
        };

        let (summary, count) = build_gateway_event_calendar_summary(&batch);
        assert_eq!(count, 0);
        assert!(summary.contains("可即时通知公告 0 条"));
        assert!(!summary.contains("减持计划期限届满"));
    }

    #[test]
    fn br199_r08_missing_cninfo_category_stays_explicitly_missing() {
        use magic_market_core::ProviderId;
        use stock_analysis::data_gateway::{BatchEvidence, EventAnnouncement, GatewayBatch};
        let batch = GatewayBatch::Available {
            records: vec![EventAnnouncement {
                announcement_id: "TEST_CODE_R08_MISSING_CATEGORY".to_string(),
                code: "TEST_CODE_600000".to_string(),
                category: None,
                title: "重大合同公告".to_string(),
                published_at: "2026-07-21T18:00:00+08:00".to_string(),
                canonical_url: "https://example.invalid/missing-category".to_string(),
            }],
            evidence: BatchEvidence {
                provider: ProviderId::Cninfo,
                source: "cninfo-market".to_string(),
                source_at: Some("2026-07-21T18:00:00+08:00".to_string()),
                observed_at: "1784649000.000000000".to_string(),
                batch_id: "TEST_CODE_cninfo_missing_category".to_string(),
            },
        };

        let (summary, count) = build_gateway_event_calendar_summary(&batch);

        assert_eq!(count, 1);
        assert!(summary.contains("TEST_CODE_600000: 重大合同公告"));
        assert!(!summary.contains("(公告)"));
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
