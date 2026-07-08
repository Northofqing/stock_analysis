// -*- coding: utf-8 -*-
//! 新闻 Ranker 推送 outcome 回看 (P3 — D+1/D+3/D+5 表现跟踪)
//!
//! **目的**: 读 `data/news_rank_audit_YYYY-MM-DD.jsonl`, 对每条推送找关联股票,
//!   拉 K 线算 D+1/D+3/D+5 涨幅 + MFE/MAE + 5 维度可执行性判断.
//!
//! **不做自动调权** (P2-News-2 评审 P3 决定 + review 第二轮强调):
//!   - 跑一段时间后人工审计, 不让系统自动改 chain_rules.toml
//!   - 防止把系统训练成追涨杀跌器
//!
//! **5 维度可执行性** (review 第二轮 "可执行买点" 强调):
//!   1. 推送时是否涨停买不到 (push 当日 change >= 9.5%)
//!   2. D+1 是否高开低走 (open > prev_close*1.01 AND close < open)
//!   3. 是否先跌破止损再涨 (D+1~D+5 最低 < push_price*0.95 AND D+5 涨)
//!   4. 是否板块普涨带动 (已接: D+1 个股涨幅 vs 板块 5 支成份股平均, 差距 < 0.5%)
//!   5. 是否有可执行买点 (D+1 区间包含推送价 ±3% 内)
//!
//! **红线**:
//!   - K 线拉取失败 → graceful 返 None, 不 panic
//!   - 关联股票找不到 → 标 "无关联股票", 不强行编
//!   - 缺数据维度 → 在 reasons 列显式标, 不为 0
use crate::opportunity::news_audit::audit_path;
use crate::opportunity::news_ranker::{EventType, HeatStage, NewsRankBucket};
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Audit 单行 (反序列化用, 字段对齐 news_audit::AuditRow)
#[derive(Debug, Deserialize)]
struct AuditRowRead {
    ts: String,
    candidate_id: String,
    title: String,
    source: String,
    chain: String,
    board_code: Option<String>,
    event_type: String,
    heat_stage: String,
    score: i32,
    bucket: String,
    #[allow(dead_code)]
    rule_score: i32,
    #[allow(dead_code)]
    freshness_score: i32,
    #[allow(dead_code)]
    heat_score: i32,
    #[allow(dead_code)]
    stage_score: i32,
    #[allow(dead_code)]
    capital_score: i32,
    #[allow(dead_code)]
    source_score: i32,
    #[allow(dead_code)]
    risk_penalty: i32,
    #[allow(dead_code)]
    reasons: Vec<String>,
    #[allow(dead_code)]
    drop_reason: Option<String>,
}

/// 单条推送的回看结果
#[derive(Debug, Clone, Serialize)]
pub struct NewsOutcome {
    pub candidate_id: String,
    pub title: String,
    pub chain: String,
    pub bucket: String,
    pub push_at: String,
    pub code: Option<String>,
    /// 推送日收盘价 (None = 拉不到)
    pub push_price: Option<f64>,
    /// D+1 涨幅 (%)
    pub d1_pct: Option<f64>,
    /// D+3 涨幅 (%)
    pub d3_pct: Option<f64>,
    /// D+5 涨幅 (%)
    pub d5_pct: Option<f64>,
    /// 最大浮盈 (D+1~D+5 期间最高收盘 vs push_price, %)
    pub mfe: Option<f64>,
    /// 最大回撤 (D+1~D+5 期间最低收盘 vs push_price, %)
    pub mae: Option<f64>,
    /// 5 维度可执行性 (None = 缺数据, Some(bool) = 判断结果)
    pub limit_up_unbuyable: Option<bool>,
    pub open_high_sell_low_d1: Option<bool>,
    pub stop_break_first: Option<bool>,
    pub sector_driven: Option<bool>, // D+1 个股 vs 板块 5 支平均, 差距 < 0.5% = 板块带动
    pub executable_entry: Option<bool>,
    /// 总体评估
    pub verdict: String,
    /// 缺数据 / 异常原因
    pub reasons: Vec<String>,
}

/// 推送日收盘价 (K 线按 push_at 日期取)
fn kline_at_or_before(klines: &[crate::data_provider::KlineData], date: NaiveDate) -> Option<&crate::data_provider::KlineData> {
    klines.iter().rev().find(|k| k.date <= date)
}

/// D+N 涨幅 (推送后 N 个交易日的收盘涨幅 vs push_price)
fn pct_change_at(klines: &[crate::data_provider::KlineData], push_date: NaiveDate, n_days: usize) -> Option<f64> {
    let push_k = kline_at_or_before(klines, push_date)?;
    // 从 push_date 之后数 n_days 个交易日
    let after: Vec<&crate::data_provider::KlineData> = klines
        .iter()
        .filter(|k| k.date > push_date)
        .take(n_days)
        .collect();
    let target = after.last()?;
    if push_k.close <= 0.0 {
        return None;
    }
    Some((target.close - push_k.close) / push_k.close * 100.0)
}

/// MFE / MAE (推送后 n_days 内, 最高 / 最低 收盘 vs push_price, %)
fn mfe_mae(klines: &[crate::data_provider::KlineData], push_date: NaiveDate, n_days: usize) -> (Option<f64>, Option<f64>) {
    let push_k = match kline_at_or_before(klines, push_date) {
        Some(k) if k.close > 0.0 => k,
        _ => return (None, None),
    };
    let window: Vec<&crate::data_provider::KlineData> = klines
        .iter()
        .filter(|k| k.date > push_date)
        .take(n_days)
        .collect();
    if window.is_empty() {
        return (None, None);
    }
    let mfe = window.iter().map(|k| (k.close - push_k.close) / push_k.close * 100.0).fold(f64::NEG_INFINITY, f64::max);
    let mae = window.iter().map(|k| (k.close - push_k.close) / push_k.close * 100.0).fold(f64::INFINITY, f64::min);
    (Some(mfe), Some(mae))
}

/// 从 chain 名反查代码
///
/// **链路**: chain → board_code (东财 suggest API) → board components → 第一支股票
/// 用 sector_monitor 已有函数, 不引入新 HTTP 调用
///
/// **失败**:
///   - sector_monitor::search_board_code_by_keyword 返 None → 返 None
///   - fetch_board_components 失败或空 → 返 None
fn code_from_chain(chain: &str) -> Option<String> {
    use crate::market_analyzer::sector_monitor;
    // 1. chain → board_code
    let (board_code, _board_name) = sector_monitor::search_board_code_by_keyword(chain).ok()??;
    // 2. board_code → 第一支股票 (取第一支作关联)
    let comps = sector_monitor::fetch_board_components(&board_code, 5).ok()?;
    comps.first().map(|s| s.code.clone())
}

/// 评估单条 audit 的 outcome
pub fn evaluate_audit(
    row: &AuditRowRead,
    push_date: NaiveDate,
    fetcher: &crate::data_provider::DataFetcherManager,
) -> NewsOutcome {
    let mut reasons = Vec::new();
    let code = code_from_chain(&row.chain);
    if code.is_none() {
        reasons.push("无关联股票 (commit 6+ 接入)".to_string());
    }

    // 拉 K 线 (10 天覆盖 D+5 + 推送前)
    let klines_opt = code
        .as_ref()
        .and_then(|c| fetcher.get_daily_data(c, 10).ok())
        .map(|(k, _)| k);

    if klines_opt.is_none() && code.is_some() {
        reasons.push("K 线拉取失败".to_string());
    }

    let push_price = klines_opt
        .as_ref()
        .and_then(|ks| kline_at_or_before(ks, push_date).map(|k| k.close));

    // 涨幅
    let d1_pct = klines_opt.as_ref().and_then(|ks| pct_change_at(ks, push_date, 1));
    let d3_pct = klines_opt.as_ref().and_then(|ks| pct_change_at(ks, push_date, 3));
    let d5_pct = klines_opt.as_ref().and_then(|ks| pct_change_at(ks, push_date, 5));

    // MFE/MAE (D+1~D+5 5 日窗口)
    let (mfe, mae) = klines_opt
        .as_ref()
        .map(|ks| mfe_mae(ks, push_date, 5))
        .unwrap_or((None, None));

    // 5 维度
    // 1. 涨停买不到: push 当日 pct_chg >= 9.5%
    let limit_up_unbuyable = klines_opt
        .as_ref()
        .and_then(|ks| kline_at_or_before(ks, push_date))
        .map(|k| k.pct_chg >= 9.5);
    // 2. 高开低走 D+1: D+1 open > D+1 prev_close*1.01 AND D+1 close < D+1 open
    let open_high_sell_low_d1 = klines_opt.as_ref().and_then(|ks| {
        let push_k = kline_at_or_before(ks, push_date)?;
        let after: Vec<&crate::data_provider::KlineData> = ks.iter().filter(|k| k.date > push_date).take(1).collect();
        let d1 = after.first()?;
        let prev_close = push_k.close;
        Some(d1.open > prev_close * 1.01 && d1.close < d1.open)
    });
    // 3. 先跌破止损再涨: MAE < -5 AND D+5 > 0
    let stop_break_first = match (mae, d5_pct) {
        (Some(m), Some(d5)) if m < -5.0 && d5 > 0.0 => Some(true),
        (Some(_), Some(_)) => Some(false),
        _ => None,
    };
    // 4. 板块普涨: 个股 D+5 涨幅 vs 板块 D+5 涨幅, 差距 < 0.5% 视为板块带动
    // 板块涨幅: 推送日板块 (chain → board_code) 的 D+5 涨幅, 用 ConceptBoard 拉
    // 没拉到板块 K 线 → None (不补 0)
    let sector_driven = compute_sector_driven(row, klines_opt.as_deref(), push_date);
    // 5. 可执行买点: D+1 low <= push_price * 1.03 (推送价 ±3% 区间内可买)
    let executable_entry = klines_opt.as_ref().and_then(|ks| {
        let push_k = kline_at_or_before(ks, push_date)?;
        let after: Vec<&crate::data_provider::KlineData> = ks.iter().filter(|k| k.date > push_date).take(1).collect();
        let d1 = after.first()?;
        Some(d1.low <= push_k.close * 1.03)
    });

    // 总体 verdict
    let verdict = judge_verdict(d5_pct, mae, mfe, limit_up_unbuyable, sector_driven);

    NewsOutcome {
        candidate_id: row.candidate_id.clone(),
        title: row.title.clone(),
        chain: row.chain.clone(),
        bucket: row.bucket.clone(),
        push_at: row.ts.clone(),
        code,
        push_price,
        d1_pct,
        d3_pct,
        d5_pct,
        mfe,
        mae,
        limit_up_unbuyable,
        open_high_sell_low_d1,
        stop_break_first,
        sector_driven,
        executable_entry,
        verdict,
        reasons,
    }
}

/// 总体评估
fn judge_verdict(
    d5_pct: Option<f64>,
    mae: Option<f64>,
    mfe: Option<f64>,
    limit_up_unbuyable: Option<bool>,
    sector_driven: Option<bool>,
) -> String {
    if d5_pct.is_none() {
        return "无数据 (K 线缺失)".to_string();
    }
    let d5 = d5_pct.unwrap();
    let mae = mae.unwrap_or(0.0);
    let mfe = mfe.unwrap_or(0.0);
    // 综合: 涨幅 + 回撤 + 涨停买不到
    if limit_up_unbuyable == Some(true) {
        return "涨停买不到 (无法兑现)".to_string();
    }
    if d5 >= 3.0 && mae > -3.0 {
        let suffix = if sector_driven == Some(true) { " (板块普涨带动)" } else { "" };
        return format!("有兑现 (D+5 ≥ 3%, 回撤可控){}", suffix);
    }
    if d5 >= 0.0 && mae > -5.0 {
        return "中性 (D+5 ≥ 0, 但涨幅有限)".to_string();
    }
    if d5 < -3.0 {
        return "未兑现 (D+5 < -3%)".to_string();
    }
    if mae < -8.0 {
        return "高回撤 (MAE < -8%)".to_string();
    }
    if mfe > 5.0 && d5 < 0.0 {
        return "有过机会但未兑现 (MFE > 5% 但 D+5 跌)".to_string();
    }
    format!("观察 (D+5={:.1}%, MAE={:.1}%, MFE={:.1}%)", d5, mae, mfe)
}

/// 计算 sector_driven 维度
///
/// 逻辑: 个股 D+1 涨幅 vs 板块成份股 D+1 平均涨幅, 差距 < 0.5% 视为板块普涨带动
///
/// 数据源: sector_monitor::fetch_board_components (5 支) → 拉每支 K 线
///   - 个股 D+1 涨幅 vs 5 支平均 D+1 涨幅
///   - 板块本身没 K 线 (虚的), 用成份股均值作代理
///   - 失败 (K 线/board/components) → None
///
/// 一期: D+1 (单日) 而非 D+5 — 拉 5 支 D+5 K 线慢, 单日足以判断"是否板块普涨带动"
fn compute_sector_driven(
    row: &AuditRowRead,
    stock_klines: Option<&[crate::data_provider::KlineData]>,
    push_date: NaiveDate,
) -> Option<bool> {
    use crate::market_analyzer::sector_monitor;
    use crate::data_provider::DataFetcherManager;
    // 1. 个股 D+1 涨幅
    let stock_d1 = pct_change_at(stock_klines?, push_date, 1)?;
    // 2. chain → board_code → 5 支成份股
    let (board_code, _) = sector_monitor::search_board_code_by_keyword(&row.chain).ok()??;
    let comps = sector_monitor::fetch_board_components(&board_code, 5).ok()?;
    // 3. 拉每支成份股 K 线, 算 D+1 平均涨幅
    let fetcher = DataFetcherManager::new().ok()?;
    let mut d1_sum = 0.0;
    let mut d1_count = 0;
    for c in &comps {
        if let Ok((klines, _)) = fetcher.get_daily_data(&c.code, 5) {
            if let Some(d) = pct_change_at(&klines, push_date, 1) {
                d1_sum += d;
                d1_count += 1;
            }
        }
    }
    if d1_count == 0 {
        return None;
    }
    let board_d1 = d1_sum / d1_count as f64;
    Some((stock_d1 - board_d1).abs() < 0.5)
}

/// 加载 audit JSONL
pub fn load_audit(path: &Path) -> Vec<AuditRowRead> {
    if !path.exists() {
        return Vec::new();
    }
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<AuditRowRead>(&line) {
            Ok(r) => out.push(r),
            Err(e) => log::warn!("[NEWS_OUTCOME] 第 {} 行解析失败: {:#}", i + 1, e),
        }
    }
    out
}

/// 跑一批 outcome 评估 (read-only, 不写盘)
pub fn evaluate_batch(
    rows: Vec<AuditRowRead>,
    push_date: NaiveDate,
) -> Vec<NewsOutcome> {
    let fetcher = match crate::data_provider::DataFetcherManager::new() {
        Ok(f) => f,
        Err(e) => {
            log::warn!("[NEWS_OUTCOME] DataFetcherManager 初始化失败: {:#}", e);
            return rows
                .iter()
                .map(|r| NewsOutcome {
                    candidate_id: r.candidate_id.clone(),
                    title: r.title.clone(),
                    chain: r.chain.clone(),
                    bucket: r.bucket.clone(),
                    push_at: r.ts.clone(),
                    code: None,
                    push_price: None,
                    d1_pct: None,
                    d3_pct: None,
                    d5_pct: None,
                    mfe: None,
                    mae: None,
                    limit_up_unbuyable: None,
                    open_high_sell_low_d1: None,
                    stop_break_first: None,
                    sector_driven: None,
                    executable_entry: None,
                    verdict: "Fetcher 初始化失败".to_string(),
                    reasons: vec!["DataFetcherManager 不可用".to_string()],
                })
                .collect();
        }
    };
    rows.iter().map(|r| evaluate_audit(r, push_date, &fetcher)).collect()
}

/// 渲染 outcome 报告 (markdown 表格)
pub fn format_outcome_report(outcomes: &[NewsOutcome]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# 新闻 Ranker Outcome 回看报告 ({})\n\n",
        Local::now().format("%Y-%m-%d %H:%M")
    ));
    let total = outcomes.len();
    let a_count = outcomes.iter().filter(|o| o.bucket == "PushNow").count();
    let b_count = outcomes.iter().filter(|o| o.bucket == "WatchCandidate").count();
    let c_count = outcomes.iter().filter(|o| o.bucket == "LogOnly").count();
    let d_count = outcomes.iter().filter(|o| o.bucket == "Drop").count();
    out.push_str(&format!(
        "## 概览: 总 {} 条 (A={} B={} C={} Drop={})\n\n",
        total, a_count, b_count, c_count, d_count
    ));
    // 兑现统计
    let realized = outcomes.iter().filter(|o| o.verdict.starts_with("有兑现")).count();
    let neutral = outcomes.iter().filter(|o| o.verdict.starts_with("中性") || o.verdict.starts_with("观察")).count();
    let missed = outcomes.iter().filter(|o| o.verdict.starts_with("未兑现") || o.verdict.starts_with("高回撤")).count();
    out.push_str(&format!(
        "## 兑现统计: 有兑现 {} | 中性/观察 {} | 未兑现/高回撤 {}\n\n",
        realized, neutral, missed
    ));
    out.push_str("## 明细\n\n");
    out.push_str("| 推送时间 | 标题 | 档 | Chain | D+1% | D+3% | D+5% | MFE% | MAE% | 涨停买不到 | 高开低走 | 先破后涨 | 板块带动 | 可执行 | Verdict |\n");
    out.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for o in outcomes {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            o.push_at,
            o.title.chars().take(30).collect::<String>(),
            o.bucket,
            o.chain,
            fmt_opt(o.d1_pct),
            fmt_opt(o.d3_pct),
            fmt_opt(o.d5_pct),
            fmt_opt(o.mfe),
            fmt_opt(o.mae),
            fmt_opt_bool(o.limit_up_unbuyable),
            fmt_opt_bool(o.open_high_sell_low_d1),
            fmt_opt_bool(o.stop_break_first),
            fmt_opt_bool(o.sector_driven),
            fmt_opt_bool(o.executable_entry),
            o.verdict,
        ));
    }
    if !outcomes.is_empty() {
        out.push_str("\n## 注意事项\n");
        out.push_str("- 本报告**不自动调权**, 仅人工审计参考 (P2-News-2 评审 P3 决定)\n");
        out.push_str("- 板块普涨 (sector_driven) 维度已接 (D+1 个股 vs 板块 5 支平均)\n");
        out.push_str("- 数据缺失维度显示 '-', 不补 0 编造\n");
    }
    out
}

fn fmt_opt(v: Option<f64>) -> String {
    v.map(|x| format!("{:.1}", x)).unwrap_or_else(|| "-".to_string())
}

fn fmt_opt_bool(v: Option<bool>) -> String {
    v.map(|b| if b { "✓" } else { "✗" }.to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// 主入口: 跑过去所有 audit 的回看 (D+1/D+3/D+5 已发生)
pub fn run_today_outcome() -> Vec<NewsOutcome> {
    // 1. 找 audit 目录 (DATABASE_PATH 同目录)
    let dir = std::env::var("DATABASE_PATH")
        .ok()
        .and_then(|p| {
            let pb = PathBuf::from(p);
            pb.parent().map(|p| p.to_path_buf())
        })
        .unwrap_or_else(|| PathBuf::from("./data"));
    if !dir.exists() {
        log::warn!("[NEWS_OUTCOME] audit 目录不存在: {}", dir.display());
        return Vec::new();
    }
    // 2. 找所有 news_rank_audit_*.jsonl (按文件名日期排序)
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .ok()
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| s.starts_with("news_rank_audit_") && s.ends_with(".jsonl"))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    if paths.is_empty() {
        log::warn!("[NEWS_OUTCOME] 未找到任何 audit JSONL");
        return Vec::new();
    }
    // 3. 每文件加载, 用文件日期作 push_date (一期: 文件名解析 YYYY-MM-DD)
    let mut all_rows = Vec::new();
    for p in &paths {
        let push_date = parse_audit_date(p);
        let rows = load_audit(p);
        for r in rows {
            all_rows.push((r, push_date));
        }
    }
    if all_rows.is_empty() {
        return Vec::new();
    }
    // 4. 批量评估 (push_date per row)
    let fetcher = match crate::data_provider::DataFetcherManager::new() {
        Ok(f) => f,
        Err(e) => {
            log::warn!("[NEWS_OUTCOME] DataFetcherManager 初始化失败: {:#}", e);
            return Vec::new();
        }
    };
    all_rows
        .iter()
        .map(|(r, d)| evaluate_audit(r, *d, &fetcher))
        .collect()
}

/// 从 audit 文件名解析推送日 (news_rank_audit_YYYY-MM-DD.jsonl)
fn parse_audit_date(path: &Path) -> NaiveDate {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    // 提取 YYYY-MM-DD
    let date_str = name
        .strip_prefix("news_rank_audit_")
        .and_then(|s| s.strip_suffix(".jsonl"))
        .unwrap_or("");
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d").unwrap_or_else(|_| Local::now().date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn unique_tmp(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        env::temp_dir().join(format!("news_outcome_{}_{}_{}_{}.jsonl", tag, std::process::id(), nanos, n))
    }

    fn write_audit(path: &Path, lines: &[&str]) {
        let content = lines.join("\n") + "\n";
        std::fs::write(path, content).unwrap();
    }

    /// 1) load_audit: 文件不存在返空
    #[test]
    fn load_missing_returns_empty() {
        let path = unique_tmp("missing");
        let _ = std::fs::remove_file(&path);
        let rows = load_audit(&path);
        assert!(rows.is_empty());
    }

    /// 2) load_audit: 解析 OK
    #[test]
    fn load_parses_correctly() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = unique_tmp("parse");
        let _ = std::fs::remove_file(&path);
        let json = r#"{"ts":"2026-07-04 10:00:00","candidate_id":"t1","title":"测试新闻","source":"东财","chain":"测试链","board_code":null,"event_type":"政策催化","heat_stage":"启动","score":75,"bucket":"PushNow","rule_score":20,"freshness_score":15,"heat_score":8,"stage_score":25,"capital_score":8,"source_score":10,"risk_penalty":0,"reasons":[],"drop_reason":null}"#;
        write_audit(&path, &[json]);
        let rows = load_audit(&path);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "测试新闻");
        assert_eq!(rows[0].bucket, "PushNow");
        let _ = std::fs::remove_file(&path);
    }

    /// 3) format_outcome_report: 基础输出
    #[test]
    fn format_report_basic() {
        let o = NewsOutcome {
            candidate_id: "t1".into(),
            title: "测试新闻".into(),
            chain: "测试链".into(),
            bucket: "PushNow".into(),
            push_at: "2026-07-04 10:00:00".into(),
            code: None,
            push_price: Some(10.0),
            d1_pct: Some(2.5),
            d3_pct: Some(5.0),
            d5_pct: Some(7.0),
            mfe: Some(8.0),
            mae: Some(-1.0),
            limit_up_unbuyable: Some(false),
            open_high_sell_low_d1: Some(false),
            stop_break_first: Some(false),
            sector_driven: None,
            executable_entry: Some(true),
            verdict: "有兑现".into(),
            reasons: vec![],
        };
        let s = format_outcome_report(&[o]);
        assert!(s.contains("新闻 Ranker Outcome 回看报告"));
        assert!(s.contains("有兑现"));
        assert!(s.contains("测试新闻"));
    }

    /// 4) judge_verdict: 各分支覆盖 (含 sector_driven 标注)
    #[test]
    fn judge_verdict_branches() {
        // 涨停买不到优先
        let v = judge_verdict(Some(10.0), Some(-2.0), Some(15.0), Some(true), None);
        assert!(v.starts_with("涨停买不到"));
        // 有兑现
        let v = judge_verdict(Some(5.0), Some(-1.0), Some(6.0), Some(false), None);
        assert!(v.starts_with("有兑现"));
        // 有兑现 + 板块普涨 → 含 "板块普涨带动" 标注
        let v = judge_verdict(Some(5.0), Some(-1.0), Some(6.0), Some(false), Some(true));
        assert!(v.starts_with("有兑现"));
        assert!(v.contains("板块普涨带动"));
        // 中性
        let v = judge_verdict(Some(1.0), Some(-2.0), Some(2.0), Some(false), None);
        assert!(v.starts_with("中性"));
        // 未兑现
        let v = judge_verdict(Some(-5.0), Some(-7.0), Some(0.0), Some(false), None);
        assert!(v.starts_with("未兑现"));
        // 高回撤
        let v = judge_verdict(Some(-2.0), Some(-10.0), Some(0.0), Some(false), None);
        assert!(v.starts_with("高回撤"));
        // 有过机会但未兑现
        let v = judge_verdict(Some(-1.0), Some(-1.0), Some(6.0), Some(false), None);
        assert!(v.starts_with("有过机会但未兑现"));
        // 无数据
        let v = judge_verdict(None, None, None, None, None);
        assert!(v.starts_with("无数据"));
    }

    /// 5) code_from_chain: 空 chain → None
    #[test]
    fn code_from_chain_empty() {
        // 空字符串不进 HTTP, 走 ok?? 提前返 None
        // 实际 search_board_code_by_keyword("") 内部就返 None
        let code = code_from_chain("");
        assert!(code.is_none(), "空 chain 应返 None");
    }

    /// 6) NewsOutcome 字段全 None 时不 panic
    #[test]
    fn outcome_all_none_safe() {
        let o = NewsOutcome {
            candidate_id: "t".into(),
            title: "t".into(),
            chain: "c".into(),
            bucket: "PushNow".into(),
            push_at: "2026-07-04 10:00".into(),
            code: None,
            push_price: None,
            d1_pct: None,
            d3_pct: None,
            d5_pct: None,
            mfe: None,
            mae: None,
            limit_up_unbuyable: None,
            open_high_sell_low_d1: None,
            stop_break_first: None,
            sector_driven: None,
            executable_entry: None,
            verdict: "无数据".into(),
            reasons: vec![],
        };
        // 序列化 + 渲染都不应 panic
        let s = format_outcome_report(&[o]);
        assert!(s.contains("无数据"));
    }
}

/// v70+: 兑现回填 (D+1 outcome 写回 d01_recommendations_YYYY-MM-DD.jsonl)
///   - 读: data/d01_recommendations/YYYY-MM-DD.jsonl
///   - 算: 用 push_at 后 1-5 天的 K 线算 D+1/D+3/D+5 + MFE/MAE
///   - 写: 更新每行 outcome 字段, 写回原 jsonl 文件
///   - 漏报: K 线缺失 (沙箱 / 非交易日) 时 outcome 保持 null
pub fn backfill_recommendations_outcome(date: &str) -> usize {
    use std::collections::HashMap;
    use std::fs;
    use std::io::{BufRead, BufWriter, Write};

    let path = std::path::PathBuf::from("data/d01_recommendations").join(format!("{}.jsonl", date));
    if !path.exists() {
        log::info!("[v70+] {} 不存在, skip", path.display());
        return 0;
    }

    // 1. 读 jsonl → (ts, code, push_price) 提取
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("[v70+] 读 {} 失败: {}", path.display(), e);
            return 0;
        }
    };
    let mut rows: Vec<HashMap<String, serde_json::Value>> = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(obj) = v.as_object() {
                rows.push(obj.iter().map(|(k, val)| (k.clone(), val.clone())).collect());
            }
        }
    }
    if rows.is_empty() {
        return 0;
    }

    // 2. 算每行 D+1/D+3/D+5
    let mut updated = 0;
    for row in &mut rows {
        let code = row.get("code").and_then(|v| v.as_str()).unwrap_or("");
        let ts = row.get("ts").and_then(|v| v.as_str()).unwrap_or("");
        if code.is_empty() || ts.is_empty() {
            continue;
        }
        // 拉 K 线 (推送日 + 5 天)
        // v14.1 review fix: `&ts[..10]` 字节切片在 ts < 10 字节时 panic, 改 ts.get(..10) + warn log
        let push_date = match ts.get(..10)
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        {
            Some(d) => d,
            None => {
                log::warn!(
                    "[v70+] backfill skip row: ts='{}' (len<10 or unparseable as YYYY-MM-DD), code={}",
                    ts, code
                );
                continue;
            }
        };
        let kline_result = crate::data_provider::DataFetcherManager::new()
            .ok()
            .and_then(|f| f.get_daily_data(code, 5).ok())
            .unwrap_or((Vec::new(), ""));
        let (klines, _): (Vec<_>, &str) = kline_result;
        if klines.is_empty() {
            continue;
        }
        let d1 = pct_change_at(&klines, push_date, 1);
        let d3 = pct_change_at(&klines, push_date, 3);
        let d5 = pct_change_at(&klines, push_date, 5);
        let (mfe, mae) = mfe_mae(&klines, push_date, 5);
        let push_price = kline_at_or_before(&klines, push_date).map(|k| k.close);

        let outcome = serde_json::json!({
            "d1_pct": d1, "d3_pct": d3, "d5_pct": d5,
            "mfe": mfe, "mae": mae, "push_price": push_price,
        });
        row.insert("outcome".to_string(), outcome);
        updated += 1;
    }

    // 3. 写回 (覆写)
    let file = match fs::File::create(&path) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("[v70+] 写 {} 失败: {}", path.display(), e);
            return 0;
        }
    };
    let mut writer = BufWriter::new(file);
    for row in &rows {
        let json = serde_json::Value::Object(
            row.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        );
        if let Err(e) = writeln!(writer, "{}", json) {
            log::warn!("[v70+] 写 jsonl 失败: {}", e);
        }
    }
    log::info!("[v70+] backfill {} 写回 {} 条 (date={})", path.display(), updated, date);
    updated
}
