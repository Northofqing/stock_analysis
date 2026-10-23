//! 观察池选股表现（按开票批次统计 N 日收益与胜率）。
//!
//! ## 为什么要有这个
//!
//! 「选出来的票后来涨没涨」是唯一能判断选股规则好不好用的证据。
//! 板块轮动、CATALYST 阈值、每板块取几只——这些参数光靠盘感调不出来，
//! 得有批次级的胜率和均值。
//!
//! ## 口径（2026-10-23 批 1.6 / 4.1 / 4.2 修订）
//!
//! - **基准价按来源分**：`p01_preopen` → `open_date` **开盘价**（09:00 开票、当日可买）；
//!   其余（`board_rotation` / `news_catalyst` / `money_flow` / `holding`）→ `open_date`
//!   **收盘价**（盘后开票，次日才能买）。原实现一律用收盘价，给 P01 票系统性晚算一天。
//! - N 日价 = 交易日历上往后第 N 个交易日的收盘；交易日历取 `board_ranking_daily`
//!   的 distinct trade_date（盘后必写）。
//! - 收益 = (N日价 / 基准价 - 1) × 100
//! - **复盘窗按分型**（D2）：情绪型 D+1/D+3、趋势型 D+5/D+10；CLI 传 `--days`
//!   时按显式窗口统计（临时对账用）。
//! - **市场基准**（4.2）：同窗全市场等权收益。参与样本 < 800 只时标「不可信」——
//!   2026-10 的日线覆盖收缩到两位数，这个标记必须显示，不能拿 50 只冒充全市场。
//!
//! 还没走到第 N 日的批次显示「—」，不补 0（BR-222）。
//!
//! 调用方：[`crate::bin::watch_pool_perf`] 手动跑，`monitor` 16:05 调度自动跑并落盘。

use std::collections::HashMap;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Text};

use crate::monitor::buy_zone::resolve_name;
use crate::monitor::ticket_regime::Regime;

/// 市场基准的最小可信样本数（4.2）。
pub const MIN_MARKET_SAMPLE: usize = 800;

#[derive(QueryableByName)]
struct TicketRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    open_date: String,
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    source: String,
}

#[derive(QueryableByName)]
struct BarRow {
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    date: String,
    #[diesel(sql_type = Double)]
    close: f64,
    #[diesel(sql_type = diesel::sql_types::Nullable<Double>)]
    open: Option<f64>,
}

#[derive(QueryableByName)]
struct DateRow {
    #[diesel(sql_type = Text)]
    trade_date: String,
}

#[derive(QueryableByName)]
struct RegimeRow {
    #[diesel(sql_type = diesel::sql_types::Nullable<Text>)]
    summary: Option<String>,
}

struct BatchTicket {
    open_date: String,
    code: String,
    name: String,
    source: String,
    regime: Regime,
    base_price: Option<f64>,
    /// 下标 = 交易日间隔 - 1（`returns[i]` 即 D+(i+1)）
    returns: Vec<Option<f64>>,
}

/// 统计参数。
pub struct PerfOptions {
    /// 显式观察周期（交易日）。`per_regime=true` 时按分型取窗，此字段不生效。
    pub periods: Vec<usize>,
    /// 只看这个日期（含）之后开的票；None = 全部。
    pub since: Option<String>,
    /// 按分型分窗输出（monitor 调度 true；CLI `--days` 用 false）。
    pub per_regime: bool,
}

impl Default for PerfOptions {
    fn default() -> Self {
        Self {
            periods: vec![1, 3, 5],
            since: None,
            per_regime: true,
        }
    }
}

/// 该 ticket 的最新分型；没有分型事件时按趋势型统计
/// （趋势型窗口更长，晚给结论比错给结论安全）。
fn latest_regime(conn: &mut SqliteConnection, ticket_id: i64) -> Regime {
    let summary = diesel::sql_query(
        "SELECT summary FROM ticket_event \
          WHERE ticket_id = ? AND event_kind = 'regime' \
          ORDER BY event_at DESC, id DESC LIMIT 1",
    )
    .bind::<BigInt, _>(ticket_id)
    .get_result::<RegimeRow>(conn)
    .ok()
    .and_then(|row| row.summary);
    summary
        .and_then(|text| text.split('|').next().and_then(Regime::parse))
        .unwrap_or(Regime::Trend)
}

/// 全市场等权收益：`from` → `to` 两日都有收盘价的票，收益均值。
/// 返回 (均值%, 样本数)。
fn market_return(
    by_date: &HashMap<String, HashMap<String, f64>>,
    from: &str,
    to: &str,
) -> Option<(f64, usize)> {
    let start = by_date.get(from)?;
    let end = by_date.get(to)?;
    let mut sum = 0.0;
    let mut count = 0usize;
    for (code, base) in start {
        if *base <= 0.0 {
            continue;
        }
        if let Some(target) = end.get(code) {
            sum += target / base - 1.0;
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }
    Some((sum / count as f64 * 100.0, count))
}

/// 生成统计文本。没有开票记录时返回提示串（不是错误）。
pub fn render(conn: &mut SqliteConnection, options: &PerfOptions) -> Result<String, String> {
    // 交易日历：升序
    let calendar: Vec<String> = diesel::sql_query(
        "SELECT DISTINCT trade_date FROM board_ranking_daily ORDER BY trade_date",
    )
    .load::<DateRow>(conn)
    .map_err(|error| error.to_string())?
    .into_iter()
    .map(|row| row.trade_date)
    .collect();
    if calendar.is_empty() {
        return Ok("板块榜为空，没有交易日历可用\n".to_string());
    }
    let day_index: HashMap<&str, usize> = calendar
        .iter()
        .enumerate()
        .map(|(index, date)| (date.as_str(), index))
        .collect();

    // 五个来源都统计：P01 / 资金流 / 持仓同样是被推荐的票，漏掉等于只看一半样本。
    let base_sql = "SELECT id, open_date, code, source FROM ticket_lifecycle \
         WHERE source IN ('board_rotation','news_catalyst','p01_preopen','money_flow','holding')";
    let tickets: Vec<TicketRow> = match &options.since {
        Some(since_date) => diesel::sql_query(format!(
            "{base_sql} AND open_date >= ? GROUP BY open_date, code \
             ORDER BY open_date DESC, code"
        ))
        .bind::<Text, _>(since_date.clone())
        .load(conn),
        None => diesel::sql_query(format!(
            "{base_sql} GROUP BY open_date, code ORDER BY open_date DESC, code"
        ))
        .load(conn),
    }
    .map_err(|error| error.to_string())?;

    // 一次性取回全部日线：既算个股收益，也算市场基准。
    let mut closes: HashMap<(String, String), f64> = HashMap::new();
    let mut opens: HashMap<(String, String), f64> = HashMap::new();
    let mut by_date: HashMap<String, HashMap<String, f64>> = HashMap::new();
    for row in diesel::sql_query("SELECT code, date, close, open FROM stock_daily WHERE close > 0")
        .load::<BarRow>(conn)
        .map_err(|error| error.to_string())?
    {
        closes.insert((row.code.clone(), row.date.clone()), row.close);
        if let Some(open) = row.open {
            opens.insert((row.code.clone(), row.date.clone()), open);
        }
        by_date
            .entry(row.date.clone())
            .or_default()
            .insert(row.code.clone(), row.close);
    }

    let max_period = options.periods.iter().copied().max().unwrap_or(10).max(10);
    let mut batches: Vec<BatchTicket> = Vec::new();
    for ticket in tickets {
        let name = resolve_name(conn, &ticket.code).unwrap_or_default();
        let regime = latest_regime(conn, ticket.id);
        // 4.1：P01 09:00 开票、当日可买 → 基准取开盘价；其余盘后开票 → 收盘价。
        let base_price = if ticket.source == "p01_preopen" {
            opens
                .get(&(ticket.code.clone(), ticket.open_date.clone()))
                .copied()
                .or_else(|| {
                    closes
                        .get(&(ticket.code.clone(), ticket.open_date.clone()))
                        .copied()
                })
        } else {
            closes
                .get(&(ticket.code.clone(), ticket.open_date.clone()))
                .copied()
        };
        let start = day_index.get(ticket.open_date.as_str()).copied();

        // 统一按最大窗口算一遍，分组输出时按各自窗口取下标。
        let returns: Vec<Option<f64>> = (1..=max_period)
            .map(|period| {
                let base = base_price?;
                let start = start?;
                let target_date = calendar.get(start + period)?;
                let target = closes.get(&(ticket.code.clone(), target_date.clone()))?;
                Some((target / base - 1.0) * 100.0)
            })
            .collect();

        batches.push(BatchTicket {
            open_date: ticket.open_date,
            code: ticket.code,
            name,
            source: ticket.source,
            regime,
            base_price,
            returns,
        });
    }

    if batches.is_empty() {
        return Ok("没有观察池开票记录\n".to_string());
    }

    let mut out = String::new();
    // 按分型分组输出（1.6）；CLI 显式给窗口时只出一组。
    let groups: Vec<(Option<Regime>, Vec<usize>)> = if options.per_regime {
        vec![
            (
                Some(Regime::Emotion),
                Regime::Emotion.perf_periods().to_vec(),
            ),
            (Some(Regime::Trend), Regime::Trend.perf_periods().to_vec()),
        ]
    } else {
        vec![(None, options.periods.clone())]
    };

    for (group, periods) in groups {
        let rows: Vec<&BatchTicket> = batches
            .iter()
            .filter(|row| group.map(|value| value == row.regime).unwrap_or(true))
            .collect();
        let title = group
            .map(|value| format!("{}(窗 {periods:?})", value.label()))
            .unwrap_or_else(|| "全部来源（显式窗口）".to_string());
        out.push_str(&format!("\n=== {title} ===\n"));
        if rows.is_empty() {
            out.push_str("  暂无样本\n");
            continue;
        }
        render_group(&mut out, &rows, &periods, &calendar, &day_index, &by_date);
    }

    out.push_str(&format!(
        "\n口径：基准价 P01=开盘价、其余=收盘价；市场基准样本 < {MIN_MARKET_SAMPLE} 只标「不可信」。假设执行、非真实收益。\n"
    ));
    Ok(out)
}

fn render_group(
    out: &mut String,
    rows: &[&BatchTicket],
    periods: &[usize],
    calendar: &[String],
    day_index: &HashMap<&str, usize>,
    by_date: &HashMap<String, HashMap<String, f64>>,
) {
    let fmt_return = |value: Option<f64>| match value {
        Some(pct) => format!("{pct:+7.2}%"),
        None => "      —".to_string(),
    };

    let mut current_date = String::new();
    for row in rows {
        if row.open_date != current_date {
            if !current_date.is_empty() {
                out.push('\n');
            }
            current_date = row.open_date.clone();
            let count = rows
                .iter()
                .filter(|item| item.open_date == current_date)
                .count();
            out.push_str(&format!("=== {current_date} 批次（{count} 只）===\n"));
            out.push_str(&format!("  {:<8} {:<10}", "代码", "名称"));
            for period in periods {
                out.push_str(&format!(" {:>8}", format!("D+{period}")));
            }
            out.push_str("  来源\n");
        }
        let source_label = match row.source.as_str() {
            "board_rotation" => "轮动",
            "news_catalyst" => "新闻",
            "p01_preopen" => "P01",
            "money_flow" => "资金流",
            "holding" => "持仓",
            other => other,
        };
        out.push_str(&format!("  {:<8} {:<10}", row.code, row.name));
        for period in periods {
            out.push_str(&format!(
                " {}",
                fmt_return(row.returns.get(period - 1).copied().flatten())
            ));
        }
        out.push_str(&format!(
            "  {source_label}  {}\n",
            row.base_price
                .map(|price| format!("基准 {price:.2}"))
                .unwrap_or_else(|| "无基准价".to_string())
        ));
    }

    out.push_str("\n=== 批次汇总 ===\n");
    out.push_str(&format!("  {:<12} {:>4}", "开票日", "票数"));
    for period in periods {
        out.push_str(&format!(
            " {:>10} {:>10} {:>16}",
            format!("D+{period}胜率"),
            format!("D+{period}均值"),
            format!("D+{period}市场")
        ));
    }
    out.push('\n');

    let mut dates: Vec<String> = rows.iter().map(|row| row.open_date.clone()).collect();
    dates.sort();
    dates.dedup();
    for date in &dates {
        let date_rows: Vec<&BatchTicket> = rows
            .iter()
            .filter(|row| &row.open_date == date)
            .copied()
            .collect();
        out.push_str(&format!("  {:<12} {:>4}", date, date_rows.len()));
        for period in periods {
            out.push_str(&summary_cell(&date_rows, *period));
            out.push_str(&market_cell(calendar, day_index, by_date, date, *period));
        }
        out.push('\n');
    }

    out.push_str(&format!("  {:<12} {:>4}", "全部", rows.len()));
    for period in periods {
        out.push_str(&summary_cell(rows, *period));
        out.push_str(&market_cell(calendar, day_index, by_date, "ALL", *period));
    }
    out.push('\n');
}

/// 全市场基准格：同日开票批次的 D+N 等权收益 + 样本数；样本不足标「不可信」。
fn market_cell(
    calendar: &[String],
    day_index: &HashMap<&str, usize>,
    by_date: &HashMap<String, HashMap<String, f64>>,
    open_date: &str,
    period: usize,
) -> String {
    // 「全部」行没有单一开票日：基准不可比，留空而不是拿某一天顶替。
    if open_date == "ALL" {
        return format!(" {:>16}", "—");
    }
    let Some(start) = day_index.get(open_date).copied() else {
        return format!(" {:>16}", "—");
    };
    let Some(target_date) = calendar.get(start + period) else {
        return format!(" {:>16}", "—");
    };
    match market_return(by_date, open_date, target_date) {
        Some((mean, sample)) if sample >= MIN_MARKET_SAMPLE => {
            format!(" {mean:>+8.2}% n={sample:<4}")
        }
        Some((mean, sample)) => format!(" {mean:>+8.2}% n={sample:<4}不可信"),
        None => format!(" {:>16}", "—"),
    }
}

/// 单格汇总：`胜率%` + `均值%`，没样本时两个都是「—」。
fn summary_cell(rows: &[&BatchTicket], period: usize) -> String {
    let values: Vec<f64> = rows
        .iter()
        .filter_map(|row| row.returns.get(period - 1).copied().flatten())
        .collect();
    if values.is_empty() {
        return format!(" {:>10} {:>10}", "—", "—");
    }
    let wins = values.iter().filter(|value| **value > 0.0).count();
    let win_rate = wins as f64 / values.len() as f64 * 100.0;
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    format!(" {:>9.0}% {:>9.2}%", win_rate, mean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_return_averages_only_common_codes() {
        let mut by_date: HashMap<String, HashMap<String, f64>> = HashMap::new();
        by_date.insert(
            "2026-10-20".to_string(),
            [("600000".to_string(), 10.0), ("600001".to_string(), 20.0)]
                .into_iter()
                .collect(),
        );
        by_date.insert(
            "2026-10-21".to_string(),
            [("600000".to_string(), 11.0), ("600002".to_string(), 30.0)]
                .into_iter()
                .collect(),
        );
        let (mean, sample) = market_return(&by_date, "2026-10-20", "2026-10-21").expect("mean");
        assert_eq!(sample, 1, "只有 600000 两天都有收盘价");
        assert!((mean - 10.0).abs() < 1e-9);
    }

    #[test]
    fn market_return_missing_date_is_none() {
        let by_date: HashMap<String, HashMap<String, f64>> = HashMap::new();
        assert!(market_return(&by_date, "2026-10-20", "2026-10-21").is_none());
    }

    #[test]
    fn regime_windows_are_documented_values() {
        assert_eq!(Regime::Emotion.perf_periods(), &[1, 3]);
        assert_eq!(Regime::Trend.perf_periods(), &[5, 10]);
    }
}
