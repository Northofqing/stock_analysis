//! 单票决策卡（2026-11-13）：日线指标 + 量能 + 进出场 + 当前状态。
//!
//! 为什么单独一层：现有 ticket 推送只有价格和区间，没有指标/量能/明确的
//! 进出场判断。这里把已有的 `indicators` 模块（MACD/KDJ/SKDJ/RSI）、
//! `ticket_decision` 的价位和 `holding_plan` 的持仓动作合成一张只读卡，
//! 供 CLI 与后续 09:25/盘中推送复用。
//!
//! 口径边界：
//! - 指标一律是**日线**，截至 `stock_daily` 最后一根已入库 K 线；不把日线
//!   指标冒充盘中指标。盘中只叠加现价/涨跌幅/量比（由调用方传入）。
//! - 缺失写 `None` 并进入 `missing`，不补 0；不伪造 SKDJ/RSI 数值。

use diesel::sql_types::{BigInt, Double, Nullable, Text};
use diesel::{OptionalExtension, QueryableByName, RunQueryDsl, SqliteConnection};

use crate::database::{ticket_lifecycle, DatabaseManager};
use crate::indicators::{calc_kdj, calc_macd, calc_rsi, calc_skdj, detect_cross};
use crate::monitor::buy_zone;
use crate::monitor::ticket_decision::DecisionContext;
use crate::monitor::ticket_regime;

#[derive(Debug, Clone)]
struct Bar {
    date: String,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    pct_chg: Option<f64>,
    volume_ratio: Option<f64>,
}

#[derive(QueryableByName)]
struct BarRow {
    #[diesel(sql_type = Text)]
    date: String,
    #[diesel(sql_type = Double)]
    high: f64,
    #[diesel(sql_type = Double)]
    low: f64,
    #[diesel(sql_type = Double)]
    close: f64,
    #[diesel(sql_type = Double)]
    volume: f64,
    #[diesel(sql_type = Nullable<Double>)]
    pct_chg: Option<f64>,
    #[diesel(sql_type = Nullable<Double>)]
    volume_ratio: Option<f64>,
}

/// 日线技术指标视图；`as_of` 是最后一根已入库 K 线的日期。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct TechView {
    pub as_of: String,
    pub close: Option<f64>,
    pub pct_chg: Option<f64>,
    pub volume_ratio: Option<f64>,
    pub macd_dif: Option<f64>,
    pub macd_dea: Option<f64>,
    pub macd_hist: Option<f64>,
    pub macd_cross: Option<&'static str>,
    pub kdj: Option<(f64, f64, f64)>,
    pub skdj: Option<(f64, f64, f64)>,
    pub rsi: Option<(f64, f64, f64)>,
    /// 20 日均线（收盘均值）；不足 20 根为 None。
    pub ma20: Option<f64>,
    /// 14 日 ATR（真实波幅均值）；不足 15 根为 None。
    pub atr14: Option<f64>,
    /// 最后一根日线是否涨停（按板分档，和 ticket_regime 同源）。
    pub limit_up: bool,
}

/// 量价形态视图（日线口径）：放量/缩量 + 涨停/滞涨/涨跌的组合。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct VolumePriceView {
    /// 当日成交量 / 前 5 日均量（不含当日）。
    pub ratio_5d: Option<f64>,
    /// 当日振幅（(high-low)/前收 × 100）。
    pub amplitude_pct: Option<f64>,
    pub tags: Vec<&'static str>,
}

/// 进出场视图；`decision_id` 为空表示没有可用提案。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct PlanView {
    pub decision_id: Option<String>,
    pub proposal_status: Option<String>,
    pub entry_mode: Option<String>,
    pub low: Option<f64>,
    pub high: Option<f64>,
    pub trigger: Option<f64>,
    pub abandon: Option<f64>,
    pub valid_until: Option<String>,
    /// 持仓才有：成本止损参考线（cost × 0.92）。
    pub stop: Option<f64>,
    /// 持仓才有：减仓观察区（cost × 1.02 ~ cost × 1.05）。
    pub reduce_low: Option<f64>,
    pub reduce_high: Option<f64>,
    /// 持仓才有：动作倾向 + 浮盈亏。
    pub position_action: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Card {
    pub code: String,
    pub name: Option<String>,
    pub stage: String,
    pub holding: bool,
    pub shares: Option<u64>,
    pub cost: Option<f64>,
    /// 当日买入、T+1 锁定的股数（只读 trades）；无持仓为 None。
    pub locked_shares: Option<u64>,
    pub tech: TechView,
    pub volume_price: VolumePriceView,
    pub plan: PlanView,
    pub status: String,
    pub missing: Vec<String>,
}

/// 构建全部活跃票 + 持仓的决策卡。
pub fn build_cards(db: &DatabaseManager, as_of: &str) -> Result<Vec<Card>, String> {
    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    build_cards_with_conn(&mut conn, as_of)
}

/// 只读连接版本：CLI/巡检不经过 `DatabaseManager::init` 的迁移与全量校验。
pub fn build_cards_with_conn(
    conn: &mut SqliteConnection,
    as_of: &str,
) -> Result<Vec<Card>, String> {
    #[derive(QueryableByName)]
    struct TicketRow {
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Text)]
        stage: String,
    }
    let mut rows: Vec<(String, String)> = diesel::sql_query(
        "SELECT code, stage FROM ticket_lifecycle WHERE close_date IS NULL \
         ORDER BY CASE stage WHEN 'holding' THEN 0 WHEN 'entry_proposed' THEN 1 ELSE 2 END, code",
    )
    .load::<TicketRow>(conn)
    .map_err(|error| format!("活跃票查询失败: {error}"))?
    .into_iter()
    .map(|row| (row.code.trim().to_owned(), row.stage))
    .collect();

    #[derive(QueryableByName)]
    struct PositionRow {
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Double)]
        buy_price: f64,
        #[diesel(sql_type = BigInt)]
        quantity: i64,
    }
    let positions: Vec<(String, f64, u64)> = diesel::sql_query(
        "SELECT code, buy_price, quantity FROM stock_position WHERE status = 'open' ORDER BY code",
    )
    .load::<PositionRow>(conn)
    .map_err(|error| format!("持仓查询失败: {error}"))?
    .into_iter()
    .filter(|row| row.quantity > 0)
    .map(|row| (row.code, row.buy_price, row.quantity as u64))
    .collect();

    for (code, _, _) in &positions {
        if !rows.iter().any(|(existing, _)| existing == code) {
            rows.push((code.clone(), "holding".to_owned()));
        }
    }

    let mut cards = Vec::with_capacity(rows.len());
    for (code, stage) in rows {
        let position = positions
            .iter()
            .find(|(item, _, _)| item == &code)
            .map(|(_, cost, shares)| (*cost, *shares));
        match build_card(conn, &code, &stage, as_of, position) {
            Ok(card) => cards.push(card),
            Err(error) => log::warn!("[决策卡] {code} 构建失败: {error}"),
        }
    }
    Ok(cards)
}

/// 构建单票决策卡。`position` 为 `(成本价, 持股数)`；命中持仓时动作走
/// `holding_plan::position_intent` 的同一套阈值。
pub fn build_card(
    conn: &mut SqliteConnection,
    code: &str,
    stage: &str,
    as_of: &str,
    position: Option<(f64, u64)>,
) -> Result<Card, String> {
    let code = code.trim();
    let bars = load_bars(conn, code, as_of, 120)?;
    let tech = compute_tech(code, &bars);
    let volume_price = compute_volume_price(code, &bars);

    let mut plan = PlanView::default();
    let mut missing = tech_missing(&tech, bars.len());

    if let Some(ticket) = ticket_lifecycle::find_active_ticket(conn, code)? {
        if let Some(context) = latest_decision(conn, ticket.id)? {
            plan.decision_id = Some(context.decision_id.clone());
            plan.proposal_status = latest_status(conn, ticket.id)?;
            plan.entry_mode = Some(context.entry_mode.as_str().to_owned());
            plan.low = context.price_low;
            plan.high = context.price_high;
            plan.trigger = context.trigger_level;
            plan.abandon = context.abandon;
            plan.valid_until = context.valid_until;
        }
    }

    let locked_shares = position.map(|(_, shares)| {
        crate::monitor::trade_ledger::bought_quantities_on(conn, as_of)
            .ok()
            .and_then(|map| map.get(code).copied())
            .unwrap_or(0)
            .min(shares)
    });
    let fully_locked = match (position, locked_shares) {
        (Some((_, shares)), Some(locked)) => shares > 0 && locked >= shares,
        _ => false,
    };

    if let Some((cost, shares)) = position {
        if cost > 0.0 {
            plan.stop = Some(cost * 0.92);
            plan.reduce_low = Some(cost * 1.02);
            plan.reduce_high = Some(cost * 1.05);
            if let Some(price) = tech.close {
                let (intent, pnl_pct) =
                    crate::decision::holding_plan::position_intent(price, cost, fully_locked);
                plan.position_action = Some(format!("{}（浮盈亏 {pnl_pct:+.1}%）", intent.label()));
            } else {
                missing.push("price_for_position_action".to_owned());
            }
        } else {
            missing.push("position_cost".to_owned());
        }
        if shares == 0 {
            missing.push("position_shares".to_owned());
        }
    }

    let status = classify(tech.close, &plan, position.is_some());
    if plan.decision_id.is_none() && !position.is_some() {
        missing.push("entry_plan".to_owned());
    }

    Ok(Card {
        code: code.to_owned(),
        name: buy_zone::resolve_name(conn, code),
        stage: stage.to_owned(),
        holding: position.is_some(),
        shares: position.map(|(_, shares)| shares),
        cost: position.map(|(cost, _)| cost),
        locked_shares,
        tech,
        volume_price,
        plan,
        status,
        missing,
    })
}

impl Card {
    /// 单张卡的纯文本渲染；推送层可直接复用。
    pub fn render(&self) -> String {
        let name = self.name.as_deref().unwrap_or("");
        let label = if name.is_empty() {
            self.code.clone()
        } else {
            format!("{}({})", name, self.code)
        };
        let kind = if self.holding { "持仓" } else { "观察" };
        let mut out = format!(
            "【{kind}】{label} {}｜日线 {} 收 {} 量比 {}",
            self.stage,
            empty_dash(&self.tech.as_of),
            fmt_opt(self.tech.close, 2),
            fmt_opt(self.tech.volume_ratio, 2)
        );
        if let Some(pct) = self.tech.pct_chg {
            out.push_str(&format!(" {pct:+.2}%"));
        }
        if self.tech.limit_up {
            out.push_str("｜昨日涨停（不追高）");
        }
        out.push('\n');
        out.push_str(&format!(
            "  指标: MACD {} 柱{}｜KDJ {}｜SKDJ {}｜RSI {}\n",
            self.tech.macd_cross.unwrap_or("N/A"),
            fmt_opt(self.tech.macd_hist, 2),
            fmt_triplet(self.tech.kdj),
            fmt_triplet(self.tech.skdj),
            fmt_triplet(self.tech.rsi)
        ));
        out.push_str(&format!(
            "  量价: {}｜5日量比 {}｜振幅 {}\n",
            if self.volume_price.tags.is_empty() {
                "N/A（日线<7根）".to_owned()
            } else {
                self.volume_price.tags.join("+")
            },
            fmt_opt(self.volume_price.ratio_5d, 2),
            match self.volume_price.amplitude_pct {
                Some(value) => format!("{value:.1}%"),
                None => "N/A".to_owned(),
            }
        ));
        if self.holding {
            let stop_text = match (self.plan.stop, self.tech.close) {
                (Some(stop), Some(close)) if close <= stop => {
                    format!("成本止损线 {stop:.2}（已跌破，现价 {close:.2}）")
                }
                (Some(stop), _) => format!("止损参考 {stop:.2}"),
                _ => "N/A".to_owned(),
            };
            let mut action_line = format!(
                "  动作: {}｜出: {}｜减仓参考 {}",
                self.plan.position_action.as_deref().unwrap_or("无法判定"),
                stop_text,
                match (self.plan.reduce_low, self.plan.reduce_high) {
                    (Some(low), Some(high)) => format!("{low:.2}~{high:.2}"),
                    _ => "N/A".to_owned(),
                }
            );
            if let Some(locked) = self.locked_shares.filter(|value| *value > 0) {
                action_line.push_str(&format!("｜今日买入锁定 {locked} 股（T+1）"));
            }
            out.push_str(&action_line);
            out.push('\n');
            if let (Some(ma20), Some(close)) = (self.tech.ma20, self.tech.close) {
                let position = if close >= ma20 { "站上" } else { "低于" };
                out.push_str(&format!("  技术参考: MA20 {ma20:.2}（现价{position}）"));
                if let Some(atr) = self.tech.atr14 {
                    out.push_str(&format!(
                        "｜ATR14 {atr:.2}｜1×ATR 参考位 {:.2}｜2×ATR 参考位 {:.2}",
                        close - atr,
                        close - 2.0 * atr
                    ));
                }
                out.push('\n');
            }
        } else {
            let (entry, trigger) = match (self.plan.low, self.plan.high) {
                (Some(low), Some(high)) if (high - low).abs() < 0.005 => {
                    let trigger = self.plan.trigger.unwrap_or(low);
                    (format!("触发位 {trigger:.2}"), String::new())
                }
                (Some(low), Some(high)) => (
                    format!("{low:.2}-{high:.2}"),
                    format!("｜突破确认 {}", fmt_opt(self.plan.trigger, 2)),
                ),
                _ => ("N/A（无活跃提案）".to_owned(), String::new()),
            };
            out.push_str(&format!(
                "  进: {entry}{trigger}｜放弃 {}\n",
                fmt_opt(self.plan.abandon, 2)
            ));
            out.push_str(&format!(
                "  日线状态（{} 收盘）: {}\n",
                empty_dash(&self.tech.as_of),
                self.status
            ));
        }
        if !self.missing.is_empty() {
            out.push_str(&format!("  缺失: {}\n", self.missing.join(",")));
        }
        out
    }
}

fn load_bars(
    conn: &mut SqliteConnection,
    code: &str,
    as_of: &str,
    limit: i64,
) -> Result<Vec<Bar>, String> {
    load_bars_at(conn, code, as_of, limit, true)
}

/// `inclusive=false` 用于 09:25：只取 before 之前已收盘的日线，
/// 避免把当天尚未生成的 K 线当成信号。
fn load_bars_at(
    conn: &mut SqliteConnection,
    code: &str,
    as_of: &str,
    limit: i64,
    inclusive: bool,
) -> Result<Vec<Bar>, String> {
    let sql = if inclusive {
        "SELECT date, high, low, close, volume, pct_chg, volume_ratio FROM stock_daily \
         WHERE code = ?1 AND date <= ?2 AND close IS NOT NULL AND close > 0 \
         ORDER BY date DESC LIMIT ?3"
    } else {
        "SELECT date, high, low, close, volume, pct_chg, volume_ratio FROM stock_daily \
         WHERE code = ?1 AND date < ?2 AND close IS NOT NULL AND close > 0 \
         ORDER BY date DESC LIMIT ?3"
    };
    let rows: Vec<BarRow> = diesel::sql_query(sql)
        .bind::<Text, _>(code)
        .bind::<Text, _>(as_of)
        .bind::<BigInt, _>(limit)
        .load(conn)
        .map_err(|error| format!("日线查询失败 ({code}): {error}"))?;
    Ok(rows
        .into_iter()
        .rev()
        .map(|row| Bar {
            date: row.date,
            high: row.high,
            low: row.low,
            close: row.close,
            volume: row.volume,
            pct_chg: row.pct_chg,
            volume_ratio: row.volume_ratio,
        })
        .collect())
}

/// 09:25 用：截至 `before`（不含）最后一个交易日的日线指标与量价形态。
/// 只提供事实，不参与给价；`as_of` 为空表示日线不足 7 根。
pub(crate) fn daily_tech_before(
    conn: &mut SqliteConnection,
    code: &str,
    before: &str,
) -> Result<(TechView, VolumePriceView), String> {
    let bars = load_bars_at(conn, code, before, 120, false)?;
    Ok((compute_tech(code, &bars), compute_volume_price(code, &bars)))
}

/// 盘中定向监控用：只取量价形态，不重复算 MACD/KDJ。
pub(crate) fn volume_price_before(
    conn: &mut SqliteConnection,
    code: &str,
    before: &str,
) -> Result<VolumePriceView, String> {
    let bars = load_bars_at(conn, code, before, 120, false)?;
    Ok(compute_volume_price(code, &bars))
}

/// P01 等盘前推送用的一行日线摘要（截至 `before`，不含）。
/// 没有任何已入库日线时返回 `None`，不补 0、不冒充盘中指标。
pub fn daily_note_before(
    conn: &mut SqliteConnection,
    code: &str,
    before: &str,
) -> Result<Option<String>, String> {
    let (tech, volume_price) = daily_tech_before(conn, code, before)?;
    Ok(format_daily_note(&tech, &volume_price))
}

fn format_daily_note(tech: &TechView, volume_price: &VolumePriceView) -> Option<String> {
    if tech.as_of.is_empty() {
        return None;
    }
    let date = tech.as_of.get(5..).unwrap_or(tech.as_of.as_str());
    let volume = match volume_price.ratio_5d {
        Some(ratio) => format!("量价 {}｜5日量比 {ratio:.2}", volume_price.tags.join("+")),
        None => "量价 N/A".to_owned(),
    };
    Some(format!(
        "日线({date})：MACD {} 柱{}｜KDJ {}｜SKDJ {}｜RSI {}｜{volume}",
        tech.macd_cross.unwrap_or("N/A"),
        tech.macd_hist
            .map(|value| format!("{value:.2}"))
            .unwrap_or_else(|| "N/A".to_owned()),
        fmt_triplet(tech.kdj),
        fmt_triplet(tech.skdj),
        fmt_triplet(tech.rsi),
    ))
}

fn compute_tech(code: &str, bars: &[Bar]) -> TechView {
    let mut view = TechView::default();
    let Some(last) = bars.last() else {
        return view;
    };
    view.as_of = last.date.clone();
    view.close = Some(last.close);
    view.pct_chg = last.pct_chg;
    view.volume_ratio = last.volume_ratio;
    view.limit_up = last
        .pct_chg
        .is_some_and(|pct| pct >= ticket_regime::limit_up_pct(code));

    if bars.len() >= 20 {
        let sum: f64 = bars[bars.len() - 20..]
            .iter()
            .map(|bar| bar.close)
            .sum();
        view.ma20 = Some(sum / 20.0);
    }
    if bars.len() >= 15 {
        let start = bars.len() - 14;
        let mut sum = 0.0;
        for index in start..bars.len() {
            let previous = bars[index - 1].close;
            let bar = &bars[index];
            sum += (bar.high - bar.low)
                .max((bar.high - previous).abs())
                .max((bar.low - previous).abs());
        }
        view.atr14 = Some(sum / 14.0);
    }

    if bars.len() < 20 {
        return view;
    }
    let closes: Vec<f64> = bars.iter().map(|bar| bar.close).collect();
    let highs: Vec<f64> = bars.iter().map(|bar| bar.high).collect();
    let lows: Vec<f64> = bars.iter().map(|bar| bar.low).collect();

    let macd = calc_macd(&closes, 6, 13, 5);
    if let Some(point) = macd.last() {
        view.macd_dif = Some(point.dif);
        view.macd_dea = Some(point.dea);
        view.macd_hist = Some(point.histogram);
        let dif: Vec<f64> = macd.iter().map(|item| item.dif).collect();
        let dea: Vec<f64> = macd.iter().map(|item| item.dea).collect();
        view.macd_cross = Some(match detect_cross(&dif, &dea, 3) {
            crate::indicators::CrossType::GoldenCross => "金叉",
            crate::indicators::CrossType::DeathCross => "死叉",
            crate::indicators::CrossType::None => "无交叉",
        });
    }

    if let Some(point) = calc_kdj(&highs, &lows, &closes, 9, 3, 3).last() {
        view.kdj = Some((point.k, point.d, point.j));
    }
    if bars.len() >= 40 {
        if let Some(point) = calc_skdj(&highs, &lows, &closes, 40, 5).last() {
            view.skdj = Some((point.k, point.d, point.j));
        }
    }
    if let Some(point) = calc_rsi(&closes).last() {
        view.rsi = Some((point.rsi6, point.rsi12, point.rsi24));
    }
    view
}

/// 放量/缩量阈值（与 trend_analyzer 的 heavy/shrink 同量级：1.8 / 0.8）。
const VOLUME_HEAVY_RATIO: f64 = 1.8;
const VOLUME_SHRINK_RATIO: f64 = 0.8;
/// 放量滞涨：涨跌幅在 ±1% 内，但振幅 ≥3%（有量无价）。
const STALL_PCT: f64 = 1.0;
const STALL_AMPLITUDE_PCT: f64 = 3.0;

/// 日线量价形态：先判涨停（按板分档），再判放量/缩量与涨跌的组合。
/// 阈值是定性观察口径，不是买卖信号；后续要用触发后收益校准。
fn compute_volume_price(code: &str, bars: &[Bar]) -> VolumePriceView {
    let mut view = VolumePriceView::default();
    if bars.len() < 7 {
        return view;
    }
    let last = &bars[bars.len() - 1];
    let prev_close = bars[bars.len() - 2].close;
    if prev_close <= 0.0 {
        return view;
    }
    let avg5 = bars[bars.len() - 6..bars.len() - 1]
        .iter()
        .map(|bar| bar.volume)
        .sum::<f64>()
        / 5.0;
    if avg5 <= 0.0 {
        return view;
    }
    let ratio = last.volume / avg5;
    let pct = last
        .pct_chg
        .unwrap_or_else(|| (last.close - prev_close) / prev_close * 100.0);
    let amplitude = (last.high - last.low) / prev_close * 100.0;
    view.ratio_5d = Some(ratio);
    view.amplitude_pct = Some(amplitude);

    let limit_up = pct >= ticket_regime::limit_up_pct(code);
    if limit_up && ratio <= VOLUME_SHRINK_RATIO {
        view.tags.push("缩量涨停");
    } else if limit_up && ratio >= VOLUME_HEAVY_RATIO {
        view.tags.push("放量涨停");
    } else if limit_up {
        view.tags.push("涨停（量能中性）");
    } else if ratio >= VOLUME_HEAVY_RATIO
        && pct.abs() <= STALL_PCT
        && amplitude >= STALL_AMPLITUDE_PCT
    {
        view.tags.push("放量滞涨");
    } else if ratio >= VOLUME_HEAVY_RATIO && pct > 0.0 {
        view.tags.push("放量上涨");
    } else if ratio >= VOLUME_HEAVY_RATIO && pct < 0.0 {
        view.tags.push("放量下跌");
    } else if ratio <= VOLUME_SHRINK_RATIO && pct < 0.0 {
        view.tags.push("缩量回调");
    } else if ratio <= VOLUME_SHRINK_RATIO && pct > 0.0 {
        view.tags.push("缩量上涨");
    } else {
        view.tags.push("量价正常");
    }
    view
}

fn tech_missing(tech: &TechView, bar_count: usize) -> Vec<String> {
    let mut missing = Vec::new();
    if bar_count == 0 {
        missing.push("daily_bars".to_owned());
        return missing;
    }
    if bar_count < 20 {
        missing.push("daily_bars<20".to_owned());
    }
    if bar_count < 40 {
        missing.push("skdj_bars<40".to_owned());
    }
    if tech.volume_ratio.is_none() {
        missing.push("volume_ratio".to_owned());
    }
    if tech.ma20.is_none() {
        missing.push("ma20_bars<20".to_owned());
    }
    if tech.atr14.is_none() {
        missing.push("atr14_bars<15".to_owned());
    }
    missing
}

pub(crate) fn latest_decision(
    conn: &mut SqliteConnection,
    ticket_id: i64,
) -> Result<Option<DecisionContext>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        context_json: String,
    }
    let row: Option<Row> = diesel::sql_query(
        "SELECT context_json FROM ticket_decision WHERE ticket_id = ?1 \
         ORDER BY as_of DESC, created_at DESC, decision_id DESC LIMIT 1",
    )
    .bind::<BigInt, _>(ticket_id)
    .get_result(conn)
    .optional()
    .map_err(|error| format!("ticket_decision 查询失败: {error}"))?;
    match row {
        Some(row) => serde_json::from_str(&row.context_json)
            .map(Some)
            .map_err(|error| format!("ticket_decision context 解析失败: {error}")),
        None => Ok(None),
    }
}

fn latest_status(conn: &mut SqliteConnection, ticket_id: i64) -> Result<Option<String>, String> {
    crate::monitor::ticket_decision::latest_proposal_status(conn, ticket_id)
        .map(|status| status.map(|value| value.as_str().to_owned()))
}

fn classify(close: Option<f64>, plan: &PlanView, holding: bool) -> String {
    let Some(close) = close else {
        return "无日线（无法判定）".to_owned();
    };
    if holding {
        return plan
            .position_action
            .clone()
            .unwrap_or_else(|| "持仓（成本缺失）".to_owned());
    }
    if plan.decision_id.is_none() {
        return "无活跃提案".to_owned();
    }
    if let Some(abandon) = plan.abandon {
        if close <= abandon {
            return format!("已跌破放弃价 {abandon:.2}");
        }
    }
    if plan.entry_mode.as_deref() == Some("breakout") {
        if let Some(trigger) = plan.trigger {
            if close >= trigger {
                return format!("已触发（站上 {trigger:.2}）");
            }
        }
    }
    match (plan.low, plan.high) {
        (Some(low), Some(high)) if close >= low && close <= high => "进入参考区间".to_owned(),
        (_, Some(high)) if close > high => "高于区间（不追）".to_owned(),
        (Some(low), _) if close < low => "低于区间（等待）".to_owned(),
        _ => "有提案但价位缺失".to_owned(),
    }
}

fn fmt_opt(value: Option<f64>, digits: usize) -> String {
    value
        .map(|item| format!("{item:.digits$}"))
        .unwrap_or_else(|| "N/A".to_owned())
}

fn fmt_triplet(value: Option<(f64, f64, f64)>) -> String {
    value
        .map(|(a, b, c)| format!("{a:.0}/{b:.0}/{c:.0}"))
        .unwrap_or_else(|| "N/A".to_owned())
}

fn empty_dash(value: &str) -> &str {
    if value.is_empty() {
        "N/A"
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(entry_mode: &str, low: f64, high: f64, trigger: Option<f64>, abandon: f64) -> PlanView {
        PlanView {
            decision_id: Some("d1".to_owned()),
            proposal_status: Some("open".to_owned()),
            entry_mode: Some(entry_mode.to_owned()),
            low: Some(low),
            high: Some(high),
            trigger,
            abandon: Some(abandon),
            ..Default::default()
        }
    }

    #[test]
    fn classify_covers_range_and_abandon() {
        let pullback = plan("pullback", 10.0, 10.5, None, 9.0);
        assert_eq!(classify(Some(10.2), &pullback, false), "进入参考区间");
        assert_eq!(classify(Some(10.8), &pullback, false), "高于区间（不追）");
        assert_eq!(classify(Some(9.8), &pullback, false), "低于区间（等待）");
        assert_eq!(classify(Some(8.9), &pullback, false), "已跌破放弃价 9.00");
        let breakout = plan("breakout", 10.0, 10.5, Some(10.6), 9.0);
        assert_eq!(
            classify(Some(10.7), &breakout, false),
            "已触发（站上 10.60）"
        );
        assert_eq!(classify(None, &pullback, false), "无日线（无法判定）");
        assert_eq!(
            classify(Some(10.2), &PlanView::default(), false),
            "无活跃提案"
        );
    }

    #[test]
    fn daily_note_formats_date_and_indicators() {
        let tech = TechView {
            as_of: "2026-11-13".to_owned(),
            macd_cross: Some("金叉"),
            macd_hist: Some(0.05),
            kdj: Some((70.0, 60.0, 90.0)),
            skdj: Some((40.0, 35.0, 45.0)),
            rsi: Some((62.0, 55.0, 48.0)),
            ..Default::default()
        };
        let volume_price = VolumePriceView {
            ratio_5d: Some(2.5),
            tags: vec!["放量上涨"],
            ..Default::default()
        };
        assert_eq!(
            format_daily_note(&tech, &volume_price).as_deref(),
            Some(
                "日线(11-13)：MACD 金叉 柱0.05｜KDJ 70/60/90｜SKDJ 40/35/45｜RSI 62/55/48｜量价 放量上涨｜5日量比 2.50"
            )
        );
        assert!(format_daily_note(&TechView::default(), &VolumePriceView::default()).is_none());
    }

    #[test]
    fn compute_tech_marks_limit_up_and_indicators() {
        // 45 根单调上涨日线，涨幅足够触发 10cm 涨停判定。
        let bars: Vec<Bar> = (0..45)
            .map(|index| Bar {
                date: format!("2026-09-{:02}", index + 1),
                high: 10.0 + index as f64 * 1.0,
                low: 9.5 + index as f64 * 1.0,
                close: 10.0 + index as f64 * 1.0,
                volume: 1000.0,
                pct_chg: Some(10.0),
                volume_ratio: Some(1.2),
            })
            .collect();
        let tech = compute_tech("600000", &bars);
        assert_eq!(tech.as_of, "2026-09-45");
        assert!(tech.macd_dif.is_some());
        assert!(tech.kdj.is_some());
        assert!(tech.skdj.is_some());
        assert!(tech.rsi.is_some());
        assert!(tech.ma20.is_some());
        assert!(tech.atr14.is_some());
        assert!(tech.limit_up);
        assert_eq!(tech.volume_ratio, Some(1.2));
    }

    fn flat_bar(volume: f64) -> Bar {
        Bar {
            date: "2026-11-11".to_owned(),
            high: 10.1,
            low: 9.9,
            close: 10.0,
            volume,
            pct_chg: Some(0.0),
            volume_ratio: Some(1.0),
        }
    }

    #[test]
    fn volume_price_recognizes_shrink_limit_up_and_heavy_stall() {
        // 前 6 根各 1000 量；最后一根 600 量 +10% → 缩量涨停（600000 为 10cm 板）。
        let mut bars: Vec<Bar> = (0..6).map(|_| flat_bar(1000.0)).collect();
        bars.push(Bar {
            date: "2026-11-12".to_owned(),
            high: 11.0,
            low: 10.0,
            close: 11.0,
            volume: 600.0,
            pct_chg: Some(10.0),
            volume_ratio: Some(0.6),
        });
        let view = compute_volume_price("600000", &bars);
        assert_eq!(view.tags, vec!["缩量涨停"]);
        assert!((view.ratio_5d.expect("ratio") - 0.6).abs() < 1e-9);

        // 最后一根 3000 量、+0.3%、振幅 5% → 放量滞涨。
        let mut stalled: Vec<Bar> = (0..6).map(|_| flat_bar(1000.0)).collect();
        stalled.push(Bar {
            date: "2026-11-12".to_owned(),
            high: 10.5,
            low: 10.0,
            close: 10.03,
            volume: 3000.0,
            pct_chg: Some(0.3),
            volume_ratio: Some(3.0),
        });
        let view = compute_volume_price("600000", &stalled);
        assert_eq!(view.tags, vec!["放量滞涨"]);
        assert!((view.amplitude_pct.expect("amplitude") - 5.0).abs() < 1e-9);
    }
}
