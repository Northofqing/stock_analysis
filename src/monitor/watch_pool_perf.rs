//! 观察池选股表现（按开票批次统计 N 日收益与胜率）。
//!
//! ## 为什么要有这个
//!
//! 「选出来的票后来涨没涨」是唯一能判断选股规则好不好用的证据。
//! 板块轮动、CATALYST 阈值、每板块取几只——这些参数光靠盘感调不出来，
//! 得有批次级的胜率和均值。
//!
//! ## 口径
//!
//! - 基准价 = `stock_daily` 在 `open_date` 当天的收盘（选出来的那天）
//! - N 日价 = 交易日历上往后第 N 个交易日的收盘
//! - 收益 = (N日价 / 基准价 - 1) × 100
//! - 交易日历取 `board_ranking_daily` 的 distinct trade_date（盘后必写）
//!
//! 还没走到第 N 日的批次显示「—」，不补 0（BR-222）。
//!
//! 调用方：[`crate::bin::watch_pool_perf`] 手动跑，
//! `monitor` 16:05 调度自动跑并落盘。

use std::collections::HashMap;

use diesel::prelude::*;
use diesel::sql_types::{Double, Text};

use crate::monitor::buy_zone::resolve_name;

#[derive(QueryableByName)]
struct TicketRow {
    #[diesel(sql_type = Text)]
    open_date: String,
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    source: String,
}

#[derive(QueryableByName)]
struct CloseRow {
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    date: String,
    #[diesel(sql_type = Double)]
    close: f64,
}

#[derive(QueryableByName)]
struct DateRow {
    #[diesel(sql_type = Text)]
    trade_date: String,
}

struct BatchTicket {
    open_date: String,
    code: String,
    name: String,
    source: String,
    base_close: Option<f64>,
    returns: Vec<Option<f64>>,
}

/// 统计参数。
pub struct PerfOptions {
    /// 观察周期（交易日），默认 `[1, 3, 5]`。
    pub periods: Vec<usize>,
    /// 只看这个日期（含）之后开的票；None = 全部。
    pub since: Option<String>,
}

impl Default for PerfOptions {
    fn default() -> Self {
        Self {
            periods: vec![1, 3, 5],
            since: None,
        }
    }
}

/// 生成统计文本。没有开票记录时返回提示串（不是错误）。
pub fn render(conn: &mut SqliteConnection, options: &PerfOptions) -> Result<String, String> {
    // 交易日历：升序
    let calendar: Vec<String> =
        diesel::sql_query("SELECT DISTINCT trade_date FROM board_ranking_daily ORDER BY trade_date")
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

    let base_sql = "SELECT open_date, code, source FROM ticket_lifecycle \
         WHERE source IN ('board_rotation','news_catalyst')";
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

    // 一次性取回全部日线
    let mut closes: HashMap<(String, String), f64> = HashMap::new();
    for row in diesel::sql_query("SELECT code, date, close FROM stock_daily WHERE close > 0")
        .load::<CloseRow>(conn)
        .map_err(|error| error.to_string())?
    {
        closes.insert((row.code, row.date), row.close);
    }

    let mut batches: Vec<BatchTicket> = Vec::new();
    for ticket in tickets {
        let name = resolve_name(conn, &ticket.code).unwrap_or_default();
        let base = closes
            .get(&(ticket.code.clone(), ticket.open_date.clone()))
            .copied();
        let start = day_index.get(ticket.open_date.as_str()).copied();

        let returns = options
            .periods
            .iter()
            .map(|period| {
                let base = base?;
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
            base_close: base,
            returns,
        });
    }

    if batches.is_empty() {
        return Ok("没有观察池开票记录\n".to_string());
    }

    let mut out = String::new();
    let fmt_return = |value: Option<f64>| match value {
        Some(pct) => format!("{pct:+7.2}%"),
        None => "      —".to_string(),
    };

    let mut current_date = String::new();
    for row in &batches {
        if row.open_date != current_date {
            if !current_date.is_empty() {
                out.push('\n');
            }
            current_date = row.open_date.clone();
            let count = batches
                .iter()
                .filter(|item| item.open_date == current_date)
                .count();
            out.push_str(&format!("=== {current_date} 批次（{count} 只）===\n"));
            out.push_str(&format!("  {:<8} {:<10}", "代码", "名称"));
            for period in &options.periods {
                out.push_str(&format!(" {:>8}", format!("D+{period}")));
            }
            out.push_str("  来源\n");
        }
        let source_label = match row.source.as_str() {
            "board_rotation" => "轮动",
            "news_catalyst" => "新闻",
            other => other,
        };
        out.push_str(&format!("  {:<8} {:<10}", row.code, row.name));
        for value in &row.returns {
            out.push_str(&format!(" {}", fmt_return(*value)));
        }
        out.push_str(&format!(
            "  {source_label}  {}\n",
            row.base_close
                .map(|close| format!("基准 {close:.2}"))
                .unwrap_or_else(|| "无基准价".to_string())
        ));
    }

    out.push_str("\n=== 批次汇总 ===\n");
    out.push_str(&format!("  {:<12} {:>4}", "开票日", "票数"));
    for period in &options.periods {
        out.push_str(&format!(
            " {:>10} {:>10}",
            format!("D+{period}胜率"),
            format!("D+{period}均值")
        ));
    }
    out.push('\n');

    let mut dates: Vec<String> = batches.iter().map(|row| row.open_date.clone()).collect();
    dates.sort();
    dates.dedup();
    for date in &dates {
        let rows: Vec<&BatchTicket> = batches
            .iter()
            .filter(|row| &row.open_date == date)
            .collect();
        out.push_str(&format!("  {:<12} {:>4}", date, rows.len()));
        for (index, _) in options.periods.iter().enumerate() {
            out.push_str(&summary_cell(&rows, index));
        }
        out.push('\n');
    }

    let all_rows: Vec<&BatchTicket> = batches.iter().collect();
    out.push_str(&format!("  {:<12} {:>4}", "全部", batches.len()));
    for (index, _) in options.periods.iter().enumerate() {
        out.push_str(&summary_cell(&all_rows, index));
    }
    out.push('\n');

    Ok(out)
}

/// 单格汇总：`胜率%` + `均值%`，没样本时两个都是「—」。
fn summary_cell(rows: &[&BatchTicket], index: usize) -> String {
    let values: Vec<f64> = rows
        .iter()
        .filter_map(|row| row.returns.get(index).copied().flatten())
        .collect();
    if values.is_empty() {
        return format!(" {:>10} {:>10}", "—", "—");
    }
    let wins = values.iter().filter(|value| **value > 0.0).count();
    let win_rate = wins as f64 / values.len() as f64 * 100.0;
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    format!(" {:>9.0}% {:>9.2}%", win_rate, mean)
}
