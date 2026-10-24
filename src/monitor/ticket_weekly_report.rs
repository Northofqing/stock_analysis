//! 选股周报（批 4.3 / 4.4，2026-10-24）。
//!
//! ## 口径
//!
//! - 窗口 = `week_end` 所在自然周的周一 … `week_end`（周五或最后一个交易日）；
//! - 只统计本周**开票**的票（`ticket_lifecycle.open_date` 落在窗口内），按 `source` 聚合；
//! - 基准价与 `watch_pool_perf` 同源：`p01_preopen` → 开盘价，其余 → 收盘价；
//! - 指标：D+1 / D+3 的胜率、均值、**中位数**、超额（个股 − 同日全市场等权）；
//! - 亏损票对照：D+1 < 0 的票列前 8 只，带来源/分型/当日涨跌，用于「为什么亏」的追问。
//!
//! ## 为什么带中位数
//!
//! 2026-10-24 实测（10-19 起 41 只）：情绪型 D+3 均值 +4.89%、中位 **-5.04%** ——
//! 两只 +33%/+35% 把均值拉起来，多数票是亏的。只给均值等于给一个不存在的「平均体验」。
//!
//! ## 边界
//!
//! - 市场基准样本 < 800 只时超额**不给数**（BR-222），只标「样本不足」；
//! - D+3 尚未走到的票不计入分母（不补 0）；
//! - 趋势型 D+5/D+10 不在周报里（周五时本周开票的票还没走到 D+5），留给 `watch_pool_perf`；
//! - 假设执行、非真实收益；不参与任何自动调参。

use std::collections::HashMap;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Text};

use crate::database::DatabaseManager;
use crate::monitor::buy_zone::resolve_name;
use crate::monitor::ticket_regime::{self, Regime};
use crate::monitor::watch_pool_perf::MIN_MARKET_SAMPLE;

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

#[derive(Debug, Clone)]
pub struct WeeklyRow {
    pub code: String,
    pub name: String,
    pub source: &'static str,
    pub regime: Regime,
    pub open_date: String,
    pub d1: Option<f64>,
    pub d3: Option<f64>,
    /// 个股 D+1 − 同日全市场等权（样本不足时 None）
    pub excess_d1: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct SourceStat {
    pub label: &'static str,
    pub n: usize,
    pub win_rate: f64,
    pub mean: f64,
    pub median: f64,
    pub excess: Option<f64>,
}

#[derive(Debug, Default)]
pub struct WeeklyReport {
    pub week_start: String,
    pub week_end: String,
    pub rows: Vec<WeeklyRow>,
    pub stats: Vec<SourceStat>,
    pub losers: Vec<WeeklyRow>,
    pub d1_n: usize,
    pub d3_n: usize,
    /// 有票但没有基准价（当日无开盘/收盘行）→ 不进分母，但要在口径里报出来
    pub no_base_price: usize,
    /// 市场基准样本不足的交易日数
    pub thin_market_days: usize,
    pub errors: Vec<String>,
}

fn source_label(source: &str) -> &'static str {
    match source {
        "board_rotation" => "轮动",
        "news_catalyst" => "新闻",
        "p01_preopen" => "P01",
        "money_flow" => "资金流",
        "holding" => "持仓",
        _ => "其它",
    }
}

/// 统计一组数值：n / 胜率 / 均值 / 中位数。
pub fn summarize(values: &[f64]) -> Option<(usize, f64, f64, f64)> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    let median = if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    };
    let wins = sorted.iter().filter(|value| **value > 0.0).count();
    Some((
        sorted.len(),
        wins as f64 / sorted.len() as f64 * 100.0,
        sorted.iter().sum::<f64>() / sorted.len() as f64,
        median,
    ))
}

fn monday_of(day: chrono::NaiveDate) -> chrono::NaiveDate {
    use chrono::Datelike;
    let offset = day.weekday().num_days_from_monday() as i64;
    day - chrono::Duration::days(offset)
}

/// 全市场等权收益（`from`→`to` 两日都有收盘价的票），返回 (均值%, 样本数)。
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

pub fn build(db: &DatabaseManager, week_end: &str) -> Result<WeeklyReport, String> {
    let end = chrono::NaiveDate::parse_from_str(week_end, "%Y-%m-%d")
        .map_err(|error| format!("week_end 非法 ({week_end}): {error}"))?;
    let start = monday_of(end);
    let start_text = start.format("%Y-%m-%d").to_string();

    let mut conn = db.get_conn().map_err(|error| error.to_string())?;

    let calendar: Vec<String> = diesel::sql_query(
        "SELECT DISTINCT trade_date FROM board_ranking_daily ORDER BY trade_date",
    )
    .load::<DateRow>(&mut conn)
    .map_err(|error| error.to_string())?
    .into_iter()
    .map(|row| row.trade_date)
    .collect();
    if calendar.is_empty() {
        return Err("板块榜为空，没有交易日历".to_string());
    }
    let day_index: HashMap<&str, usize> = calendar
        .iter()
        .enumerate()
        .map(|(index, date)| (date.as_str(), index))
        .collect();

    let tickets: Vec<TicketRow> = diesel::sql_query(
        "SELECT id, open_date, code, source FROM ticket_lifecycle \
          WHERE open_date >= ?1 AND open_date <= ?2 \
          GROUP BY open_date, code ORDER BY open_date, code",
    )
    .bind::<Text, _>(start_text.clone())
    .bind::<Text, _>(week_end.to_owned())
    .load(&mut conn)
    .map_err(|error| format!("本周开票查询失败: {error}"))?;

    // 只取窗口内的日线（含 D+3 需要的后续交易日）。
    let horizon = calendar
        .iter()
        .rev()
        .find(|date| date.as_str() <= week_end)
        .cloned()
        .unwrap_or_else(|| week_end.to_string());
    let horizon_index = day_index.get(horizon.as_str()).copied().unwrap_or(0);
    let last_needed = calendar
        .get(horizon_index + 5)
        .cloned()
        .unwrap_or_else(|| horizon.clone());

    let mut closes: HashMap<(String, String), f64> = HashMap::new();
    let mut opens: HashMap<(String, String), f64> = HashMap::new();
    let mut by_date: HashMap<String, HashMap<String, f64>> = HashMap::new();
    for row in diesel::sql_query(
        "SELECT code, date, close, open FROM stock_daily \
          WHERE close > 0 AND date >= ?1 AND date <= ?2",
    )
    .bind::<Text, _>(start_text.clone())
    .bind::<Text, _>(last_needed.clone())
    .load::<BarRow>(&mut conn)
    .map_err(|error| format!("日线查询失败: {error}"))?
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

    let mut report = WeeklyReport {
        week_start: start_text,
        week_end: week_end.to_string(),
        ..Default::default()
    };
    let mut thin_days: std::collections::HashSet<String> = std::collections::HashSet::new();

    for ticket in &tickets {
        let decision = match ticket_regime::resolve(
            &mut conn,
            ticket.id,
            &ticket.code,
            &ticket.source,
            week_end,
        ) {
            Ok(decision) => decision,
            Err(error) => {
                report
                    .errors
                    .push(format!("{} 分型失败: {error}", ticket.code));
                continue;
            }
        };
        let base = if ticket.source == "p01_preopen" {
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
        let Some(base) = base else {
            // 无基准价：不补值，也不进分母，但必须计数——否则「开票 40 只」和
            // ticket_lifecycle 里的 41 只对不上，读的人会以为漏票。
            report.no_base_price += 1;
            continue;
        };
        let Some(start_index) = day_index.get(ticket.open_date.as_str()).copied() else {
            continue;
        };
        let price_at = |period: usize| -> Option<f64> {
            let date = calendar.get(start_index + period)?;
            closes.get(&(ticket.code.clone(), date.clone())).copied()
        };
        let pct = |price: Option<f64>| price.map(|value| (value / base - 1.0) * 100.0);
        let d1 = pct(price_at(1));
        let d3 = pct(price_at(3));

        // 超额：D+1 当天全市场等权。
        let excess_d1 = match (d1, calendar.get(start_index + 1)) {
            (Some(stock), Some(target_date)) => {
                match market_return(&by_date, &ticket.open_date, target_date) {
                    Some((market, sample)) if sample >= MIN_MARKET_SAMPLE => Some(stock - market),
                    Some((_, _)) => {
                        thin_days.insert(ticket.open_date.clone());
                        None
                    }
                    None => None,
                }
            }
            _ => None,
        };

        report.rows.push(WeeklyRow {
            code: ticket.code.clone(),
            name: resolve_name(&mut conn, &ticket.code).unwrap_or_default(),
            source: source_label(&ticket.source),
            regime: decision.regime,
            open_date: ticket.open_date.clone(),
            d1,
            d3,
            excess_d1,
        });
    }

    report.d1_n = report.rows.iter().filter(|row| row.d1.is_some()).count();
    report.d3_n = report.rows.iter().filter(|row| row.d3.is_some()).count();
    report.thin_market_days = thin_days.len();

    // 按来源聚合
    let mut labels: Vec<&str> = report.rows.iter().map(|row| row.source).collect();
    labels.sort();
    labels.dedup();
    for label in labels {
        let rows: Vec<&WeeklyRow> = report
            .rows
            .iter()
            .filter(|row| row.source == label)
            .collect();
        let d1: Vec<f64> = rows.iter().filter_map(|row| row.d1).collect();
        if let Some((n, win_rate, mean, median)) = summarize(&d1) {
            let excess: Vec<f64> = rows.iter().filter_map(|row| row.excess_d1).collect();
            report.stats.push(SourceStat {
                label,
                n,
                win_rate,
                mean,
                median,
                excess: if excess.is_empty() {
                    None
                } else {
                    Some(excess.iter().sum::<f64>() / excess.len() as f64)
                },
            });
        }
    }
    report.stats.sort_by(|a, b| {
        b.mean
            .partial_cmp(&a.mean)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut losers: Vec<WeeklyRow> = report
        .rows
        .iter()
        .filter(|row| row.d1.map(|value| value < 0.0).unwrap_or(false))
        .cloned()
        .collect();
    losers.sort_by(|a, b| a.d1.partial_cmp(&b.d1).unwrap_or(std::cmp::Ordering::Equal));
    losers.truncate(8);
    report.losers = losers;

    Ok(report)
}

impl WeeklyReport {
    /// 飞书简化版正文（用户偏好：报告整理简化后推送）。
    pub fn render(&self) -> String {
        let mut out = format!(
            "【选股周报 {}~{}】\n本周开票 {} 只（D+1 可算 {} / D+3 可算 {}）\n",
            self.week_start,
            self.week_end,
            self.rows.len() + self.no_base_price,
            self.d1_n,
            self.d3_n
        );
        if self.no_base_price > 0 {
            out.push_str(&format!(
                "其中 {} 只无基准价（当日无行情行），不参与统计\n",
                self.no_base_price
            ));
        }
        if self.rows.is_empty() {
            out.push_str("本周没有开票。\n");
            return out;
        }

        out.push_str("\n按入口（D+1）：\n");
        for stat in &self.stats {
            let excess = match stat.excess {
                Some(value) => format!("{value:+.2}%"),
                None => "样本不足".to_string(),
            };
            out.push_str(&format!(
                "· {} n={} 胜率{:.0}% 均值{:+.2}% 中位{:+.2}% 超额{}\n",
                stat.label, stat.n, stat.win_rate, stat.mean, stat.median, excess
            ));
        }

        let emotion = self
            .rows
            .iter()
            .filter(|row| row.regime == Regime::Emotion)
            .count();
        out.push_str(&format!(
            "\n分型：情绪型 {} / 趋势型 {}\n",
            emotion,
            self.rows.len() - emotion
        ));

        if !self.losers.is_empty() {
            out.push_str("\n亏损票（D+1 最差 8 只）：\n");
            for row in &self.losers {
                out.push_str(&format!(
                    "· {} {}〔{}·{}〕{}\n",
                    row.code,
                    row.name,
                    row.source,
                    row.regime.label(),
                    row.d1
                        .map(|value| format!("{value:+.2}%"))
                        .unwrap_or_default()
                ));
            }
        }

        if self.thin_market_days > 0 {
            out.push_str(&format!(
                "\n注：{} 个交易日全市场日线样本不足 {} 只，超额不给数。\n",
                self.thin_market_days, MIN_MARKET_SAMPLE
            ));
        }
        out.push_str("假设执行、非真实收益；不自动调参。");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_reports_median_not_just_mean() {
        // 两只尾部大牛 + 多数小亏：均值正、中位负（2026-10-24 实测形态）
        let values = [-14.75, 35.38, -7.61, -5.81, -4.76, -5.33, 33.12, 8.91];
        let (n, win_rate, mean, median) = summarize(&values).expect("stats");
        assert_eq!(n, 8);
        assert!((win_rate - 37.5).abs() < 1e-9);
        assert!((mean - 4.89).abs() < 0.01);
        assert!((median + 5.045).abs() < 0.01, "中位应为负");
        assert!(mean > median, "均值被尾部拉高");
    }

    #[test]
    fn summarize_empty_is_none() {
        assert!(summarize(&[]).is_none());
    }

    #[test]
    fn monday_math() {
        let friday = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
        assert_eq!(
            monday_of(friday),
            chrono::NaiveDate::from_ymd_opt(2026, 10, 19).unwrap()
        );
        let monday = chrono::NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        assert_eq!(monday_of(monday), monday);
    }
}
