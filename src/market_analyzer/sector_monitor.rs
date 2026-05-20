// -*- coding: utf-8 -*-
//! 板块监控与共振引擎
//!
//! 通过东方财富 push2 API 实时拉取概念板块涨幅榜与主力净流入榜，
//! 与宏观新闻 / AI 提示中出现的板块名称做共振判断，
//! 输出共振板块的龙头股，供候选股票池注入使用。
//!
//! 设计目标：解决“AI 不敢喊代码 → 风口被错过”的痛点：
//! - AI 只需要给出**板块名**（机器人、低空经济、固态电池…）
//! - Rust 端基于真实成交数据筛出**真正在涨且有资金的板块**
//! - 取这些板块的龙头加入分析池
//!
//! 共振规则（同时满足才视为强共振）：
//!  A. 板块当日涨幅排名前 N （表明热度真实存在）
//!  B. 主力资金净流入排名前 N （表明真金白银在买）
//!  C. 板块名称命中宏观新闻 / AI 推荐文本（可选加权）

use anyhow::{Context, Result};
use log::{info, warn};
use reqwest::blocking::Client;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// 东方财富概念板块条目
#[derive(Debug, Clone)]
pub struct ConceptBoard {
    /// 板块代码，例如 "BK0815"
    pub code: String,
    /// 板块名称，例如 "机器人概念"
    pub name: String,
    /// 当日涨跌幅 (%)
    pub change_pct: f64,
    /// 主力净流入金额 (元)
    pub main_inflow: f64,
    /// 领涨股名称（板块自带字段）
    pub leader_name: String,
}

/// 板块成份股条目（精简版）
#[derive(Debug, Clone)]
pub struct BoardStock {
    pub code: String,
    pub name: String,
    pub change_pct: f64,
    /// 当日成交额（元）
    pub amount: f64,
}

/// 共振板块及其龙头候选
#[derive(Debug, Clone)]
pub struct ResonanceSector {
    pub board: ConceptBoard,
    /// 命中的共振维度: "change" / "inflow" / "news"
    pub hit_dims: Vec<&'static str>,
    /// 该板块挑出的龙头候选
    pub leaders: Vec<BoardStock>,
}

/// 概念板块端点 (m:90+t:3 即"概念板块")
///
/// 东方财富 push2 主域名近期对部分公网出口存在 RST/断流问题，
/// 这里按优先级尝试多个候选主机，第一个返回有效 JSON 的胜出。
const BOARD_LIST_HOSTS: &[&str] = &[
    "push2delay.eastmoney.com",
    "push2.eastmoney.com",
    "82.push2.eastmoney.com",
];

fn build_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("创建HTTP客户端失败(sector_monitor)")
}

/// 在多个候选主机上依次发起同一 query，第一个成功的胜出。
fn get_with_fallback(client: &Client, params: &[(&str, &str)]) -> Result<Value> {
    let mut last_err: Option<anyhow::Error> = None;
    for host in BOARD_LIST_HOSTS {
        let url = format!("https://{}/api/qt/clist/get", host);
        let resp = client
            .get(&url)
            .query(params)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
            )
            .header("Referer", "https://quote.eastmoney.com/")
            .send();
        match resp.and_then(|r| r.text()) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(json) => {
                    if json.get("data").map(|d| !d.is_null()).unwrap_or(false) {
                        return Ok(json);
                    }
                    last_err = Some(anyhow::anyhow!("{} 响应 data=null", host));
                }
                Err(e) => last_err = Some(anyhow::anyhow!("{} JSON解析失败: {}", host, e)),
            },
            Err(e) => last_err = Some(anyhow::anyhow!("{} 请求失败: {}", host, e)),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("所有候选主机均不可达")))
}

/// 按指定字段从东财概念板块榜拉取前 `top_n` 名
///
/// `fid` 取值：
/// - "f3"  按涨跌幅排序
/// - "f62" 按主力净流入排序
fn fetch_board_ranking(fid: &str, top_n: usize) -> Result<Vec<ConceptBoard>> {
    let client = build_client()?;
    let pz = top_n.clamp(10, 200).to_string();
    let params: [(&str, &str); 11] = [
        ("pn", "1"),
        ("pz", pz.as_str()),
        ("po", "1"),
        ("np", "1"),
        ("ut", "bd1d9ddb04089700cf9c27f6f7426281"),
        ("fltt", "2"),
        ("invt", "2"),
        ("fid", fid),
        ("fs", "m:90+t:3+f:!50"),
        // f3=涨幅, f12=代码, f14=名称, f62=主力净流入, f128=领涨股名
        ("fields", "f3,f12,f14,f62,f128"),
        ("_", "1"),
    ];

    let json = get_with_fallback(&client, &params).context("拉取概念板块榜失败")?;

    let diff = match json
        .get("data")
        .and_then(|d| d.get("diff"))
        .and_then(|d| d.as_array())
    {
        Some(arr) => arr,
        None => return Ok(Vec::new()),
    };

    let mut boards = Vec::with_capacity(diff.len());
    for item in diff {
        let code = item.get("f12").and_then(|v| v.as_str()).unwrap_or("");
        let name = item.get("f14").and_then(|v| v.as_str()).unwrap_or("");
        if code.is_empty() || name.is_empty() {
            continue;
        }
        let change_pct = item.get("f3").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let main_inflow = item.get("f62").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let leader_name = item
            .get("f128")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        boards.push(ConceptBoard {
            code: code.to_string(),
            name: name.to_string(),
            change_pct,
            main_inflow,
            leader_name,
        });
    }
    Ok(boards)
}

/// 拉取板块成份股，按成交额降序，取前 `top_n` 名
pub fn fetch_board_components(board_code: &str, top_n: usize) -> Result<Vec<BoardStock>> {
    let client = build_client()?;
    let pz = top_n.clamp(5, 100).to_string();
    let fs = format!("b:{}", board_code);
    let params: [(&str, &str); 11] = [
        ("pn", "1"),
        ("pz", pz.as_str()),
        ("po", "1"),
        ("np", "1"),
        ("ut", "bd1d9ddb04089700cf9c27f6f7426281"),
        ("fltt", "2"),
        ("invt", "2"),
        ("fid", "f6"), // 按成交额排序
        ("fs", fs.as_str()),
        // f2=价, f3=涨跌幅, f6=成交额, f12=代码, f14=名称
        ("fields", "f2,f3,f6,f12,f14"),
        ("_", "1"),
    ];

    let json = get_with_fallback(&client, &params)
        .with_context(|| format!("拉取板块 {} 成份股失败", board_code))?;

    let diff = match json
        .get("data")
        .and_then(|d| d.get("diff"))
        .and_then(|d| d.as_array())
    {
        Some(arr) => arr,
        None => return Ok(Vec::new()),
    };

    let mut stocks = Vec::with_capacity(diff.len());
    for item in diff {
        let code = item.get("f12").and_then(|v| v.as_str()).unwrap_or("");
        let name = item.get("f14").and_then(|v| v.as_str()).unwrap_or("");
        if code.is_empty() || name.is_empty() {
            continue;
        }
        // 过滤 ST / 北交所 (保持与 limit_up 一致的口径)
        if name.contains("ST") || name.contains("st") {
            continue;
        }
        if code.starts_with('8') || code.starts_with('4') || code.starts_with('9') {
            continue;
        }
        let change_pct = item.get("f3").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let amount = item.get("f6").and_then(|v| v.as_f64()).unwrap_or(0.0);
        stocks.push(BoardStock {
            code: code.to_string(),
            name: name.to_string(),
            change_pct,
            amount,
        });
    }
    Ok(stocks)
}

/// 检测共振板块
///
/// 参数：
/// - `news_text`：宏观新闻 / AI 推荐拼接文本，用于做新闻共振判断
/// - `rank_top`：涨幅榜 / 资金榜各取前 N 名
/// - `max_sectors`：最终返回的共振板块数量上限
/// - `leaders_per_sector`：每个板块输出的龙头数量
///
/// 返回的板块满足以下任一强条件：
///   1. 同时出现在「涨幅榜前 N」与「主力净流入榜前 N」 → change+inflow 共振
///   2. 出现在涨幅榜前 N 且板块名命中新闻文本 → change+news 共振
///   3. 出现在资金榜前 N 且板块名命中新闻文本 → inflow+news 共振
pub fn detect_resonance_sectors(
    news_text: &str,
    rank_top: usize,
    max_sectors: usize,
    leaders_per_sector: usize,
) -> Result<Vec<ResonanceSector>> {
    let by_change = fetch_board_ranking("f3", rank_top)?;
    let by_inflow = fetch_board_ranking("f62", rank_top)?;

    if by_change.is_empty() && by_inflow.is_empty() {
        warn!("[共振] 概念板块榜为空，跳过共振检测");
        return Ok(Vec::new());
    }

    // 用 code 作为板块唯一键，合并两路榜单
    let mut map: HashMap<String, (ConceptBoard, Vec<&'static str>)> = HashMap::new();
    let change_set: HashSet<&String> = by_change.iter().map(|b| &b.code).collect();
    let inflow_set: HashSet<&String> = by_inflow.iter().map(|b| &b.code).collect();

    for b in by_change.iter().chain(by_inflow.iter()) {
        map.entry(b.code.clone()).or_insert_with(|| (b.clone(), Vec::new()));
    }

    for (code, (board, dims)) in map.iter_mut() {
        if change_set.contains(code) {
            dims.push("change");
        }
        if inflow_set.contains(code) {
            dims.push("inflow");
        }
        if !news_text.is_empty() && news_match(news_text, &board.name) {
            dims.push("news");
        }
    }

    // 仅保留至少命中 2 个维度的板块
    let mut hits: Vec<(ConceptBoard, Vec<&'static str>)> = map
        .into_values()
        .filter(|(_, dims)| dims.len() >= 2)
        .collect();

    // 排序：news 命中优先，其次 change+inflow 同时命中，最后按涨幅
    hits.sort_by(|a, b| {
        let score_a = score_dims(&a.1, a.0.change_pct, a.0.main_inflow);
        let score_b = score_dims(&b.1, b.0.change_pct, b.0.main_inflow);
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    hits.truncate(max_sectors);

    let mut result = Vec::with_capacity(hits.len());
    for (board, dims) in hits {
        match fetch_board_components(&board.code, leaders_per_sector.max(3) * 2) {
            Ok(comps) => {
                let leaders = pick_leaders(&comps, leaders_per_sector);
                info!(
                    "[共振] 板块 {}({}) 命中维度{:?} 涨幅={:.2}% 资金={:.2}亿 龙头={}",
                    board.name,
                    board.code,
                    dims,
                    board.change_pct,
                    board.main_inflow / 1e8,
                    leaders
                        .iter()
                        .map(|s| format!("{}({})", s.name, s.code))
                        .collect::<Vec<_>>()
                        .join(",")
                );
                result.push(ResonanceSector {
                    board,
                    hit_dims: dims,
                    leaders,
                });
            }
            Err(e) => {
                warn!("[共振] 拉取板块 {} 成份股失败: {}", board.code, e);
            }
        }
    }

    Ok(result)
}

/// 把共振板块龙头展平为去重后的股票代码列表（保持插入顺序）
pub fn collect_leader_codes(sectors: &[ResonanceSector]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut codes = Vec::new();
    for s in sectors {
        for stk in &s.leaders {
            if seen.insert(stk.code.clone()) {
                codes.push(stk.code.clone());
            }
        }
    }
    codes
}

// ---------- 内部工具 ----------

/// 板块名匹配新闻文本：先 substring，再做一层别名/关键词包含
fn news_match(news_text: &str, board_name: &str) -> bool {
    if news_text.contains(board_name) {
        return true;
    }
    // 板块名常以“xx概念/xx板块”结尾，去掉这些尾缀再试
    for tail in ["概念", "板块", "产业链"] {
        if let Some(stem) = board_name.strip_suffix(tail) {
            if !stem.is_empty() && news_text.contains(stem) {
                return true;
            }
        }
    }
    false
}

/// 共振维度评分：用于排序
fn score_dims(dims: &[&'static str], change_pct: f64, main_inflow: f64) -> f64 {
    let mut score = 0.0;
    if dims.contains(&"news") {
        score += 100.0;
    }
    if dims.contains(&"change") && dims.contains(&"inflow") {
        score += 50.0;
    }
    score += change_pct;             // 单位 %
    score += (main_inflow / 1e8).max(-50.0); // 单位 亿
    score
}

/// 龙头筛选：先取成交额 Top3，再补充涨幅 Top2（去重）
fn pick_leaders(comps: &[BoardStock], leaders_per_sector: usize) -> Vec<BoardStock> {
    if comps.is_empty() || leaders_per_sector == 0 {
        return Vec::new();
    }
    let take_amount = leaders_per_sector.min(3).max(1);
    let take_change = leaders_per_sector.saturating_sub(take_amount).min(2);

    let mut by_amount: Vec<&BoardStock> = comps.iter().collect();
    by_amount.sort_by(|a, b| b.amount.partial_cmp(&a.amount).unwrap_or(std::cmp::Ordering::Equal));

    let mut by_change: Vec<&BoardStock> = comps.iter().collect();
    by_change.sort_by(|a, b| {
        b.change_pct
            .partial_cmp(&a.change_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut seen = HashSet::new();
    let mut picks = Vec::new();
    for s in by_amount.iter().take(take_amount) {
        if seen.insert(s.code.clone()) {
            picks.push((*s).clone());
        }
    }
    for s in by_change.iter() {
        if picks.len() >= take_amount + take_change {
            break;
        }
        if seen.insert(s.code.clone()) {
            picks.push((*s).clone());
        }
    }
    picks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn news_match_basic() {
        assert!(news_match("今日机器人产业链爆发", "机器人概念"));
        assert!(news_match("低空经济政策落地", "低空经济"));
        assert!(!news_match("猪肉价格回升", "机器人概念"));
    }

    #[test]
    fn pick_leaders_dedup() {
        let comps = vec![
            BoardStock { code: "000001".into(), name: "A".into(), change_pct: 5.0, amount: 1e9 },
            BoardStock { code: "000002".into(), name: "B".into(), change_pct: 9.0, amount: 8e8 },
            BoardStock { code: "000003".into(), name: "C".into(), change_pct: 2.0, amount: 7e8 },
            BoardStock { code: "000004".into(), name: "D".into(), change_pct: 8.0, amount: 1e8 },
        ];
        let picks = pick_leaders(&comps, 5);
        // top3 by amount: 1,2,3 ; +top2 by change: 2(已选), 4
        let codes: Vec<_> = picks.iter().map(|s| s.code.as_str()).collect();
        assert_eq!(codes, vec!["000001", "000002", "000003", "000004"]);
    }
}
