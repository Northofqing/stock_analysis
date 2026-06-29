//! 修复 Top10#3+#4: chain_analysis.rs (1839 行) 拆 3 子模块
//!
//! 这个文件: `chain_analysis/fetchers.rs` — 数据获取 helpers
//!
//! 包含 8 个 fetch_* / push2_get 函数, 共 370 行.
//! 拆分后 mod.rs 从 1839 → 1469 行 (-20%)

//! 子模块互见: 在 mod.rs 把 fetchers 声明为 super 模块, 这里用 super::xxx 调入 fetchers.

use anyhow::Result;
use futures::stream::{self, StreamExt};
use log::{info, warn};
use serde_json::json;
use std::collections::{HashMap, HashSet};

use crate::agent::tool::Tool;
use crate::agent::tools_sector::FetchSectorTool;
use crate::analyzer::{AgentMode, GeminiAnalyzer};
use crate::database::DatabaseManager;
use crate::lhb_analyzer::LhbDataFetcher;
use crate::market_data::TopStock;
use crate::search_service::{SearchResult, SearchService};

use super::ChainCluster;
// PUSH2_HOSTS 在 mod.rs 用 pub(crate) 暴露
use crate::pipeline::chain_analysis::PUSH2_HOSTS;
// is_generic_board 在 mod.rs 是 pub(super) — 让 fetchers 可见
use super::is_generic_board;

/// 获取指定代码集的概念标签：优先 7 天内缓存，缺失的并发拉取并落库。
pub(super) async fn fetch_concepts_cached(codes: &[String]) -> HashMap<String, Vec<String>> {
    let db = DatabaseManager::get();
    let mut map = db.get_cached_concepts(7);

    let missing: Vec<String> = codes
        .iter()
        .filter(|c| !map.contains_key(*c))
        .cloned()
        .collect();

    if !missing.is_empty() {
        info!(
            "[产业链] 概念缓存命中 {}/{}，在线拉取 {} 只...",
            codes.len() - missing.len(),
            codes.len(),
            missing.len()
        );
        let tool = FetchSectorTool::new();
        let fetched: Vec<(String, Vec<String>)> = stream::iter(missing)
            .map(|code| {
                let tool = &tool;
                async move {
                    let boards = fetch_boards_via_tool(tool, &code).await;
                    (code, boards)
                }
            })
            .buffer_unordered(6)
            .collect()
            .await;

        for (code, boards) in fetched {
            if !boards.is_empty() {
                db.save_stock_concepts(&code, &boards);
            }
            map.insert(code, boards);
        }
    }
    map
}

/// 调 FetchSectorTool 拉单只股票的板块列表；失败返回空列表。
pub(super) async fn fetch_boards_via_tool(tool: &FetchSectorTool, code: &str) -> Vec<String> {
    match tool.call(json!({ "code": code })).await {
        Ok(raw) => serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|v| {
                v.get("all_boards").and_then(|b| {
                    b.as_array().map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                })
            })
            .unwrap_or_default(),
        Err(e) => {
            warn!("[产业链] {} 板块拉取失败: {}", code, e);
            Vec::new()
        }
    }
}

/// 拉取东财全部概念板块列表，返回 板块名 -> 板块代码(BKxxxx)。
pub(super) async fn fetch_board_code_map() -> HashMap<String, String> {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };
    let mut map = HashMap::new();
    'pages: for page in 1..=2 {
        let pn = page.to_string();
        let params = [
            ("pn", pn.as_str()),
            ("pz", "500"),
            ("po", "1"),
            ("np", "1"),
            ("ut", "bd1d9ddb04089700cf9c27f6f7426281"),
            ("fltt", "2"),
            ("invt", "2"),
            ("fid", "f3"),
            ("fs", "m:90+t:3"),
            ("fields", "f12,f14"),
        ];
        let json = match push2_get(&client, &params).await {
            Some(j) => j,
            None => {
                warn!("[产业链] 概念板块列表获取失败（所有主机）");
                break 'pages;
            }
        };
        let diff = match json
            .get("data")
            .and_then(|d| d.get("diff"))
            .and_then(|d| d.as_array())
        {
            Some(arr) if !arr.is_empty() => arr,
            _ => break,
        };
        for item in diff {
            let code = item.get("f12").and_then(|v| v.as_str()).unwrap_or("");
            let name = item.get("f14").and_then(|v| v.as_str()).unwrap_or("");
            if !code.is_empty() && !name.is_empty() {
                map.insert(name.to_string(), code.to_string());
            }
        }
        if diff.len() < 500 {
            break;
        }
    }
    map
}

/// 带多主机回退的 push2 clist 请求。
pub(super) async fn push2_get(
    client: &reqwest::Client,
    params: &[(&str, &str)],
) -> Option<serde_json::Value> {
    for host in PUSH2_HOSTS {
        let url = format!("{}/api/qt/clist/get", host);
        let resp = client
            .get(&url)
            .query(params)
            .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .header("Referer", "https://quote.eastmoney.com/")
            .send()
            .await;
        match resp {
            Ok(r) => match r.json::<serde_json::Value>().await {
                Ok(j) if j.get("data").is_some() => return Some(j),
                _ => continue,
            },
            Err(_) => continue,
        }
    }
    None
}

/// 拉取某概念板块成分股，筛选补涨候选：未涨停、今日涨幅 -3%~+7%、非 ST、非北交所。
/// 按涨幅降序取前 8 只。
pub(super) async fn fetch_laggard_candidates(
    board_code: &str,
    limit_codes: &HashSet<String>,
) -> Vec<TopStock> {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let fs = format!("b:{}", board_code);
    let params = [
        ("pn", "1"),
        ("pz", "300"),
        ("po", "1"),
        ("np", "1"),
        ("ut", "bd1d9ddb04089700cf9c27f6f7426281"),
        ("fltt", "2"),
        ("invt", "2"),
        ("fid", "f3"),
        ("fs", fs.as_str()),
        ("fields", "f2,f3,f12,f14"),
    ];
    let json = match push2_get(&client, &params).await {
        Some(j) => j,
        None => {
            warn!("[产业链] 板块 {} 成分股获取失败（所有主机）", board_code);
            return Vec::new();
        }
    };
    let diff = match json
        .get("data")
        .and_then(|d| d.get("diff"))
        .and_then(|d| d.as_array())
    {
        Some(arr) => arr,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for item in diff {
        let code = item.get("f12").and_then(|v| v.as_str()).unwrap_or("");
        let name = item.get("f14").and_then(|v| v.as_str()).unwrap_or("");
        let pct = item.get("f3").and_then(|v| v.as_f64()).unwrap_or(f64::NAN);
        let price = item.get("f2").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if code.is_empty() || pct.is_nan() {
            continue;
        }
        if limit_codes.contains(code) {
            continue;
        }
        if name.contains("ST") || name.contains("st") {
            continue;
        }
        if code.starts_with('8') || code.starts_with('4') || code.starts_with('9') {
            continue;
        }
        // 涨幅适中：没大跌（链上情绪未崩）也没接近涨停（还有空间）
        if !(-3.0..=7.0).contains(&pct) {
            continue;
        }
        out.push(TopStock {
            code: code.to_string(),
            name: name.to_string(),
            change_pct: pct,
            price,
            ..Default::default()
        });
    }
    out.sort_by(|a, b| b.change_pct.partial_cmp(&a.change_pct).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(8);
    out
}

/// 今日龙虎榜净买入映射 code -> 净买额(万元)，失败返回空。
pub(super) async fn fetch_lhb_map() -> HashMap<String, f64> {
    match crate::lhb_analyzer::LhbDataFetcher::new() {
        Ok(fetcher) => match fetcher.get_today_lhb().await {
            Ok(records) => records
                .into_iter()
                .map(|r| (r.code, r.net_amount))
                .collect(),
            Err(e) => {
                warn!("[产业链] 龙虎榜获取失败: {}", e);
                HashMap::new()
            }
        },
        Err(_) => HashMap::new(),
    }
}

/// 拉取盘后催化快讯，专门用于更新报告时效性。
/// 通过搜索引擎搜最新主题相关新闻，返回格式化的 Markdown 片段。
pub(super) async fn fetch_after_market_catalysts(top_themes: &[&str]) -> String {
    use crate::search_service::get_search_service;
    let svc = get_search_service();
    if !svc.is_available() {
        return String::new();
    }

    let now = chrono::Local::now();
    let today_str = now.format("%m月%d日").to_string();
    let hour = now.format("%H").to_string().parse::<u32>().unwrap_or(0);
    let time_label = if hour >= 15 { "盘后" } else { "盘中" };

    let mut items: Vec<String> = Vec::new();

    // 对TOP主线逐个搜最新催化
    for theme in top_themes.iter().take(5) {
        if items.len() >= 10 { break; }
        let q = format!("{} {} 最新 突发 催化", today_str, theme);
        let results = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            svc.search_topic(&q, 2),
        )
        .await
        .unwrap_or_default();
        for r in results {
            let date = r.published_date.as_deref().unwrap_or("");
            let snippet: String = r.snippet.chars().take(100).collect();
            let item = format!("- 🔥 **{}** [{}] {}\n  {}", r.title, theme, date, snippet);
            if !items.iter().any(|i| i.contains(&r.title)) {
                items.push(item);
            }
        }
    }

    if items.is_empty() {
        return String::new();
    }

    format!(
        "## 🚨 盘后催化追踪（{} {} 最新动态，{} 条）\n\n{}\n",
        today_str,
        time_label,
        items.len(),
        items.join("\n")
    )
}

/// 定向检索某主线簇的产业催化新闻（主线级，区别于通用宏观头条）。
///
/// 两段式：先让 LLM 根据簇内股票推测催化事件方向、生成具体搜索词
/// （解决"世界杯转播/上游停产/替代材料"这类不含概念名的催化搜不到的问题），
/// 再连同默认检索词一起执行、合并去重。
pub(super) async fn fetch_cluster_news(
    analyzer: &GeminiAnalyzer,
    cluster: &ChainCluster,
    concepts: &HashMap<String, Vec<String>>,
) -> String {
    let search = crate::search_service::get_search_service();
    if !search.is_available() {
        return String::new();
    }

    // 默认检索词：板块集体涨停原因
    let leaders: Vec<&str> = cluster
        .stocks
        .iter()
        .take(2)
        .map(|s| s.name.as_str())
        .collect();
    let mut queries = vec![format!(
        "{} 板块 集体涨停 原因 {}",
        cluster.concept,
        leaders.join(" ")
    )];

    // LLM 推测催化方向 → 生成事件级搜索词
    let mut stock_lines = String::new();
    for s in cluster.stocks.iter().take(10) {
        let tags: Vec<&str> = concepts
            .get(&s.code)
            .map(|bs| {
                bs.iter()
                    .filter(|b| !is_generic_board(b))
                    .map(|b| b.as_str())
                    .take(6)
                    .collect()
            })
            .unwrap_or_default();
        stock_lines.push_str(&format!("- {}：{}\n", s.name, tags.join("、")));
    }
    let q_prompt = format!(
        r#"今日 A 股「{}」概念 {} 只股票集体涨停（股票及其概念标签）：
{}
请推测最可能驱动这次集体涨停的催化事件方向，输出 2-3 条具体的中文新闻搜索词，每行一条，不要编号、不要解释。
要求：
- 搜索词必须指向具体事件/商品价格/供给变化/政策/赛事（例："钨 出口管制 价格上涨"、"世界杯 转播权 广告 概念股"、"六氟化钨 停产"）
- 禁止使用"板块 涨停 原因"这类泛词
- 从股票组合的共性倒推：这些公司共同的上游、下游或终端场景最近可能发生了什么"#,
        cluster.concept,
        cluster.stocks.len(),
        stock_lines
    );
    match analyzer
        .call_api_mode(
            &q_prompt,
            "你是A股题材挖掘专家，只输出新闻搜索词，每行一条。",
            AgentMode::Quick,
        )
        .await
    {
        Ok(text) => {
            for line in text.lines() {
                let q = line
                    .trim()
                    .trim_start_matches(|c: char| {
                        c.is_ascii_digit() || c == '.' || c == '-' || c == '、' || c == '*'
                    })
                    .trim()
                    .trim_matches('"');
                // 过滤思考过程泄漏：合法搜索词应当短小、不含句子标点
                let len = q.chars().count();
                let looks_like_sentence =
                    q.contains('。') || q.contains('，') || q.contains('；') || q.contains('？');
                if (4..=40).contains(&len) && !looks_like_sentence && queries.len() < 4 {
                    queries.push(q.to_string());
                }
            }
        }
        Err(e) => warn!("[产业链] 主线「{}」催化搜索词生成失败: {}", cluster.concept, e),
    }
    log::debug!("[产业链] 主线「{}」检索词: {:?}", cluster.concept, queries);

    // 执行检索，按标题去重合并
    let mut seen: HashSet<String> = HashSet::new();
    let mut items: Vec<String> = Vec::new();
    for q in &queries {
        let results = match tokio::time::timeout(
            std::time::Duration::from_secs(15),
            search.search_topic(q, 4),
        )
        .await
        {
            Ok(r) => r,
            Err(_) => {
                warn!("[产业链] 主线「{}」检索词 '{}' 超时", cluster.concept, q);
                continue;
            }
        };
        for r in results {
            let key: String = r.title.chars().take(20).collect();
            if !seen.insert(key) {
                continue;
            }
            let t = r.published_date.as_deref().unwrap_or("");
            let snippet: String = r.snippet.chars().take(150).collect();
            items.push(format!("- **{}** {}\n  {}", r.title, t, snippet));
            if items.len() >= 10 {
                break;
            }
        }
        if items.len() >= 10 {
            break;
        }
    }
    items.join("\n")
}

