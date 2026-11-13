//! 情绪票竞价买价（批 1.3，2026-10-23）+ 09:25 板块热度（批 2.1）。
//!
//! ## 口径（Q2 = 两者都给；Q4 = 上浮 0.5%）
//!
//! 情绪型票不走 ma 回踩 —— 回测（2026-10-22）显示涨停次日盘中触及 ma10(±1%) 只有 15.4%，
//! 给回踩价等于给一个统计上不会成交的价。改为**竞价价直给 + 分档**：
//!
//! | 竞价涨幅 | 给什么价 | 标签 |
//! |---|---|---|
//! | > +6% | 不给价 | 高开过多不追（该子集均值 -1.97%、胜率 28.4%，n=338） |
//! | +2% ~ +6% | 竞价价 | 轻仓 |
//! | -2% ~ +2% | 竞价价 × 1.005 | 保成交（Q4 默认 0.5%） |
//! | < -2% | 竞价价 | 低开博弈区（不设低开放弃线：低开 >2% 是唯一正期望分组） |
//!
//! 文案必须带「竞价买入当日 T+1 不可卖」—— 这不是免责声明，是交易规则。
//!
//! ## 边界
//!
//! - 只有 `regime=emotion` 的票进这条路径；趋势型仍走 `buy_zone` 回踩区间；
//! - 竞价价缺失（停牌/未撮合）的票不出现，不填 0（BR-222）；
//! - 09:25 前 `auction_volume_ratio` 只有首段有值，所以量比只作为参考列，不做筛选。

use std::collections::HashMap;

use diesel::prelude::*;
use diesel::sql_types::{Double, Nullable, Text};

use crate::database::{ticket_lifecycle, DatabaseManager};
use crate::monitor::auction_watch::AuctionRow;
use crate::monitor::ticket_card;
use crate::monitor::ticket_regime::{self, Regime};

/// Q4：保成交上浮比例（operator 可改）。
pub const ENSURE_FILL_MARKUP: f64 = 0.005;
/// 高开帽：竞价涨幅超过它不给买价。
pub const MAX_AUCTION_PCT: f64 = 6.0;
/// 板块热度取前几名。
const BOARD_HEAT_TOP: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputKind {
    /// WP5 目标语义：只输出参考价，不声称可执行/可成交。
    #[default]
    ReferencePrice,
    /// 旧文案；仅用于 shadow 期保持推送不变。
    LegacyAction,
}

/// 6.10：正文必须区分「持仓加仓参考 / 新买入参考 / 持仓未知」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PositionContext {
    /// 有底仓。
    Held,
    /// 确认无持仓。
    #[default]
    NoPosition,
    /// 持仓读取失败：不能伪装成「无持仓」。
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// 高开过多，不给价
    SkipHigh,
    /// +2~+6%，轻仓
    Light,
    /// -2~+2%，上浮保成交
    EnsureFill,
    /// <-2%，低开博弈区
    Bargain,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Tier::SkipHigh => "高开过多不追",
            Tier::Light => "轻仓",
            Tier::EnsureFill => "保成交",
            Tier::Bargain => "低开博弈区",
        }
    }

    /// WP5 的参考语义：保留分档信息，但不把档位写成下单动作。
    pub fn reference_label(self) -> &'static str {
        match self {
            Tier::SkipHigh => "高开过多不追",
            Tier::Light => "轻仓参考",
            Tier::EnsureFill => "中间区间参考",
            Tier::Bargain => "低开参考",
        }
    }

    /// 证据包里的稳定键（不受文案改动影响）。
    pub fn key(self) -> &'static str {
        match self {
            Tier::SkipHigh => "skip_high",
            Tier::Light => "light",
            Tier::EnsureFill => "ensure_fill",
            Tier::Bargain => "bargain",
        }
    }
}

pub fn tier_for(auction_pct: f64) -> Tier {
    if auction_pct > MAX_AUCTION_PCT {
        Tier::SkipHigh
    } else if auction_pct >= 2.0 {
        Tier::Light
    } else if auction_pct >= -2.0 {
        Tier::EnsureFill
    } else {
        Tier::Bargain
    }
}

/// 给什么价：`None` = 不给价。
pub fn quote_price(auction_price: f64, tier: Tier) -> Option<f64> {
    if auction_price <= 0.0 {
        return None;
    }
    match tier {
        Tier::SkipHigh => None,
        Tier::Light | Tier::Bargain => Some(auction_price),
        Tier::EnsureFill => Some(auction_price * (1.0 + ENSURE_FILL_MARKUP)),
    }
}

#[derive(Debug, Clone)]
pub struct PriceLine {
    pub code: String,
    pub name: String,
    pub auction_price: f64,
    pub auction_pct: f64,
    /// 给出来的价（含上浮），`None` = 不给
    pub price: Option<f64>,
    pub tier: Tier,
    /// 默认只输出参考价；旧文案要显式降级。
    pub output_kind: OutputKind,
    /// 涨停池/市场阶段快照缺失时置 true：正文写「高位状态未核实」，
    /// 不能把未知当成「不属于高位」（6.1）。
    pub high_state_unverified: bool,
    /// 持仓上下文（6.4）：有底仓写「已有 N 股/成本/浮盈亏」，
    /// 无持仓写「持仓：无」，读取失败写「读取失败」，不能省略。
    pub position_note: String,
    /// 6.10：持仓状态决定参考价前缀，读取失败不能当无持仓。
    pub position: PositionContext,
    /// 6.10：同日风险提示（risk_flag，最近 5 日）；有则正文必须披露，
    /// 与给出的加仓参考同时存在时标「冲突」。
    pub risk_note: Option<String>,
    /// 竞价量比（首段）；缺失写 N/A，不补 0。
    pub auction_volume_ratio: Option<f64>,
    /// 日线指标摘要（截至上一交易日收盘）；盘中不冒充分钟级。
    pub tech_note: Option<String>,
    /// 最新提案区间与竞价位置；终态提案只作复盘，不再当新参考。
    pub plan_note: Option<String>,
}

impl PriceLine {
    pub fn render(&self) -> String {
        let kind = if crate::monitor::output_contract::reference_language_enabled() {
            self.output_kind
        } else {
            OutputKind::LegacyAction
        };
        self.render_as(kind)
    }

    pub fn render_as(&self, kind: OutputKind) -> String {
        let note = if self.high_state_unverified {
            "｜高位状态未核实"
        } else {
            ""
        };
        let prefix = match (kind, self.price) {
            (OutputKind::ReferencePrice, Some(_)) => match self.position {
                PositionContext::Held => "持仓加仓参考｜",
                PositionContext::NoPosition => "新买入参考｜",
                PositionContext::Unknown => "持仓未知｜",
            },
            _ => "",
        };
        let base = match (kind, self.price) {
            (OutputKind::ReferencePrice, Some(price)) => format!(
                "{prefix}{} {} 竞价 {:.2}（{:+.2}%）→ 竞价参考价 {:.2}〔{}〕{}",
                self.code,
                self.name,
                self.auction_price,
                self.auction_pct,
                price,
                self.tier.reference_label(),
                note
            ),
            (OutputKind::ReferencePrice, None) => format!(
                "{} {} 竞价 {:.2}（{:+.2}%）→ 不给参考价〔{}〕{}",
                self.code,
                self.name,
                self.auction_price,
                self.auction_pct,
                self.tier.reference_label(),
                note
            ),
            (OutputKind::LegacyAction, Some(price)) => format!(
                "{} {} 竞价 {:.2}（{:+.2}%）→ 买价 {:.2}〔{}〕{}",
                self.code,
                self.name,
                self.auction_price,
                self.auction_pct,
                price,
                self.tier.label(),
                note
            ),
            (OutputKind::LegacyAction, None) => format!(
                "{} {} 竞价 {:.2}（{:+.2}%）→ 不给价〔{}〕{}",
                self.code,
                self.name,
                self.auction_price,
                self.auction_pct,
                self.tier.label(),
                note
            ),
        };
        let mut rendered = if self.position_note.is_empty() {
            base
        } else {
            format!("{base}\n  {}", self.position_note)
        };
        if let Some(note) = &self.tech_note {
            rendered.push_str(&format!("\n  {note}"));
        }
        if let Some(note) = &self.plan_note {
            rendered.push_str(&format!("\n  {note}"));
        }
        if matches!(kind, OutputKind::ReferencePrice) {
            if let Some(risk) = &self.risk_note {
                let risk = risk.strip_prefix("风险提示：").unwrap_or(risk);
                let conflict = if self.price.is_some() {
                    "｜冲突：风险提示与参考价同时存在"
                } else {
                    ""
                };
                rendered.push_str(&format!("\n  ⚠ 风险提示：{risk}{conflict}"));
            }
        }
        rendered
    }
}

#[derive(Debug, Clone)]
pub struct BoardHeat {
    pub board_name: String,
    pub mean_pct: f64,
    pub median_volume_ratio: Option<f64>,
    pub members: usize,
}

impl BoardHeat {
    pub fn render(&self) -> String {
        let ratio = self
            .median_volume_ratio
            .map(|value| format!(" 量比中位 {value:.2}"))
            .unwrap_or_else(|| " 量比 —".to_string());
        format!(
            "{} 竞价均 {:+.2}%（{} 只）{}",
            self.board_name, self.mean_pct, self.members, ratio
        )
    }
}

/// 一条 09:25 决策的原始事实，供 6.3 证据包落盘。
#[derive(Debug, Clone)]
pub struct EvidenceFact {
    pub ticket_id: i64,
    pub code: String,
    pub name: String,
    pub auction_price: f64,
    pub auction_pct: f64,
    pub tier: String,
    pub quoted_price: Option<f64>,
    pub streak: Option<u32>,
    pub streak_source_date: Option<String>,
    pub phase: Option<String>,
    pub phase_source_date: Option<String>,
    pub position_quantity: Option<u64>,
    pub cost_price: Option<f64>,
    pub position_error: Option<String>,
    /// 6.10：risk_flag 摘要（最近 5 日），写证据包并与正文同源。
    pub risk_note: Option<String>,
    /// 6.8：触发前 3 日涨幅；仅作证据事实，不参与给价/趋势判断。
    pub pre3_gain: Option<f64>,
    /// 6.9：提案可成交性；由当日竞价价与提案区间比较。
    pub fillability: Option<String>,
    pub fillability_note: Option<String>,
}

#[derive(Debug, Default)]
pub struct AuctionPriceReport {
    pub emotion: usize,
    pub quoted: Vec<PriceLine>,
    pub skipped_high: Vec<PriceLine>,
    pub no_price: usize,
    /// 涨停池/市场阶段快照缺失的票数（高位状态未核实）。
    pub high_state_unknown: usize,
    /// 情绪高潮期被压掉的连板票（2026-10-24）
    pub skipped_phase: Vec<PriceLine>,
    /// 最新提案已是终态、按 state_v1 不重复定价的票（shadow 只计数）。
    pub skipped_stale: usize,
    pub errors: Vec<String>,
    pub board_heat: Vec<BoardHeat>,
    /// 每条带价决策的原始事实（6.3 证据包用）。
    pub evidence_facts: Vec<EvidenceFact>,
}

impl AuctionPriceReport {
    pub fn summary_line(&self) -> String {
        let label = if crate::monitor::output_contract::reference_language_enabled() {
            "竞价参考价"
        } else {
            "竞价买价"
        };
        format!(
            "[{}] emotion={} quoted={} skipped_high={} skipped_phase={} no_price={} high_state_unknown={} skipped_stale={} errors={}",
            label,
            self.emotion,
            self.quoted.len(),
            self.skipped_high.len(),
            self.skipped_phase.len(),
            self.no_price,
            self.high_state_unknown,
            self.skipped_stale,
            self.errors.len()
        )
    }

    /// 推送正文；没有情绪型票时返回 `None`（不推空消息）。
    pub fn render(&self, as_of: &str) -> Option<String> {
        self.render_with(as_of, crate::monitor::output_contract::reference_language_enabled())
    }

    /// 6.10：同一份正文可按参考/legacy 语义渲染，便于四态单测，
    /// 避免测试修改进程级环境变量。
    pub fn render_with(&self, as_of: &str, reference: bool) -> Option<String> {
        if self.emotion == 0 {
            return None;
        }
        let mut out = String::new();
        if reference {
            out.push_str(&format!("【{as_of} 09:25 竞价参考价】\n"));
        } else {
            out.push_str(&format!("【{as_of} 09:25 竞价买价】\n"));
        }
        out.push_str(&format!(
            "情绪型 {} 只（趋势型走 ma 回踩，见明日计划）\n",
            self.emotion
        ));
        for line in &self.quoted {
            out.push_str(&format!("\n{}\n", line.render()));
        }
        for line in &self.skipped_high {
            out.push_str(&format!("\n{}\n", line.render()));
        }
        if !self.skipped_phase.is_empty() {
            out.push_str(&format!(
                "\n情绪高潮 · 连板≥{} 不追（{} 只）\n",
                crate::monitor::market_regime::EMOTION_STREAK,
                self.skipped_phase.len()
            ));
            for line in &self.skipped_phase {
                out.push_str(&format!(
                    "· {} {} 竞价{:+.2}%\n",
                    line.code, line.name, line.auction_pct
                ));
            }
        }
        if self.no_price > 0 {
            let no_price_label = if reference {
                "不给参考价（证据不足）"
            } else {
                "不给价"
            };
            out.push_str(&format!(
                "\n{} 只竞价价缺失（停牌/未撮合），{}\n",
                self.no_price, no_price_label
            ));
        }
        if !self.board_heat.is_empty() {
            out.push_str("\n📊 板块热度（竞价均涨幅）\n");
            for heat in &self.board_heat {
                out.push_str(&format!("· {}\n", heat.render()));
            }
        }
        if reference {
            out.push_str("\n⚠ 参考价，不是下单指令；未核验成交，T+1 不可卖。");
        } else {
            out.push_str("\n⚠ 竞价买入当日 T+1 不可卖。给的是参考价，不是成交承诺。");
        }
        Some(out)
    }
}

/// 板块热度（2.1）：竞价 rows 按 `board_member` 聚合，取均涨幅前 5。
fn board_heat(conn: &mut SqliteConnection, rows: &[AuctionRow]) -> Result<Vec<BoardHeat>, String> {
    #[derive(QueryableByName)]
    struct MemberRow {
        #[diesel(sql_type = Text)]
        board_name: String,
        #[diesel(sql_type = Text)]
        instrument_code: String,
    }
    let members: Vec<MemberRow> =
        diesel::sql_query("SELECT board_name, instrument_code FROM board_member")
            .load(conn)
            .map_err(|error| format!("板块成分查询失败: {error}"))?;
    if members.is_empty() {
        return Ok(Vec::new());
    }

    let by_code: HashMap<&str, &AuctionRow> = rows
        .iter()
        .filter_map(|row| row.auction_pct.map(|_| (row.code.as_str(), row)))
        .collect();

    let mut buckets: HashMap<&str, (Vec<f64>, Vec<f64>)> = HashMap::new();
    for member in &members {
        let Some(row) = by_code.get(member.instrument_code.as_str()) else {
            continue;
        };
        let Some(pct) = row.auction_pct else {
            continue;
        };
        let entry = buckets.entry(member.board_name.as_str()).or_default();
        entry.0.push(pct);
        if let Some(ratio) = row.auction_volume_ratio {
            entry.1.push(ratio);
        }
    }

    let mut heats: Vec<BoardHeat> = buckets
        .into_iter()
        .filter(|(_, (pcts, _))| pcts.len() >= 2)
        .map(|(board_name, (pcts, ratios))| {
            let mean = pcts.iter().sum::<f64>() / pcts.len() as f64;
            BoardHeat {
                board_name: board_name.to_string(),
                mean_pct: mean,
                median_volume_ratio: median(&ratios),
                members: pcts.len(),
            }
        })
        .collect();
    heats.sort_by(|a, b| {
        b.mean_pct
            .partial_cmp(&a.mean_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    heats.truncate(BOARD_HEAT_TOP);
    Ok(heats)
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    Some(if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    })
}

/// 6.10/state_v1：是否真的把终态提案排除出 09:25 定价。
/// 默认 shadow：只记“将跳过”，不改变推送；`TICKET_STATE_FILTER=on` 才跳过。
fn state_filter_enabled() -> bool {
    matches!(
        std::env::var("TICKET_STATE_FILTER").as_deref(),
        Ok("on") | Ok("1")
    )
}

/// 6.8/6.10：触发前 3 日涨幅。取 D-1 与 D-4 收盘；不足 4 根日线返回 None，
/// 不拿更早的日期硬凑。
fn pre3_gain_on(conn: &mut SqliteConnection, code: &str, as_of: &str) -> Option<f64> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Double)]
        close: f64,
    }
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT close FROM stock_daily WHERE code = ? AND date < ? \
         ORDER BY date DESC LIMIT 4",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(as_of)
    .load(conn)
    .ok()?;
    if rows.len() < 4 || rows[3].close <= 0.0 {
        return None;
    }
    Some(rows[0].close / rows[3].close - 1.0)
}

/// 6.9：用当日竞价价与提案区间算可成交性。只做事实标记，
/// 不改变给价；`unfillable_gap` = 竞价已高于区间上沿 3% 以上。
fn fillability_on(
    conn: &mut SqliteConnection,
    ticket_id: i64,
    as_of: &str,
    auction_price: f64,
) -> (Option<String>, Option<String>) {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Nullable<Double>)]
        price_low: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        price_high: Option<f64>,
    }
    let row: Option<Row> = diesel::sql_query(
        "SELECT price_low, price_high FROM ticket_decision \
         WHERE ticket_id = ? AND as_of = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind::<diesel::sql_types::BigInt, _>(ticket_id)
    .bind::<Text, _>(as_of)
    .get_result(conn)
    .ok();
    let Some(row) = row else {
        return (None, None);
    };
    let (Some(low), Some(high)) = (row.price_low, row.price_high) else {
        return (None, None);
    };
    if high <= 0.0 {
        return (None, None);
    }
    let (state, note) = classify_fillability(auction_price, low, high);
    (Some(state), Some(note))
}

/// 纯函数：竞价价 vs 提案区间。`unfillable_gap` 表示竞价已高于
/// 区间上沿 3% 以上，按区间挂单当天不可成交。
fn classify_fillability(auction_price: f64, low: f64, high: f64) -> (String, String) {
    if auction_price > high * 1.03 {
        (
            "unfillable_gap".to_owned(),
            format!("竞价 {auction_price:.2} 高于区间上沿 {high:.2} 超过 3%"),
        )
    } else if auction_price > high {
        (
            "above_zone".to_owned(),
            format!("竞价 {auction_price:.2} 高于区间上沿 {high:.2}"),
        )
    } else if auction_price < low {
        (
            "below_zone".to_owned(),
            format!("竞价 {auction_price:.2} 低于区间下沿 {low:.2}"),
        )
    } else {
        (
            "in_zone".to_owned(),
            format!("竞价 {auction_price:.2} 在区间 {low:.2}~{high:.2}"),
        )
    }
}

/// 日线指标 + 量价形态 + 竞价量比，拼成一行；09:25 只展示，不参与给价。
fn daily_note(
    auction_volume_ratio: Option<f64>,
    tech: &ticket_card::TechView,
    volume_price: &ticket_card::VolumePriceView,
) -> String {
    let mut parts = Vec::new();
    parts.push(match auction_volume_ratio {
        Some(value) => format!("竞价量比 {value:.2}（未参与触发）"),
        None => "竞价量比 N/A（未参与触发）".to_owned(),
    });
    if tech.as_of.is_empty() {
        parts.push("日线：N/A（K线不足）".to_owned());
    } else {
        let date = tech.as_of.get(5..).unwrap_or(tech.as_of.as_str());
        parts.push(format!(
            "日线({date})：MACD {} 柱{}｜KDJ {}｜SKDJ {}｜RSI {}",
            tech.macd_cross.unwrap_or("N/A"),
            tech.macd_hist
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "N/A".to_owned()),
            fmt_triplet(tech.kdj),
            fmt_triplet(tech.skdj),
            fmt_triplet(tech.rsi),
        ));
    }
    match volume_price.ratio_5d {
        Some(ratio) => parts.push(format!(
            "量价 {}｜5日量比 {ratio:.2}",
            volume_price.tags.join("+")
        )),
        None => parts.push("量价 N/A".to_owned()),
    }
    parts.join("｜")
}

fn fmt_triplet(value: Option<(f64, f64, f64)>) -> String {
    match value {
        Some((a, b, c)) => format!("{a:.0}/{b:.0}/{c:.0}"),
        None => "N/A".to_owned(),
    }
}

fn proposal_status_label(status: &str) -> &'static str {
    match status {
        "triggered" => "触发",
        "invalidated" => "失效",
        "expired" => "过期",
        "superseded" => "被替换",
        _ => "未定",
    }
}

/// 最新提案区间与竞价位置；终态只作复盘，避免把旧区间当新参考。
fn plan_note_on(
    conn: &mut SqliteConnection,
    ticket_id: i64,
    auction_price: f64,
) -> Option<String> {
    let status = crate::monitor::ticket_decision::latest_proposal_status(conn, ticket_id)
        .ok()
        .flatten();
    let context = ticket_card::latest_decision(conn, ticket_id).ok().flatten();
    let Some(context) = context else {
        return Some("提案：无有效提案".to_owned());
    };
    let status_text = status.map(|value| value.as_str()).unwrap_or("open");
    let terminal = matches!(
        status_text,
        "triggered" | "invalidated" | "expired" | "superseded"
    );
    let mut note = match (context.price_low, context.price_high) {
        (Some(low), Some(high)) if high > 0.0 => {
            let zero_width = (high - low).abs() < 0.005;
            let breakout = context.entry_mode.as_str() == "breakout";
            let breakout_trigger = context.trigger_level.unwrap_or(low);
            let position = if breakout {
                if auction_price >= breakout_trigger {
                    "竞价站上触发位（价格已触及，非成交）"
                } else {
                    "竞价低于触发位（等待）"
                }
            } else if auction_price > high {
                "竞价高于上沿（不追）"
            } else if auction_price < low {
                "竞价低于下沿（等待）"
            } else {
                "竞价在区间内"
            };
            let range = if zero_width {
                format!("触发位 {low:.2}")
            } else {
                format!("区间 {low:.2}~{high:.2}")
            };
            let trigger = if zero_width {
                String::new()
            } else {
                context
                    .trigger_level
                    .map(|value| format!("｜突破 {value:.2}"))
                    .unwrap_or_default()
            };
            let abandon = context
                .abandon
                .map(|value| format!("｜放弃 {value:.2}"))
                .unwrap_or_default();
            format!("提案：{range}{trigger}{abandon}｜{position}")
        }
        _ => format!("提案：{}（无区间）", proposal_status_label(status_text)),
    };
    if terminal {
        note.push_str(&format!(
            "｜提案已{}，仅复盘",
            proposal_status_label(status_text)
        ));
    }
    Some(note)
}

/// 给情绪型活跃票算竞价买价。
///
/// `rows` 由 `auction_watch::observe` 提供（活跃票 ∪ 持仓的竞价终态）。
pub fn build(
    db: &DatabaseManager,
    as_of: &str,
    rows: &[AuctionRow],
) -> Result<AuctionPriceReport, String> {
    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    let mut report = AuctionPriceReport::default();

    // 6.4：持仓上下文。读取失败不能当成「无持仓」，单独写「读取失败」。
    let (positions, positions_error) = match crate::portfolio::load_positions() {
        Ok(positions) => (
            positions
                .into_iter()
                .map(|p| (p.code.clone(), (p.shares, p.cost_price)))
                .collect::<HashMap<_, _>>(),
            None,
        ),
        Err(error) => (HashMap::new(), Some(error)),
    };

    // 6.10：同日冲突披露。risk_flag 由 15:52 排雷写 ticket_event，
    // 这里只读取最近 5 日；signal_fusion 分值未落库，不伪造。
    #[derive(QueryableByName)]
    struct RiskRow {
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Nullable<Text>)]
        summary: Option<String>,
    }
    let risk_notes: HashMap<String, String> = diesel::sql_query(
        "SELECT t.code AS code, e.summary AS summary \
           FROM ticket_event e JOIN ticket_lifecycle t ON t.id = e.ticket_id \
          WHERE e.event_kind = 'risk_flag' AND date(e.event_at) >= date(?1, '-5 day') \
          ORDER BY e.event_at ASC, e.id ASC",
    )
    .bind::<Text, _>(as_of)
    .load::<RiskRow>(&mut conn)
    .map(|rows| {
        rows.into_iter()
            .filter_map(|row| row.summary.map(|summary| (row.code, summary)))
            .collect()
    })
    .unwrap_or_default();

    for row in rows {
        let ticket = match ticket_lifecycle::find_active_ticket(&mut conn, &row.code) {
            Ok(Some(ticket)) => ticket,
            Ok(None) => continue,
            Err(error) => {
                report.errors.push(format!("{} {error}", row.code));
                continue;
            }
        };
        // state_v1：已经 triggered/expired/invalidated/superseded 的旧提案，
        // 09:25 不再当新提案定价。默认 shadow 只记“将跳过”。
        if let Ok(Some(state)) = ticket_lifecycle::ticket_state(&mut conn, ticket.id) {
            if matches!(
                state.proposal_status.as_deref(),
                Some("triggered" | "invalidated" | "expired" | "superseded")
            ) {
                report.skipped_stale += 1;
                if state_filter_enabled() {
                    log::info!(
                        "[竞价买价] {} 提案状态={:?}，旧提案不重复定价",
                        row.code,
                        state.proposal_status
                    );
                    continue;
                }
                log::info!(
                    "[竞价买价][state-v1] shadow 将跳过 {} 状态={:?}",
                    row.code,
                    state.proposal_status
                );
            }
        }
        let decision =
            match ticket_regime::resolve(&mut conn, ticket.id, &row.code, &ticket.source, as_of) {
                Ok(decision) => decision,
                Err(error) => {
                    report
                        .errors
                        .push(format!("{} 分型失败: {error}", row.code));
                    continue;
                }
            };
        if decision.regime != Regime::Emotion {
            continue;
        }
        report.emotion += 1;

        let (Some(auction_price), Some(auction_pct)) = (row.auction_price, row.auction_pct) else {
            report.no_price += 1;
            continue;
        };
        // 2026-10-24：情绪高潮期不给连板 ≥4 的票定价 —— 文章说的「情绪高潮不追高」，
        // 我们这边就用市场级情绪档来压，而不是逐票拍脑袋。
        let streak_src = crate::monitor::market_regime::streak_with_source_on(
            &mut conn, as_of, &row.code,
        );
        let phase_src = crate::monitor::market_regime::phase_with_source_on(&mut conn, as_of);
        let high_state_unverified = streak_src.is_none() || phase_src.is_none();
        if high_state_unverified {
            report.high_state_unknown += 1;
            log::info!(
                "[竞价买价] {} 高位状态未核实 streak_src={:?} phase_src={:?}",
                row.code,
                streak_src.as_ref().map(|(_, date)| date.as_str()),
                phase_src.as_ref().map(|(_, date)| date.as_str())
            );
        } else {
            log::debug!(
                "[竞价买价] {} streak={:?} src={:?} phase={:?} src={:?}",
                row.code,
                streak_src.as_ref().map(|(value, _)| *value),
                streak_src.as_ref().map(|(_, date)| date.as_str()),
                phase_src.as_ref().map(|(value, _)| *value),
                phase_src.as_ref().map(|(_, date)| date.as_str())
            );
        }
        let streak = streak_src.as_ref().map(|(value, _)| *value);
        let phase = phase_src.as_ref().map(|(value, _)| *value);
        let pre3_gain = pre3_gain_on(&mut conn, &row.code, as_of);
        let (fillability, fillability_note) =
            fillability_on(&mut conn, ticket.id, as_of, auction_price);
        let position_note = match &positions_error {
            Some(error) => format!("持仓：读取失败（{error}）"),
            None => match positions.get(&row.code) {
                Some((shares, cost)) if *shares > 0 => {
                    if *cost > 0.0 {
                        // D1：动作与 I-04 HoldingPlan 同口径（09:25 早于 09:45 计划）。
                        let (intent, pnl) =
                            crate::decision::holding_plan::position_intent(
                                auction_price,
                                *cost,
                                false,
                            );
                        format!(
                            "持仓：已有 {shares} 股，成本 {cost:.2}，浮盈亏 {pnl:+.2}%｜动作：{}",
                            intent.label()
                        )
                    } else {
                        format!("持仓：已有 {shares} 股，成本缺失（动作无法判定）")
                    }
                }
                _ => "持仓：无（新买入参考）".to_string(),
            },
        };
        let position_context = match &positions_error {
            Some(_) => PositionContext::Unknown,
            None => match positions.get(&row.code) {
                Some((shares, _)) if *shares > 0 => PositionContext::Held,
                _ => PositionContext::NoPosition,
            },
        };
        let risk_note = risk_notes.get(&row.code).cloned();
        let (daily_tech, volume_price) =
            ticket_card::daily_tech_before(&mut conn, &row.code, as_of).unwrap_or_else(|error| {
                report
                    .errors
                    .push(format!("{} 日线指标失败: {error}", row.code));
                (
                    ticket_card::TechView::default(),
                    ticket_card::VolumePriceView::default(),
                )
            });
        let tech_note = Some(daily_note(
            row.auction_volume_ratio,
            &daily_tech,
            &volume_price,
        ));
        let plan_note = plan_note_on(&mut conn, ticket.id, auction_price);
        let line_for_skip = || PriceLine {
            code: row.code.clone(),
            name: row.name.clone(),
            auction_price,
            auction_pct,
            price: None,
            tier: Tier::SkipHigh,
            output_kind: OutputKind::ReferencePrice,
            high_state_unverified,
            position_note: position_note.clone(),
            position: position_context,
            risk_note: risk_note.clone(),
            auction_volume_ratio: row.auction_volume_ratio,
            tech_note: tech_note.clone(),
            plan_note: plan_note.clone(),
        };
        if streak.is_some_and(|value| value >= crate::monitor::market_regime::EMOTION_STREAK)
            && phase == Some(crate::monitor::market_regime::MarketPhase::Climax)
        {
            report.skipped_phase.push(line_for_skip());
            continue;
        }
        let tier = tier_for(auction_pct);
        let price = quote_price(auction_price, tier);
        report.evidence_facts.push(EvidenceFact {
            ticket_id: ticket.id,
            code: row.code.clone(),
            name: row.name.clone(),
            auction_price,
            auction_pct,
            tier: tier.key().to_owned(),
            quoted_price: price,
            streak,
            streak_source_date: streak_src.as_ref().map(|(_, date)| date.clone()),
            phase: phase.map(|value| value.code().to_owned()),
            phase_source_date: phase_src.as_ref().map(|(_, date)| date.clone()),
            position_quantity: positions.get(&row.code).map(|(shares, _)| *shares),
            cost_price: positions.get(&row.code).map(|(_, cost)| *cost),
            position_error: positions_error.clone(),
            risk_note: risk_note.clone(),
            pre3_gain,
            fillability,
            fillability_note,
        });
        let line = PriceLine {
            code: row.code.clone(),
            name: row.name.clone(),
            auction_price,
            auction_pct,
            price,
            tier,
            output_kind: OutputKind::ReferencePrice,
            high_state_unverified,
            position_note,
            position: position_context,
            risk_note,
            auction_volume_ratio: row.auction_volume_ratio,
            tech_note,
            plan_note,
        };
        match tier {
            Tier::SkipHigh => report.skipped_high.push(line),
            _ => report.quoted.push(line),
        }
    }

    report.board_heat = board_heat(&mut conn, rows)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_boundaries_match_backtest() {
        assert_eq!(tier_for(6.01), Tier::SkipHigh);
        assert_eq!(tier_for(6.0), Tier::Light);
        assert_eq!(tier_for(2.0), Tier::Light);
        assert_eq!(tier_for(1.99), Tier::EnsureFill);
        assert_eq!(tier_for(-2.0), Tier::EnsureFill);
        assert_eq!(tier_for(-2.01), Tier::Bargain);
    }

    #[test]
    fn ensure_fill_markup_is_half_a_percent() {
        let price = quote_price(10.0, Tier::EnsureFill).expect("quoted");
        assert!((price - 10.05).abs() < 1e-9);
        // 轻仓与低开不给上浮
        assert_eq!(quote_price(10.0, Tier::Light), Some(10.0));
        assert_eq!(quote_price(10.0, Tier::Bargain), Some(10.0));
        // 高开帽：不给价
        assert_eq!(quote_price(10.0, Tier::SkipHigh), None);
        // 缺价不补 0
        assert_eq!(quote_price(0.0, Tier::Light), None);
    }

    #[test]
    fn render_always_carries_t1_warning() {
        let report = AuctionPriceReport {
            emotion: 1,
            quoted: vec![PriceLine {
                code: "002696".to_string(),
                name: "测试".to_string(),
                auction_price: 10.0,
                auction_pct: 3.0,
                price: Some(10.0),
                tier: Tier::Light,
                output_kind: OutputKind::ReferencePrice,
                high_state_unverified: false,
                position_note: "持仓：无（新买入参考）".to_string(),
                position: PositionContext::NoPosition,
                risk_note: None,
                auction_volume_ratio: None,
                tech_note: None,
                plan_note: None,
            }],
            ..Default::default()
        };
        let text = report.render("2026-10-23").expect("non-empty");
        assert!(text.contains("T+1 不可卖"));
        assert!(text.contains("002696"));
        assert!(text.contains("轻仓"));
    }

    #[test]
    fn render_includes_volume_indicators_and_plan_status() {
        let line = PriceLine {
            code: "002205".to_string(),
            name: "国统股份".to_string(),
            auction_price: 10.10,
            auction_pct: -1.20,
            price: Some(10.10),
            tier: Tier::EnsureFill,
            output_kind: OutputKind::ReferencePrice,
            high_state_unverified: false,
            position_note: "持仓：无（新买入参考）".to_string(),
            position: PositionContext::NoPosition,
            risk_note: None,
            auction_volume_ratio: Some(1.32),
            tech_note: Some(
                "竞价量比 1.32｜日线(11-13)：MACD 金叉 柱0.05｜KDJ 70/60/90｜SKDJ 40/35/45｜RSI 62/55/48｜量价 放量上涨｜5日量比 2.50".to_string(),
            ),
            plan_note: Some(
                "提案：区间 10.25~10.85｜突破 10.85｜放弃 9.21｜竞价低于下沿（等待）".to_string(),
            ),
        };
        let text = line.render_as(OutputKind::ReferencePrice);
        assert!(text.contains("竞价量比 1.32"));
        assert!(text.contains("日线(11-13)"));
        assert!(text.contains("量价 放量上涨"));
        assert!(text.contains("提案：区间 10.25~10.85"));
        assert!(text.contains("竞价低于下沿"));
        assert!(!text.contains("建议买入"));
        assert!(!text.contains("保证成交"));
    }

    #[test]
    fn reference_output_never_claims_buy_or_fill() {
        let line = PriceLine {
            code: "001216".to_string(),
            name: "华瓷股份".to_string(),
            auction_price: 22.78,
            auction_pct: -5.41,
            price: Some(22.78),
            tier: Tier::Bargain,
            output_kind: OutputKind::ReferencePrice,
            high_state_unverified: false,
            position_note: "持仓：无（新买入参考）".to_string(),
            position: PositionContext::NoPosition,
            risk_note: None,
            auction_volume_ratio: None,
            tech_note: None,
            plan_note: None,
        };
        let text = line.render_as(OutputKind::ReferencePrice);
        assert!(text.contains("竞价参考价"));
        assert!(!text.contains("买价"));
        assert!(!text.contains("保成交"));
        assert!(!text.contains("保证成交"));
        assert!(!text.contains("可执行"));
    }

    #[test]
    fn unverified_high_state_is_visible_in_text() {
        let line = PriceLine {
            code: "001216".to_string(),
            name: "华瓷股份".to_string(),
            auction_price: 22.78,
            auction_pct: -5.41,
            price: Some(22.78),
            tier: Tier::Bargain,
            output_kind: OutputKind::ReferencePrice,
            high_state_unverified: true,
            position_note: "持仓：已有 500 股，成本 26.61，浮盈亏 -4.9%｜动作：加仓观察".to_string(),
            position: PositionContext::Held,
            risk_note: None,
            auction_volume_ratio: None,
            tech_note: None,
            plan_note: None,
        };
        let text = line.render_as(OutputKind::ReferencePrice);
        assert!(text.contains("高位状态未核实"));
        assert!(text.contains("持仓：已有 500 股"));
        assert!(text.contains("动作：加仓观察"));
    }

    #[test]
    fn fillability_marks_gap_above_zone_as_unfillable() {
        // 601188 2026-11-10：竞价 4.61 vs 提案 3.48~3.55。
        let (state, note) = classify_fillability(4.61, 3.48, 3.55);
        assert_eq!(state, "unfillable_gap");
        assert!(note.contains("超过 3%"));
        assert_eq!(classify_fillability(3.50, 3.48, 3.55).0, "in_zone");
        assert_eq!(classify_fillability(3.56, 3.48, 3.55).0, "above_zone");
        assert_eq!(classify_fillability(3.40, 3.48, 3.55).0, "below_zone");
    }

    #[test]
    fn reference_prefix_and_risk_conflict_are_explicit() {
        let mut line = PriceLine {
            code: "001216".to_string(),
            name: "华瓷股份".to_string(),
            auction_price: 22.78,
            auction_pct: -5.41,
            price: Some(22.78),
            tier: Tier::Bargain,
            output_kind: OutputKind::ReferencePrice,
            high_state_unverified: false,
            position_note: "持仓：已有 100 股，成本 61.80，浮盈亏 -72.06%｜动作：止损核查"
                .to_string(),
            position: PositionContext::Held,
            risk_note: Some("风险提示：违规减持".to_string()),
            auction_volume_ratio: None,
            tech_note: None,
            plan_note: None,
        };
        let text = line.render_as(OutputKind::ReferencePrice);
        assert!(text.contains("持仓加仓参考｜"));
        assert!(text.contains("风险提示：违规减持"));
        assert!(text.contains("冲突：风险提示与参考价同时存在"));
        assert!(text.contains("竞价参考价"));

        line.position = PositionContext::NoPosition;
        line.position_note = "持仓：无（新买入参考）".to_string();
        let text = line.render_as(OutputKind::ReferencePrice);
        assert!(text.contains("新买入参考｜"));

        line.position = PositionContext::Unknown;
        let text = line.render_as(OutputKind::ReferencePrice);
        assert!(text.contains("持仓未知｜"));

        // 不给价时不写「冲突」，但风险仍必须披露。
        line.price = None;
        line.tier = Tier::SkipHigh;
        line.position = PositionContext::Held;
        let text = line.render_as(OutputKind::ReferencePrice);
        assert!(text.contains("不给参考价"));
        assert!(!text.contains("冲突"));
        assert!(text.contains("风险提示：违规减持"));

        // legacy 文案保持 shadow 期原样，不新增风险行。
        let legacy = line.render_as(OutputKind::LegacyAction);
        assert!(!legacy.contains("风险提示"));
    }

    #[test]
    fn missing_auction_is_labelled_evidence_insufficient_in_reference_mode() {
        let report = AuctionPriceReport {
            emotion: 1,
            no_price: 1,
            ..Default::default()
        };
        let reference = report.render_with("2026-11-10", true).expect("non-empty");
        assert!(reference.contains("不给参考价（证据不足）"));
        let legacy = report.render_with("2026-11-10", false).expect("non-empty");
        assert!(legacy.contains("不给价"));
        assert!(!legacy.contains("证据不足"));
    }

    #[test]
    fn no_emotion_tickets_means_no_push() {
        let report = AuctionPriceReport::default();
        assert!(report.render("2026-10-23").is_none());
    }

    #[test]
    fn median_handles_even_and_odd() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), Some(2.5));
        assert_eq!(median(&[]), None);
    }
}
