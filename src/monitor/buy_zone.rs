//! P2 可买价（按票, 2026-10-17）。
//!
//! ## 输入（全部来自已有数据，不引入新的外部依赖）
//!
//! - `stock_daily`：最近收盘、ma5/ma10/ma20、volume_ratio、最近 30 天 high/low
//! - `trading_fact_snapshot`：当日涨跌停价（仅观测快照日有值，可能为 NULL）
//!
//! ## 输出结构
//!
//! 每只票三句话：**现在什么位置** → **在什么区间买** → **什么情况放弃**。
//!
//! ## 判定规则（写死在代码里，便于复核）
//!
//! 趋势：ma5>ma10>ma20 多头 / ma5<ma10<ma20 空头 / 其余纠缠 / 缺值未知
//!
//! 买点区间：
//! - 多头：回踩 ma10 附近 `[ma10*0.99, ma10*1.01]`
//! - 空头或纠缠：`[close*0.98, min(close*1.01, ma5)]`，并要求"站上 ma5 且量比>1.2"
//! - 已跌破 ma20 且为空头排列：不给买点区间（只给"观察"）
//!
//! 放弃价：
//! - 多头：`max(ma20, 20日低)` 的 0.99 倍（破 ma20 或破平台都算错）
//! - 空头/纠缠：`20日低 * 0.99`
//! - 若有跌停价且低于上述值，取跌停价下方（跌停是理论底，破不了更不该追）
//!
//! ## 为什么不给"必买价"
//!
//! 技术指标只能给出**概率性的位置判断**，任何单一价位都不是保证。这里给的是
//! "在这个区间内、且满足触发条件时才有意义"，而不是一个精确点位。

use diesel::prelude::*;
use diesel::sql_types::{Double, Nullable, Text};

const LOOKBACK_DAYS: i64 = 30;
const SUPPORT_WINDOW: usize = 20;
const VOLUME_CONFIRM: f64 = 1.2;

/// 三道超买门：任一命中就不给买点（2026-10-17 由 601811 三连板暴露）。
///
/// 601811 当时 close=28.14 / ma5=24.22（偏离 +16.2%）/ 近 5 日 +34.5% / 三连板封死，
/// 原规则仍给"回踩 ma10 22.1 买" —— 等于要求等一个 22% 的崩盘，或去涨停板上追。
const MAX_DEVIATION_FROM_MA5: f64 = 0.10;
const MAX_GAIN_5D: f64 = 0.25;
const MIN_LIMIT_UP_STREAK_TO_SKIP: usize = 3;
/// 涨停判定阈值（pct_chg 是百分数）。2026-10-23 批 1.2 改按板分档：
/// 20cm 板（300/301/688/689）19.5%、其余 9.85%，与 `ticket_regime::limit_up_pct` 同源。
/// 原 9.8 会把 20cm 板 +10% 的普通上涨计成涨停，连板数虚高 → 超买门误杀。
fn limit_up_threshold(code: &str) -> f64 {
    crate::monitor::ticket_regime::limit_up_pct(code)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trend {
    Bullish,
    Bearish,
    Tangled,
    Unknown,
}

impl Trend {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Trend::Bullish => "多头排列",
            Trend::Bearish => "空头排列",
            Trend::Tangled => "均线纠缠",
            Trend::Unknown => "均线不全",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BuyZone {
    pub code: String,
    /// 股票名称。查不到就是 None —— 不编造（BR-222），显示时只出代码。
    pub name: Option<String>,
    pub as_of: String,
    pub close: f64,
    pub ma5: Option<f64>,
    pub ma10: Option<f64>,
    pub ma20: Option<f64>,
    pub volume_ratio: Option<f64>,
    pub low_20d: Option<f64>,
    pub high_20d: Option<f64>,
    pub limit_up: Option<f64>,
    pub limit_down: Option<f64>,
    pub trend: Trend,
    pub entry_low: Option<f64>,
    pub entry_high: Option<f64>,
    pub abandon: Option<f64>,
    pub trigger: String,
    /// 是否因为"已在 20 日低附近"而不给买点（区别于"数据不足"）
    pub at_support: bool,
    /// 是否因为超买/异动（连板 / 近5日涨幅 / 偏离 ma5）而不给买点
    pub overheated: bool,
}

fn classify(ma5: Option<f64>, ma10: Option<f64>, ma20: Option<f64>) -> Trend {
    match (ma5, ma10, ma20) {
        (Some(a), Some(b), Some(c)) if a > b && b > c => Trend::Bullish,
        (Some(a), Some(b), Some(c)) if a < b && b < c => Trend::Bearish,
        (Some(_), Some(_), Some(_)) => Trend::Tangled,
        _ => Trend::Unknown,
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// 查股票名称：按可信度依次查四张表，都没有就返回 None（不编造，BR-222）。
///
/// 为什么不是一个统一的股票主表：库里没有这张表。名称只散落在
/// 持仓、持仓快照、推送记录、催化剂观察里，所以按"离当前状态最近"的顺序查。
pub fn resolve_name(conn: &mut SqliteConnection, code: &str) -> Option<String> {
    #[derive(QueryableByName)]
    struct NameRow {
        #[diesel(sql_type = Text)]
        name: String,
    }
    const SOURCES: [&str; 4] = [
        "stock_position",
        "user_position_snapshot_item",
        "pushed_stocks",
        "catalyst_watchlist_daily",
    ];
    for table in SOURCES {
        let sql = format!(
            "SELECT name FROM {table} \
             WHERE code = ? AND name IS NOT NULL AND name <> '' \
             ORDER BY rowid DESC LIMIT 1"
        );
        if let Ok(row) = diesel::sql_query(sql)
            .bind::<Text, _>(code)
            .get_result::<NameRow>(conn)
        {
            return Some(row.name);
        }
    }
    // 观察池的新票不在上述四张表里（没持仓过、没推送过），但 F10 建档
    // 正文头部固定带名称：`☆经营分析☆ ◇000701 厦门信达 更新日期：…◇`。
    // 这是权威数据源自带的字段，不是推导值（BR-222 允许）。
    name_from_f10(conn, code)
}

/// 从 `stock_f10_profile.content` 头部解析股票名称。
///
/// 格式固定为 `◇{code} {name} 更新日期：…◇`；解析不出来就返回 None，
/// 由调用方退回「只显代码」。
fn name_from_f10(conn: &mut SqliteConnection, code: &str) -> Option<String> {
    #[derive(QueryableByName)]
    struct ContentRow {
        #[diesel(sql_type = Text)]
        content: String,
    }
    let row: ContentRow = diesel::sql_query(
        "SELECT content FROM stock_f10_profile WHERE code = ? \
         ORDER BY fetched_at DESC LIMIT 1",
    )
    .bind::<Text, _>(code)
    .get_result(conn)
    .ok()?;

    let marker = format!("◇{code} ");
    let start = row.content.find(&marker)? + marker.len();
    let rest = &row.content[start..];
    let end = rest.find(" 更新日期").or_else(|| rest.find('◇'))?;
    let name = rest[..end].trim();
    if name.is_empty() || name.len() > 12 {
        // 超过 12 字基本是解析错行（F10 正文里还有别的 ◇ 标记），宁可不填
        return None;
    }
    Some(name.to_string())
}

pub fn compute(
    conn: &mut SqliteConnection,
    code: &str,
    as_of: &str,
) -> Result<Option<BuyZone>, String> {
    #[derive(QueryableByName)]
    struct BarRow {
        #[diesel(sql_type = Text)]
        date: String,
        #[diesel(sql_type = Nullable<Double>)]
        close: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        high: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        low: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        ma5: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        ma10: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        ma20: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        volume_ratio: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        pct_chg: Option<f64>,
    }

    let bars: Vec<BarRow> = diesel::sql_query(
        "SELECT date, close, high, low, ma5, ma10, ma20, volume_ratio, pct_chg \
         FROM stock_daily WHERE code = ? AND date <= ? \
         ORDER BY date DESC LIMIT ?",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(as_of)
    .bind::<diesel::sql_types::BigInt, _>(LOOKBACK_DAYS)
    .load(conn)
    .map_err(|e| format!("{code} 读取日线失败: {e}"))?;

    let latest = match bars.first() {
        Some(row) => row,
        None => return Ok(None),
    };
    let close = match latest.close {
        Some(value) => value,
        None => return Ok(None),
    };

    let window = &bars[..bars.len().min(SUPPORT_WINDOW)];
    let low_20d =
        window
            .iter()
            .filter_map(|row| row.low)
            .fold(None::<f64>, |acc, value| match acc {
                Some(current) => Some(current.min(value)),
                None => Some(value),
            });
    let high_20d =
        window
            .iter()
            .filter_map(|row| row.high)
            .fold(None::<f64>, |acc, value| match acc {
                Some(current) => Some(current.max(value)),
                None => Some(value),
            });

    #[derive(QueryableByName)]
    struct BandRow {
        #[diesel(sql_type = Nullable<Double>)]
        limit_up: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        limit_down: Option<f64>,
    }
    let band: Option<BandRow> = diesel::sql_query(
        "SELECT limit_up, limit_down FROM trading_fact_snapshot \
         WHERE code = ? AND trade_date = ?",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(as_of)
    .get_result(conn)
    .ok();

    let trend = classify(latest.ma5, latest.ma10, latest.ma20);

    // ---- 三道超买门（2026-10-17 由 601811 三连板暴露）----
    // 连板：从最新往前数连续 pct_chg >= 阈值的天数
    let limit_up_gate = limit_up_threshold(code);
    let limit_up_streak = bars
        .iter()
        .take_while(|row| {
            row.pct_chg
                .map(|value| value >= limit_up_gate)
                .unwrap_or(false)
        })
        .count();
    // 近 5 个交易日涨幅（bars[0] 为最新，bars[5] 为 5 个交易日前）
    let gain_5d = if bars.len() > 5 {
        match (bars[0].close, bars[5].close) {
            (Some(latest_close), Some(base_close)) if base_close > 0.0 => {
                Some((latest_close - base_close) / base_close)
            }
            _ => None,
        }
    } else {
        None
    };
    // 现价相对 ma5 的偏离度
    let deviation = latest.ma5.and_then(|ma5| {
        if ma5 > 0.0 {
            Some((close - ma5) / ma5)
        } else {
            None
        }
    });

    let mut overheat: Option<String> = None;
    if limit_up_streak >= MIN_LIMIT_UP_STREAK_TO_SKIP {
        overheat = Some(format!("连续 {limit_up_streak} 个涨停（封板买不进）"));
    } else if let Some(gain) = gain_5d {
        if gain > MAX_GAIN_5D {
            overheat = Some(format!("近 5 日涨幅 {:.1}% 过大", gain * 100.0));
        }
    }
    if overheat.is_none() {
        if let Some(dev) = deviation {
            if dev > MAX_DEVIATION_FROM_MA5 {
                overheat = Some(format!("现价偏离 ma5 {:.1}%", dev * 100.0));
            }
        }
    }

    let mut entry_low = None;
    let mut entry_high = None;
    let mut abandon = None;
    let mut trigger = String::new();
    let mut at_support = false;

    match trend {
        Trend::Bullish => {
            if let Some(ma10) = latest.ma10 {
                entry_low = Some(round2(ma10 * 0.99));
                entry_high = Some(round2(ma10 * 1.01));
            }
            let floor = match (latest.ma20, low_20d) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };
            abandon = floor.map(|value| round2(value * 0.99));
            trigger = format!(
                "回踩 ma10 {} 附近可买；确认条件：量比>{VOLUME_CONFIRM:.1}",
                latest.ma10.map(round2).unwrap_or_default()
            );
        }
        Trend::Bearish | Trend::Tangled => {
            // 空头/纠缠：不追高。上沿取 min(close*1.01, ma5) —— ma5 在头顶时不要越过它。
            let upper = match latest.ma5 {
                Some(ma5) => (close * 1.01).min(ma5),
                None => close * 1.01,
            };
            let floor_abandon = low_20d.map(|value| round2(value * 0.99));

            // 已在 20 日低附近时不给买点：entry_low = close*0.98 会低于 abandon = low*0.99，
            // 等于"买了就触发止损"（2026-10-17 实测 002131: 买 3.98-4.1 / 放弃 3.99）。
            // 此时只说明"跌破多少放弃"，不在支撑位上怂恿买入。
            // 浮点边界：600703 收盘 10.18 / 20日低 9.98，9.98*1.02 = 10.1796，
            // 差 0.0004 远超浮点表示误差（1e-9 无效，2026-10-17 实测）。
            // 改为按两位小数比较：round2(10.1796) = 10.18，10.18 <= 10.18 成立。
            at_support = low_20d
                .map(|value| close <= round2(value * 1.02))
                .unwrap_or(false);

            if at_support {
                abandon = floor_abandon;
                trigger = format!(
                    "已在 20 日低点 {} 附近，此处不买；跌破 {} 视为支撑失守",
                    low_20d.map(round2).unwrap_or_default(),
                    abandon.map(round2).unwrap_or_default()
                );
            } else {
                // 下沿不低于 20 日低（否则区间与放弃价交叉）
                let low = match low_20d {
                    Some(value) => (close * 0.98).max(value * 1.001),
                    None => close * 0.98,
                };
                entry_low = Some(round2(low.min(upper)));
                entry_high = Some(round2(upper));
                abandon = floor_abandon;
                trigger = format!(
                    "不追高；站上 ma5 {} 且量比>{VOLUME_CONFIRM:.1} 才算有效",
                    latest.ma5.map(round2).unwrap_or_default()
                );
            }
        }
        Trend::Unknown => {
            trigger = "均线数据不足（HistoricalBars 历史 < 20 个交易日），不给买点".to_string();
        }
    }

    // 三道超买门任一命中：覆盖 trend 分支给出的买点
    if let Some(reason) = overheat.as_deref() {
        entry_low = None;
        entry_high = None;
        at_support = false;
        trigger = format!("{reason}，不参与；等回踩或开板后再看");
    }

    // 跌停价是理论底：放弃价不应低于它（否则永远触发不了），只作下限提示
    let limit_down = band.as_ref().and_then(|row| row.limit_down);
    if let (Some(value), Some(floor)) = (abandon, limit_down) {
        if value <= floor {
            abandon = Some(round2(floor));
            trigger.push_str(&format!(
                "；注意当日跌停价 {}，放弃价已按跌停价收敛",
                round2(floor)
            ));
        }
    }

    Ok(Some(BuyZone {
        code: code.to_string(),
        name: resolve_name(conn, code),
        as_of: latest.date.clone(),
        close: round2(close),
        ma5: latest.ma5.map(round2),
        ma10: latest.ma10.map(round2),
        ma20: latest.ma20.map(round2),
        volume_ratio: latest
            .volume_ratio
            .map(|value| (value * 100.0).round() / 100.0),
        low_20d: low_20d.map(round2),
        high_20d: high_20d.map(round2),
        limit_up: band.as_ref().and_then(|row| row.limit_up),
        limit_down,
        trend,
        entry_low,
        entry_high,
        abandon,
        trigger,
        at_support,
        overheated: overheat.is_some(),
    }))
}

impl BuyZone {
    /// 单票一段（用户要求"按票"，信息完整不聚合）。
    /// 展示用标签：代码 + 名称。查不到名称时只出代码（不编造）。
    pub fn display_name(&self) -> String {
        match self.name.as_deref() {
            Some(name) if !name.is_empty() => format!("{} {}", self.code, name),
            _ => self.code.clone(),
        }
    }

    pub fn render(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "{}  收盘 {}（{}）",
            self.display_name(),
            self.close,
            self.as_of
        ));

        let ma_text = match (self.ma5, self.ma10, self.ma20) {
            (Some(a), Some(b), Some(c)) => format!("ma5 {a} / ma10 {b} / ma20 {c}"),
            _ => "均线不全".to_string(),
        };
        lines.push(format!("  位置：{} · {}", self.trend.label(), ma_text));

        let band_text = match (self.low_20d, self.high_20d) {
            (Some(low), Some(high)) => format!("20日区间 {low}-{high}"),
            _ => "20日区间未知".to_string(),
        };
        let volume_text = match self.volume_ratio {
            Some(value) => format!("量比 {value}"),
            None => "量比未知".to_string(),
        };
        lines.push(format!("  参考：{band_text} · {volume_text}"));

        match (self.entry_low, self.entry_high) {
            (Some(low), Some(high)) => lines.push(format!("  买点：{low}-{high}")),
            _ if self.overheated => lines.push("  买点：不给（超买/异动，不参与）".to_string()),
            _ if self.at_support => {
                lines.push("  买点：不给（已在 20 日低点附近，不追）".to_string())
            }
            _ => lines.push("  买点：不给（均线数据不足）".to_string()),
        }
        if !self.trigger.is_empty() {
            lines.push(format!("  条件：{}", self.trigger));
        }
        match self.abandon {
            Some(value) => lines.push(format!("  放弃：跌破 {value}")),
            None => lines.push("  放弃：无法判定".to_string()),
        }
        if let (Some(up), Some(down)) = (self.limit_up, self.limit_down) {
            lines.push(format!("  当日价带：涨停 {up} / 跌停 {down}"));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_detects_bullish_and_bearish() {
        assert_eq!(classify(Some(3.0), Some(2.0), Some(1.0)), Trend::Bullish);
        assert_eq!(classify(Some(1.0), Some(2.0), Some(3.0)), Trend::Bearish);
        assert_eq!(classify(Some(2.0), Some(1.0), Some(3.0)), Trend::Tangled);
        assert_eq!(classify(Some(2.0), Some(1.0), None), Trend::Unknown);
    }

    #[test]
    fn round2_keeps_two_decimals() {
        assert_eq!(round2(10.185), 10.19);
        assert_eq!(round2(10.184), 10.18);
    }
}
