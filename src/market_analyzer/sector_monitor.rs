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
    /// 量比 (f10)：今日成交节奏 / 近5日均值；>1 表示放量
    pub vol_ratio: f64,
    /// 换手率 (f8, %)
    pub turnover: f64,
    /// 今日主力净占比 (f184, %)
    pub main_net_pct_today: f64,
    /// 5日主力净占比 (f165, %)
    pub main_net_pct_5d: f64,
}

impl ConceptBoard {
    /// 资金流加速度（百分点）：今日主力净占比 − 5日主力净占比。
    /// >0 表示主力资金当下正在加速流入，是典型的领先（先行）信号。
    pub fn inflow_accel(&self) -> f64 {
        self.main_net_pct_today - self.main_net_pct_5d
    }
}

/// 领先信号阈值配置（全部可由环境变量覆盖）
#[derive(Debug, Clone)]
pub struct LeadingConfig {
    /// 是否启用领先信号；false 时回退为旧的"涨幅∩净流入∩新闻"行为
    pub enabled: bool,
    /// 资金加速度达标阈值（百分点）
    pub accel_min: f64,
    /// 资金加速度强信号阈值（单维即可判定领先）
    pub accel_strong: f64,
    /// 量比达标阈值
    pub vol_ratio_min: f64,
    /// 量比强信号阈值（单维即可判定领先）
    pub vol_ratio_strong: f64,
    /// 板块已涨幅超过此值视为过热，评分降权（避免继续追顶）
    pub overextended_pct: f64,
}

impl Default for LeadingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            accel_min: 3.0,
            accel_strong: 8.0,
            vol_ratio_min: 1.3,
            vol_ratio_strong: 2.5,
            overextended_pct: 9.0,
        }
    }
}

impl LeadingConfig {
    /// 从环境变量读取，缺省回落到 `Default`。
    pub fn from_env() -> Self {
        let d = Self::default();
        let f = |key: &str, def: f64| -> f64 {
            std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(def)
        };
        Self {
            enabled: std::env::var("SECTOR_LEAD_ENABLED")
                .map(|v| v.to_lowercase() != "false")
                .unwrap_or(d.enabled),
            accel_min: f("SECTOR_LEAD_ACCEL_MIN", d.accel_min),
            accel_strong: f("SECTOR_LEAD_ACCEL_STRONG", d.accel_strong),
            vol_ratio_min: f("SECTOR_LEAD_VOLRATIO_MIN", d.vol_ratio_min),
            vol_ratio_strong: f("SECTOR_LEAD_VOLRATIO_STRONG", d.vol_ratio_strong),
            overextended_pct: f("SECTOR_OVEREXTENDED_PCT", d.overextended_pct),
        }
    }
}

/// 龙头筛选配置：在保留少量真龙头的同时，注入「板块涨但个股未启动」的低位卡位/补涨候选。
///
/// 旧逻辑只取成交额 Top 的高位龙头，等于追板块里已被买爆的票。低位卡位逻辑：
/// 当板块整体在涨（`board_min_change`）时，挑出**自身涨幅还低、但量比已抬头、
/// 流动性足够**的成份股——它们是题材扩散时最可能补涨的卡位标的。
#[derive(Debug, Clone)]
pub struct LeaderConfig {
    /// 是否启用低位卡位注入；false 时回退为纯成交额/涨幅龙头
    pub lowpos_enabled: bool,
    /// 板块当日涨幅达到此值才触发低位卡位（题材确实在发酵）
    pub board_min_change: f64,
    /// 保留的真龙头数量（成交额 Top）
    pub momentum_keep: usize,
    /// 低位个股涨幅上限（超过则视为已启动，不算"卡位"）
    pub lowpos_max_change: f64,
    /// 低位个股涨幅下限（低于则视为走弱，排除）
    pub lowpos_min_change: f64,
    /// 低位个股量比下限（资金开始关注；字段缺失=0 时不据此排除）
    pub lowpos_min_vol_ratio: f64,
    /// 低位个股成交额下限（元，保证可交易流动性）
    pub lowpos_min_amount: f64,
}

impl Default for LeaderConfig {
    fn default() -> Self {
        Self {
            lowpos_enabled: true,
            board_min_change: 2.0,
            momentum_keep: 2,
            lowpos_max_change: 5.0,
            lowpos_min_change: -2.0,
            lowpos_min_vol_ratio: 1.0,
            lowpos_min_amount: 1e8,
        }
    }
}

impl LeaderConfig {
    pub fn from_env() -> Self {
        let d = Self::default();
        let f = |key: &str, def: f64| -> f64 {
            std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(def)
        };
        let u = |key: &str, def: usize| -> usize {
            std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(def)
        };
        Self {
            lowpos_enabled: std::env::var("SECTOR_LOWPOS_ENABLED")
                .map(|v| v.to_lowercase() != "false")
                .unwrap_or(d.lowpos_enabled),
            board_min_change: f("SECTOR_LOWPOS_BOARD_MIN", d.board_min_change),
            momentum_keep: u("SECTOR_MOMENTUM_KEEP", d.momentum_keep),
            lowpos_max_change: f("SECTOR_LOWPOS_MAX_CHANGE", d.lowpos_max_change),
            lowpos_min_change: f("SECTOR_LOWPOS_MIN_CHANGE", d.lowpos_min_change),
            lowpos_min_vol_ratio: f("SECTOR_LOWPOS_MIN_VOLRATIO", d.lowpos_min_vol_ratio),
            lowpos_min_amount: f("SECTOR_LOWPOS_MIN_AMOUNT", d.lowpos_min_amount),
        }
    }
}

/// 板块成份股条目（精简版）
#[derive(Debug, Clone)]
pub struct BoardStock {
    pub code: String,
    pub name: String,
    pub change_pct: f64,
    /// 当日成交额（元）
    pub amount: f64,
    /// 量比 (f10)：>1 表示今日成交较近5日放大，资金开始关注
    pub vol_ratio: f64,
    /// 换手率 (f8, %)
    pub turnover: f64,
}

/// 板块点火广度统计（首板溢价的板块级代理）
///
/// 真·连板高度需要涨停历史接口，这里用「板块成份股中涨停 / 接近涨停的家数」
/// 作为板块级近似：点火广度越大，说明题材正在新点火（领先），而非单一龙头独涨。
#[derive(Debug, Clone, Default)]
pub struct IgnitionStats {
    /// 涨停家数（按各板涨跌停阈值判定）
    pub limit_up_count: usize,
    /// 接近涨停家数（涨幅 ≥ 阈值-1.5%，含涨停）
    pub near_limit_count: usize,
    /// 参与统计的成份股样本数
    pub sample: usize,
}

/// 共振板块及其龙头候选
#[derive(Debug, Clone)]
pub struct ResonanceSector {
    pub board: ConceptBoard,
    /// 命中的共振维度: "change" / "inflow" / "news" / "lead"
    pub hit_dims: Vec<&'static str>,
    /// 该板块挑出的龙头候选
    pub leaders: Vec<BoardStock>,
    /// 点火广度统计（板块级首板溢价代理）
    pub ignition: IgnitionStats,
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
///
/// 两路调用均请求完整字段集（涨幅/净流入 + 量比/换手 + 今日/5日主力净占比），
/// 保证无论板块来自哪一路榜单都携带完整的领先信号，便于后续共振判定。
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
        // f3=涨幅, f8=换手率, f10=量比, f12=代码, f14=名称, f62=主力净流入,
        // f128=领涨股名, f184=今日主力净占比, f165=5日主力净占比
        ("fields", "f3,f8,f10,f12,f14,f62,f128,f184,f165"),
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
        // 领先信号字段：缺失/非数值时回落为 0，后续逻辑按"不达标"处理，不会误判。
        let vol_ratio = item.get("f10").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let turnover = item.get("f8").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let main_net_pct_today = item.get("f184").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let main_net_pct_5d = item.get("f165").and_then(|v| v.as_f64()).unwrap_or(0.0);

        boards.push(ConceptBoard {
            code: code.to_string(),
            name: name.to_string(),
            change_pct,
            main_inflow,
            leader_name,
            vol_ratio,
            turnover,
            main_net_pct_today,
            main_net_pct_5d,
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
        // f2=价, f3=涨跌幅, f6=成交额, f8=换手率, f10=量比, f12=代码, f14=名称
        ("fields", "f2,f3,f6,f8,f10,f12,f14"),
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
        let vol_ratio = item.get("f10").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let turnover = item.get("f8").and_then(|v| v.as_f64()).unwrap_or(0.0);
        stocks.push(BoardStock {
            code: code.to_string(),
            name: name.to_string(),
            change_pct,
            amount,
            vol_ratio,
            turnover,
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
/// 共振维度：
///   - `change`：当日涨幅榜前 N
///   - `inflow`：主力净流入榜前 N
///   - `news`  ：板块名命中宏观新闻 / AI 推荐
///   - `lead`  ：**领先信号**（资金加速 / 量比突变），表明题材正在点火、尚未涨透
///
/// 板块需至少命中 2 个维度。引入 `lead` 维度后，「资金正在加速 + 题材」或
/// 「资金正在加速 + 已在涨幅/资金榜」即可入选，从而比旧的「已涨∩已净流入」更早捕捉热点。
pub fn detect_resonance_sectors(
    news_text: &str,
    rank_top: usize,
    max_sectors: usize,
    leaders_per_sector: usize,
) -> Result<Vec<ResonanceSector>> {
    let cfg = LeadingConfig::from_env();
    let leader_cfg = LeaderConfig::from_env();
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
        if cfg.enabled && is_leading(board, &cfg) {
            dims.push("lead");
        }
    }

    // 仅保留至少命中 2 个维度的板块
    let mut hits: Vec<(ConceptBoard, Vec<&'static str>)> = map
        .into_values()
        .filter(|(_, dims)| dims.len() >= 2)
        .collect();

    // 排序：领先信号 / news 优先，其次 change+inflow，再叠加动量奖励与过热惩罚
    hits.sort_by(|a, b| {
        let score_a = score_board(&a.1, &a.0, &cfg);
        let score_b = score_board(&b.1, &b.0, &cfg);
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    hits.truncate(max_sectors);

    let mut result = Vec::with_capacity(hits.len());
    for (board, dims) in hits {
        match fetch_board_components(&board.code, leaders_per_sector.max(3) * 2) {
            Ok(comps) => {
                let ignition = compute_ignition(&comps);
                let leaders = pick_leaders(&comps, leaders_per_sector, board.change_pct, &leader_cfg);
                info!(
                    "[共振] 板块 {}({}) 维度{:?} 涨幅={:.2}% 资金={:.2}亿 加速={:+.2}pp 量比={:.2} 点火(涨停{}/接近{}) 龙头={}",
                    board.name,
                    board.code,
                    dims,
                    board.change_pct,
                    board.main_inflow / 1e8,
                    board.inflow_accel(),
                    board.vol_ratio,
                    ignition.limit_up_count,
                    ignition.near_limit_count,
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
                    ignition,
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

/// 判定板块是否具备**领先（动量先行）信号**。
///
/// 触发任一条件即视为领先：
///  - 资金加速度 ≥ `accel_min` **且** 量比 ≥ `vol_ratio_min`（双弱共振）
///  - 资金加速度 ≥ `accel_strong`（单强：资金当下猛烈加速）
///  - 量比 ≥ `vol_ratio_strong`（单强：今日异常放量）
///
/// 字段缺失时（值为 0）自然不达标，不会误判为领先。
fn is_leading(b: &ConceptBoard, cfg: &LeadingConfig) -> bool {
    let accel = b.inflow_accel();
    if accel >= cfg.accel_min && b.vol_ratio >= cfg.vol_ratio_min {
        return true;
    }
    if accel >= cfg.accel_strong {
        return true;
    }
    if b.vol_ratio >= cfg.vol_ratio_strong {
        return true;
    }
    false
}

/// 板块领先强度（连续量），用于排序时奖励"正在加速"的板块。
fn leading_score(b: &ConceptBoard) -> f64 {
    let accel = b.inflow_accel();
    // 量比以 1.0 为基准（1 表示与近5日持平），高于 1 才算放量。
    let vol_excess = (b.vol_ratio - 1.0).max(0.0);
    accel.max(0.0) * 3.0 + vol_excess * 12.0
}

/// 综合评分（用于命中板块排序）。
///
/// 相比旧的纯滞后评分，新增：
///  - `lead` 维度加权（领先信号最高优先级）
///  - 连续动量奖励 `leading_score`
///  - **过热惩罚**：已涨幅超过 `overextended_pct` 的板块降权，避免继续追顶。
fn score_board(dims: &[&'static str], b: &ConceptBoard, cfg: &LeadingConfig) -> f64 {
    let mut score = 0.0;

    // 领先信号优先级最高：资金正在加速且题材匹配，是最理想的早期介入点。
    if dims.contains(&"lead") && dims.contains(&"news") {
        score += 120.0;
    } else if dims.contains(&"lead") {
        score += 80.0;
    }
    if dims.contains(&"news") {
        score += 100.0;
    }
    if dims.contains(&"change") && dims.contains(&"inflow") {
        score += 50.0;
    }

    // 连续动量奖励
    if cfg.enabled {
        score += leading_score(b);
    }

    // 基础项
    score += b.change_pct; // 单位 %
    score += (b.main_inflow / 1e8).max(-50.0); // 单位 亿

    // 过热惩罚：已涨太多说明右侧偏晚，线性降权（每超 1% 扣 5 分）
    if b.change_pct > cfg.overextended_pct {
        score -= (b.change_pct - cfg.overextended_pct) * 5.0;
    }

    score
}

/// 从板块成份股计算点火广度（首板溢价的板块级代理）。
///
/// 用各板涨跌停阈值判定涨停 / 接近涨停家数；不依赖涨停历史接口。
fn compute_ignition(comps: &[BoardStock]) -> IgnitionStats {
    let mut limit_up_count = 0usize;
    let mut near_limit_count = 0usize;
    for s in comps {
        let limit_pct = component_limit_pct(&s.code, &s.name);
        if s.change_pct >= limit_pct - 0.15 {
            limit_up_count += 1;
        }
        if s.change_pct >= limit_pct - 1.5 {
            near_limit_count += 1;
        }
    }
    IgnitionStats {
        limit_up_count,
        near_limit_count,
        sample: comps.len(),
    }
}

/// 成份股涨跌停幅度限制（与 MarketAnalyzer::get_limit_pct 同口径，
/// 本模块已过滤 ST/北交所，这里只区分主板与创业板/科创板）。
fn component_limit_pct(code: &str, name: &str) -> f64 {
    if name.contains("ST") || name.contains("st") {
        5.0
    } else if code.starts_with("30") || code.starts_with("688") {
        20.0
    } else {
        10.0
    }
}

/// 龙头筛选（带低位卡位注入）。
///
/// - 当低位卡位关闭或板块整体未明显上涨时，回退到 `pick_leaders_legacy`
///   （成交额 Top3 + 涨幅 Top2）。
/// - 当板块在涨时：先保留 `momentum_keep` 只真龙头（成交额 Top），
///   其余名额用**低位卡位/补涨候选**填充（自身未启动、量比抬头、流动性达标），
///   不足再用涨幅 Top 兜底。
fn pick_leaders(
    comps: &[BoardStock],
    leaders_per_sector: usize,
    board_change_pct: f64,
    cfg: &LeaderConfig,
) -> Vec<BoardStock> {
    if comps.is_empty() || leaders_per_sector == 0 {
        return Vec::new();
    }
    if !cfg.lowpos_enabled || board_change_pct < cfg.board_min_change {
        return pick_leaders_legacy(comps, leaders_per_sector);
    }

    let mut by_amount: Vec<&BoardStock> = comps.iter().collect();
    by_amount.sort_by(|a, b| b.amount.partial_cmp(&a.amount).unwrap_or(std::cmp::Ordering::Equal));

    let mut seen = HashSet::new();
    let mut picks: Vec<BoardStock> = Vec::new();

    // 1. 保留真龙头（成交额 Top）
    let keep = cfg.momentum_keep.min(leaders_per_sector);
    for s in by_amount.iter().take(keep) {
        if seen.insert(s.code.clone()) {
            picks.push((*s).clone());
        }
    }

    // 2. 低位卡位/补涨候选填充剩余名额
    for s in pick_low_position(comps, &seen, cfg) {
        if picks.len() >= leaders_per_sector {
            break;
        }
        if seen.insert(s.code.clone()) {
            picks.push(s.clone());
        }
    }

    // 3. 仍不足 → 用涨幅 Top 兜底
    if picks.len() < leaders_per_sector {
        let mut by_change: Vec<&BoardStock> = comps.iter().collect();
        by_change.sort_by(|a, b| {
            b.change_pct
                .partial_cmp(&a.change_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for s in by_change {
            if picks.len() >= leaders_per_sector {
                break;
            }
            if seen.insert(s.code.clone()) {
                picks.push(s.clone());
            }
        }
    }

    picks
}

/// 低位卡位/补涨候选：板块在涨、但个股自身尚未启动的成份股。
///
/// 过滤条件：未被选中、涨幅在 [min,max] 区间（未涨透且未走弱）、
/// 成交额达流动性下限、量比抬头（字段缺失=0 时不据此排除）。
/// 排序：涨幅升序（最未启动者优先），同涨幅按成交额降序（更易交易）。
fn pick_low_position<'a>(
    comps: &'a [BoardStock],
    seen: &HashSet<String>,
    cfg: &LeaderConfig,
) -> Vec<&'a BoardStock> {
    let mut cands: Vec<&BoardStock> = comps
        .iter()
        .filter(|s| !seen.contains(&s.code))
        .filter(|s| s.change_pct <= cfg.lowpos_max_change && s.change_pct >= cfg.lowpos_min_change)
        .filter(|s| s.amount >= cfg.lowpos_min_amount)
        .filter(|s| s.vol_ratio >= cfg.lowpos_min_vol_ratio || s.vol_ratio == 0.0)
        .collect();

    cands.sort_by(|a, b| {
        a.change_pct
            .partial_cmp(&b.change_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                b.amount
                    .partial_cmp(&a.amount)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    cands
}

/// 龙头筛选（旧逻辑）：先取成交额 Top3，再补充涨幅 Top2（去重）
fn pick_leaders_legacy(comps: &[BoardStock], leaders_per_sector: usize) -> Vec<BoardStock> {
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

    fn bs(code: &str, change: f64, amount: f64, vol: f64) -> BoardStock {
        BoardStock {
            code: code.into(),
            name: code.into(),
            change_pct: change,
            amount,
            vol_ratio: vol,
            turnover: 0.0,
        }
    }

    #[test]
    fn pick_leaders_legacy_when_board_flat() {
        let comps = vec![
            bs("000001", 5.0, 1e9, 1.0),
            bs("000002", 9.0, 8e8, 1.0),
            bs("000003", 2.0, 7e8, 1.0),
            bs("000004", 8.0, 1e8, 1.0),
        ];
        // 板块涨幅 0 < board_min_change → 回退旧逻辑（成交额Top3 + 涨幅Top2）
        let picks = pick_leaders(&comps, 5, 0.0, &LeaderConfig::default());
        let codes: Vec<_> = picks.iter().map(|s| s.code.as_str()).collect();
        assert_eq!(codes, vec!["000001", "000002", "000003", "000004"]);
    }

    #[test]
    fn pick_leaders_injects_low_position_when_board_rising() {
        // 板块大涨 6%：高位龙头 + 低位卡位混合
        let comps = vec![
            bs("600001", 9.8, 1e9, 1.2),  // 高位龙头（成交额最大）
            bs("600002", 9.5, 9e8, 1.1),  // 高位次龙头
            bs("600003", 1.0, 5e8, 1.6),  // 低位卡位：未启动+放量+够流动性 ✓
            bs("600004", 0.3, 3e8, 1.4),  // 低位卡位：更未启动 ✓（涨幅更低，优先）
            bs("600005", 0.5, 1e6, 2.0),  // 低位但成交额太小（<1亿）✗
            bs("600006", -5.0, 4e8, 1.5), // 走弱（<min_change）✗
        ];
        let picks = pick_leaders(&comps, 4, 6.0, &LeaderConfig::default());
        let codes: Vec<_> = picks.iter().map(|s| s.code.as_str()).collect();
        // momentum_keep=2 → 600001,600002；低位按涨幅升序 → 600004(0.3),600003(1.0)
        assert_eq!(codes, vec!["600001", "600002", "600004", "600003"]);
    }

    #[test]
    fn low_position_filters_started_and_illiquid() {
        let comps = vec![
            bs("600010", 7.0, 5e8, 1.5), // 已涨透(>max_change=5) ✗
            bs("600011", 2.0, 5e7, 1.5), // 流动性不足(<1亿) ✗
            bs("600012", 1.0, 5e8, 1.5), // 合格 ✓
        ];
        let seen = HashSet::new();
        let cands = pick_low_position(&comps, &seen, &LeaderConfig::default());
        let codes: Vec<_> = cands.iter().map(|s| s.code.as_str()).collect();
        assert_eq!(codes, vec!["600012"]);
    }

    fn board(change: f64, today_pct: f64, pct5: f64, vol: f64) -> ConceptBoard {
        ConceptBoard {
            code: "BK0001".into(),
            name: "测试概念".into(),
            change_pct: change,
            main_inflow: 1e8,
            leader_name: String::new(),
            vol_ratio: vol,
            turnover: 0.0,
            main_net_pct_today: today_pct,
            main_net_pct_5d: pct5,
        }
    }

    #[test]
    fn inflow_accel_is_today_minus_5d() {
        let b = board(3.0, 6.0, 1.0, 1.2);
        assert!((b.inflow_accel() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn is_leading_dual_weak_resonance() {
        let cfg = LeadingConfig::default();
        // accel = 4.0 ≥ 3.0 且 量比 1.4 ≥ 1.3 → 领先
        assert!(is_leading(&board(2.0, 5.0, 1.0, 1.4), &cfg));
        // accel 达标但量比不足 → 非领先
        assert!(!is_leading(&board(2.0, 5.0, 1.0, 1.0), &cfg));
    }

    #[test]
    fn is_leading_single_strong_signals() {
        let cfg = LeadingConfig::default();
        // 仅资金强加速（accel=9 ≥ 8），量比平平
        assert!(is_leading(&board(1.0, 10.0, 1.0, 1.0), &cfg));
        // 仅量比异常放量（2.6 ≥ 2.5），资金未加速
        assert!(is_leading(&board(1.0, 1.0, 1.0, 2.6), &cfg));
    }

    #[test]
    fn missing_fields_not_leading() {
        let cfg = LeadingConfig::default();
        // 字段缺失全为 0 → 不应误判为领先
        assert!(!is_leading(&board(0.0, 0.0, 0.0, 0.0), &cfg));
    }

    #[test]
    fn lead_news_outscores_pure_lagging() {
        let cfg = LeadingConfig::default();
        // 领先+题材的早期板块（涨幅不高）
        let early = board(3.0, 8.0, 1.0, 1.8);
        let early_score = score_board(&["lead", "news"], &early, &cfg);
        // 纯滞后：已在涨幅+资金榜但已涨透、无加速
        let late = board(9.0, 2.0, 2.5, 0.9);
        let late_score = score_board(&["change", "inflow"], &late, &cfg);
        assert!(early_score > late_score, "早期领先板块应排在滞后板块之前");
    }

    #[test]
    fn overextended_penalty_applies() {
        let cfg = LeadingConfig::default();
        let dims = ["change", "inflow"];
        // 仅涨幅不同：12% 已过热(>9%) 应低于 6% 的得分
        let hot = board(12.0, 0.0, 0.0, 1.0);
        let mild = board(6.0, 0.0, 0.0, 1.0);
        assert!(score_board(&dims, &mild, &cfg) > score_board(&dims, &hot, &cfg));
    }

    #[test]
    fn compute_ignition_counts_limit_ups() {
        let comps = vec![
            bs("600001", 10.0, 1e8, 1.0), // 主板涨停(≥9.85)
            bs("300001", 20.0, 1e8, 1.0), // 创业板涨停(≥19.85)
            bs("600002", 8.6, 1e8, 1.0),  // 接近涨停(主板 8.6 ≥ 10-1.5)
            bs("600003", 3.0, 1e8, 1.0),  // 普通
        ];
        let ig = compute_ignition(&comps);
        assert_eq!(ig.limit_up_count, 2);
        assert_eq!(ig.near_limit_count, 3);
        assert_eq!(ig.sample, 4);
    }
}
