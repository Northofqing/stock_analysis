//! 布林+MACD 信号回测
//!
//! 用 `reports/analysis/closed_positions_with_ai.csv` 中 136 笔已平仓数据，
//! 在每笔的"买入日"时点重跑 `detect_boll_macd_signal`，统计：
//!
//! 1. 该笔交易的买入日会被新规则归为什么动作（BottomBuy / UptrendStart / ...）
//! 2. 各动作的胜率、平均收益率、盈亏比 —— 与原 A 方案 (15.44% 胜率) 对比
//!
//! 用法：cargo run --bin boll_macd_backtest

use anyhow::{Context, Result};
use chrono::NaiveDate;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};

use stock_analysis::data_provider::DataFetcherManager;
use stock_analysis::strategy::{detect_boll_macd_signal, BollMacdAction};

#[derive(Debug, Clone)]
struct ClosedTrade {
    code: String,
    name: String,
    buy_date: NaiveDate,
    #[allow(dead_code)]
    sell_date: NaiveDate,
    buy_price: f64,
    return_pct: f64,
    hold_days: i32,
    ai_score: i32,
}

fn parse_csv(path: &str) -> Result<Vec<ClosedTrade>> {
    let f = File::open(path).with_context(|| format!("open {}", path))?;
    let mut out = Vec::new();
    for (i, line) in BufReader::new(f).lines().enumerate() {
        let line = line?;
        if i == 0 {
            continue;
        }
        // 简单 CSV 解析（处理 "..." 引号字段）
        let mut fields: Vec<String> = Vec::new();
        let mut buf = String::new();
        let mut in_quote = false;
        for c in line.chars() {
            match c {
                '"' => in_quote = !in_quote,
                ',' if !in_quote => {
                    fields.push(buf.trim().to_string());
                    buf.clear();
                }
                _ => buf.push(c),
            }
        }
        fields.push(buf.trim().to_string());
        if fields.len() < 12 {
            continue;
        }
        // 列顺序：代码,名称,买入日期,卖出日期,买入价,卖出价,收益率_pct,持仓天数,买入日AI评分,买入日建议,买入日趋势,卖出日AI评分,卖出日建议
        let code = fields[0].clone();
        let name = fields[1].clone();
        let buy_date = NaiveDate::parse_from_str(&fields[2], "%Y-%m-%d")?;
        let sell_date = NaiveDate::parse_from_str(&fields[3], "%Y-%m-%d")?;
        let buy_price = fields[4].parse::<f64>().unwrap_or(0.0);
        let return_pct = fields[6].parse::<f64>().unwrap_or(0.0);
        let hold_days = fields[7].parse::<i32>().unwrap_or(0);
        let ai_score = fields[8].parse::<i32>().unwrap_or(0);
        out.push(ClosedTrade {
            code, name, buy_date, sell_date, buy_price, return_pct, hold_days, ai_score,
        });
    }
    Ok(out)
}

#[derive(Default, Debug)]
struct ActionStats {
    count: usize,
    wins: usize,
    sum_return: f64,
    sum_win_return: f64,
    sum_loss_return: f64,
    win_count: usize,
    loss_count: usize,
}

impl ActionStats {
    fn add(&mut self, ret: f64) {
        self.count += 1;
        self.sum_return += ret;
        if ret > 0.0 {
            self.wins += 1;
            self.win_count += 1;
            self.sum_win_return += ret;
        } else {
            self.loss_count += 1;
            self.sum_loss_return += ret;
        }
    }

    fn win_rate(&self) -> f64 {
        if self.count == 0 { 0.0 } else { self.wins as f64 / self.count as f64 * 100.0 }
    }
    fn avg_return(&self) -> f64 {
        if self.count == 0 { 0.0 } else { self.sum_return / self.count as f64 }
    }
    fn avg_win(&self) -> f64 {
        if self.win_count == 0 { 0.0 } else { self.sum_win_return / self.win_count as f64 }
    }
    fn avg_loss(&self) -> f64 {
        if self.loss_count == 0 { 0.0 } else { self.sum_loss_return / self.loss_count as f64 }
    }
    fn profit_loss_ratio(&self) -> f64 {
        let l = self.avg_loss().abs();
        if l == 0.0 { 0.0 } else { self.avg_win() / l }
    }
}

fn action_label(a: BollMacdAction) -> &'static str {
    match a {
        BollMacdAction::None => "None(无信号)",
        BollMacdAction::PreReversal => "PreReversal(变盘)",
        BollMacdAction::BottomBuy => "BottomBuy(下轨抄底)",
        BollMacdAction::UptrendStart => "UptrendStart(主升浪)",
        BollMacdAction::TopSell => "TopSell(上轨减仓)",
    }
}

fn main() -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    let _guard = rt.enter();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let trades = parse_csv("reports/analysis/closed_positions_with_ai.csv")?;
    println!("📊 共加载 {} 笔已平仓交易\n", trades.len());

    let manager = DataFetcherManager::new()?;

    let mut stats: BTreeMap<&'static str, ActionStats> = BTreeMap::new();
    let mut returns_by_action: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
    let mut overall = ActionStats::default();
    let mut high_score_block_stats = ActionStats::default(); // AI score >= 60
    let mut by_action_examples: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();

    let mut data_failures = 0;
    let mut date_misses = 0;

    for (i, t) in trades.iter().enumerate() {
        // 拉取足够历史：从今天回看到 buy_date 之前 60 天，足以做布林20+背离30
        let today = chrono::Local::now().date_naive();
        let need_days = (today - t.buy_date).num_days() as usize + 80;
        print!("[{}/{}] {} {} buy@{} ret={:+.2}% ... ",
               i + 1, trades.len(), t.code, t.name, t.buy_date, t.return_pct);

        let data = match manager.get_daily_data(&t.code, need_days) {
            Ok((d, _)) => d,
            Err(e) => {
                println!("❌ 数据失败: {}", e);
                data_failures += 1;
                continue;
            }
        };

        // data 倒序：data[0] = 最新。找到 date == buy_date 的位置
        let idx = data.iter().position(|k| k.date == t.buy_date);
        let idx = match idx {
            Some(i) => i,
            None => {
                println!("⚠️  无 {} 当日数据", t.buy_date);
                date_misses += 1;
                continue;
            }
        };

        // 切片：[buy_date, ..., 最早]，模拟买入日当日的视角
        let slice = &data[idx..];
        if slice.len() < 35 {
            println!("⚠️  历史不足 {} < 35", slice.len());
            continue;
        }

        let sig = detect_boll_macd_signal(slice);
        let label = action_label(sig.action);
        let entry = stats.entry(label).or_default();
        entry.add(t.return_pct);
        returns_by_action.entry(label).or_default().push(t.return_pct);
        overall.add(t.return_pct);
        if t.ai_score >= 60 {
            high_score_block_stats.add(t.return_pct);
        }
        by_action_examples
            .entry(label)
            .or_default()
            .push(format!("{} {} ret={:+.2}% AI={} {}d", t.code, t.name, t.return_pct, t.ai_score, t.hold_days));

        println!("{} (DIF={:.3} hist={:+.3} bw={:.1}% Δbw={:+.1}%)",
                 label, sig.macd_dif, sig.macd_hist, sig.band_width_pct, sig.band_change_pct);
    }

    println!("\n========== 回测结果 ==========");
    println!("数据失败: {} 笔，买入日缺失: {} 笔\n", data_failures, date_misses);

    let header = format!(
        "{:<22} | {:>5} | {:>7} | {:>9} | {:>8} | {:>8} | {:>5}",
        "动作", "笔数", "胜率%", "平均收益%", "平均盈%", "平均亏%", "盈亏比"
    );
    println!("{}", header);
    println!("{}", "-".repeat(header.len()));

    for (label, s) in &stats {
        println!(
            "{:<22} | {:>5} | {:>7.2} | {:>9.2} | {:>8.2} | {:>8.2} | {:>5.2}",
            label,
            s.count,
            s.win_rate(),
            s.avg_return(),
            s.avg_win(),
            s.avg_loss(),
            s.profit_loss_ratio(),
        );
    }
    println!("{}", "-".repeat(header.len()));
    println!(
        "{:<22} | {:>5} | {:>7.2} | {:>9.2} | {:>8.2} | {:>8.2} | {:>5.2}",
        "全样本(原A方案)",
        overall.count,
        overall.win_rate(),
        overall.avg_return(),
        overall.avg_win(),
        overall.avg_loss(),
        overall.profit_loss_ratio(),
    );

    println!("\n========== 关键对比 ==========");
    let buy_signals: Vec<&str> = vec!["BottomBuy(下轨抄底)", "UptrendStart(主升浪)"];
    let mut filtered = ActionStats::default();
    for label in &buy_signals {
        if let Some(rs) = returns_by_action.get(label) {
            for r in rs {
                filtered.add(*r);
            }
        }
    }

    println!("✅ 新规则【仅 BottomBuy + UptrendStart】筛选后：");
    println!(
        "   {} 笔 / 胜率 {:.2}% / 均收益 {:+.2}% / 盈亏比 {:.2}",
        filtered.count,
        filtered.win_rate(),
        filtered.avg_return(),
        filtered.profit_loss_ratio(),
    );
    println!(
        "❌ 原 A 方案（136 笔）：胜率 {:.2}% / 均收益 {:+.2}% / 盈亏比 {:.2}",
        overall.win_rate(),
        overall.avg_return(),
        overall.profit_loss_ratio(),
    );

    // AI score >= 60 子集（即原 A 方案会被 HIGH_SCORE_BLOCK 过滤掉的部分）
    println!(
        "\n🔍 对照：AI≥60 子集 {} 笔 / 胜率 {:.2}% / 均收益 {:+.2}%",
        high_score_block_stats.count,
        high_score_block_stats.win_rate(),
        high_score_block_stats.avg_return(),
    );

    println!("\n========== 各动作样本（前 5）==========");
    for (label, exs) in &by_action_examples {
        println!("\n{} ({} 笔):", label, exs.len());
        for ex in exs.iter().take(5) {
            println!("  {}", ex);
        }
    }

    Ok(())
}
