//! 修复 Top10#3+#4: chain_analysis.rs (1839 行) 拆 3 子模块
//!
//! 这个文件: `chain_analysis/fetchers.rs` — 数据获取 helpers
//!
//! 包含 8 个 fetch_* / push2_get 函数, 共 370 行.
//! 拆分后 mod.rs 从 1839 → 1469 行 (-20%)

//! 子模块互见: 在 mod.rs 把 fetchers 声明为 super 模块, 这里用 super::xxx 调入 fetchers.

use futures::stream::{self, StreamExt};
use log::{info, warn};
use serde_json::json;
use std::collections::{HashMap, HashSet};

use crate::agent::tool::Tool;
use crate::agent::tools_sector::FetchSectorTool;
use crate::analyzer::{AgentMode, GeminiAnalyzer};
use crate::database::DatabaseManager;
use crate::market_data::TopStock;

use super::ChainCluster;
// PUSH2_HOSTS 在 mod.rs 用 pub(crate) 暴露
use crate::pipeline::chain_analysis::PUSH2_HOSTS;
// is_generic_board 在 mod.rs 是 pub(super) — 让 fetchers 可见
use super::is_generic_board;

type SearchFuture = futures::future::BoxFuture<'static, Vec<crate::search_service::SearchResult>>;

/// 获取指定代码集的概念标签：优先 7 天内缓存，缺失的并发拉取并落库。
pub(super) async fn fetch_concepts_cached(
    codes: &[String],
) -> Result<HashMap<String, Vec<String>>, String> {
    if codes.is_empty() || codes.iter().any(|code| code.trim().is_empty()) {
        return Err("产业链概念批次代码为空".to_string());
    }
    let db =
        DatabaseManager::try_get().ok_or_else(|| "产业链概念缓存数据库未初始化".to_string())?;
    let mut map = db.get_cached_concepts(7)?;

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
        let fetched: Vec<(String, Result<Vec<String>, String>)> = stream::iter(missing)
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
            let boards = boards?;
            db.save_stock_concepts(&code, &boards)?;
            map.insert(code, boards);
        }
    }
    if codes.iter().any(|code| !map.contains_key(code)) {
        return Err("产业链概念批次未覆盖全部股票代码".to_string());
    }
    Ok(map)
}

/// 调 FetchSectorTool 拉单只股票的完整板块列表。
pub(super) async fn fetch_boards_via_tool(
    tool: &FetchSectorTool,
    code: &str,
) -> Result<Vec<String>, String> {
    let raw = tool
        .call(json!({ "code": code }))
        .await
        .map_err(|error| format!("产业链 {code} 板块拉取失败: {error}"))?;
    parse_tool_boards(&raw, code)
}

/// BR-114: validate a complete sector-tool response before it enters the cache.
fn parse_tool_boards(raw: &str, code: &str) -> Result<Vec<String>, String> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| format!("产业链 {code} 板块 JSON 非法: {error}"))?;
    let rows = value
        .get("all_boards")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("产业链 {code} 缺少 all_boards 数组"))?;
    if rows.is_empty() {
        return Err(format!("产业链 {code} all_boards 为空"));
    }
    let mut boards = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let board = row
            .as_str()
            .filter(|board| !board.trim().is_empty())
            .ok_or_else(|| format!("产业链 {code} all_boards[{index}] 非法"))?;
        if !boards.iter().any(|existing| existing == board) {
            boards.push(board.to_string());
        }
    }
    Ok(boards)
}

/// BR-114: merge one complete board page and reject duplicate identities.
fn merge_board_code_page(
    map: &mut HashMap<String, String>,
    codes: &mut HashSet<String>,
    json: &serde_json::Value,
    page: usize,
) -> Result<usize, String> {
    let diff = json
        .get("data")
        .and_then(|data| data.get("diff"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("产业链板块第 {page} 页缺少 data.diff"))?;
    for (index, item) in diff.iter().enumerate() {
        let code = item
            .get("f12")
            .and_then(serde_json::Value::as_str)
            .filter(|value| value.starts_with("BK") && value.len() > 2)
            .ok_or_else(|| format!("产业链板块第 {page} 页第 {index} 行 code 非法"))?;
        let name = item
            .get("f14")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("产业链板块第 {page} 页第 {index} 行 name 非法"))?;
        if !codes.insert(code.to_string()) {
            return Err(format!("产业链板块代码重复: {code}"));
        }
        if map.insert(name.to_string(), code.to_string()).is_some() {
            return Err(format!("产业链板块名称重复: {name}"));
        }
    }
    Ok(diff.len())
}

/// 拉取东财全部概念板块列表，返回 板块名 -> 板块代码(BKxxxx)。
pub(super) async fn fetch_board_code_map() -> Result<HashMap<String, String>, String> {
    fetch_board_code_map_from_hosts(PUSH2_HOSTS).await
}

async fn fetch_board_code_map_from_hosts(
    hosts: &[&str],
) -> Result<HashMap<String, String>, String> {
    let client_builder = reqwest::Client::builder();
    #[cfg(test)]
    let client_builder = client_builder.no_proxy();
    let client = client_builder
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| format!("产业链板块 HTTP client 初始化失败: {error}"))?;
    fetch_board_code_map_with_client(&client, hosts).await
}

async fn fetch_board_code_map_with_client(
    client: &reqwest::Client,
    hosts: &[&str],
) -> Result<HashMap<String, String>, String> {
    let mut map = HashMap::new();
    let mut codes = HashSet::new();
    let mut terminal_seen = false;
    for page in 1..=20 {
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
        let json = push2_get_from_hosts(client, &params, hosts).await?;
        let page_len = merge_board_code_page(&mut map, &mut codes, &json, page)?;
        if page_len == 0 {
            terminal_seen = true;
            break;
        }
        if page_len < 500 {
            terminal_seen = true;
            break;
        }
    }
    if !terminal_seen {
        return Err("产业链板块分页超过 20 页，批次完整性未知".to_string());
    }
    if map.is_empty() {
        return Err("产业链板块批次为空".to_string());
    }
    Ok(map)
}

async fn push2_get_from_hosts(
    client: &reqwest::Client,
    params: &[(&str, &str)],
    hosts: &[&str],
) -> Result<serde_json::Value, String> {
    let mut errors = Vec::new();
    for host in hosts {
        let url = format!("{}/api/qt/clist/get", host);
        let resp = client
            .get(&url)
            .query(params)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
            )
            .header("Referer", "https://quote.eastmoney.com/")
            .send()
            .await;
        match resp {
            Ok(response) => {
                let status = response.status().as_u16();
                match response.text().await {
                    Ok(body) => match parse_push2_http_payload(host, status, &body) {
                        Ok(json) => return Ok(json),
                        Err(error) => errors.push(error),
                    },
                    Err(error) => errors.push(format!("{host}: response body error: {error}")),
                }
            }
            Err(error) => errors.push(format!("{host}: request error: {error}")),
        }
    }
    Err(format!("产业链 push2 所有主机失败: {}", errors.join(" | ")))
}

fn parse_push2_http_payload(
    host: &str,
    status: u16,
    body: &str,
) -> Result<serde_json::Value, String> {
    if !(200..300).contains(&status) {
        return Err(format!("{host}: HTTP status {status}"));
    }
    let json: serde_json::Value =
        serde_json::from_str(body).map_err(|error| format!("{host}: invalid JSON: {error}"))?;
    if json.get("data").is_none() {
        return Err(format!("{host}: missing data"));
    }
    Ok(json)
}

/// 拉取某概念板块成分股，筛选补涨候选：未涨停、今日涨幅 -3%~+7%、非 ST、非北交所。
/// 按涨幅降序取前 8 只。
pub(super) async fn fetch_laggard_candidates(
    board_code: &str,
    limit_codes: &HashSet<String>,
) -> Result<Vec<TopStock>, String> {
    fetch_laggard_candidates_from_hosts(board_code, limit_codes, PUSH2_HOSTS).await
}

async fn fetch_laggard_candidates_from_hosts(
    board_code: &str,
    limit_codes: &HashSet<String>,
    hosts: &[&str],
) -> Result<Vec<TopStock>, String> {
    if !board_code.starts_with("BK") {
        return Err(format!("产业链板块代码非法: {board_code}"));
    }
    let client_builder = reqwest::Client::builder();
    #[cfg(test)]
    let client_builder = client_builder.no_proxy();
    let client = client_builder
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| format!("产业链成份 HTTP client 初始化失败: {error}"))?;
    fetch_laggard_candidates_with_client(&client, board_code, limit_codes, hosts).await
}

async fn fetch_laggard_candidates_with_client(
    client: &reqwest::Client,
    board_code: &str,
    limit_codes: &HashSet<String>,
    hosts: &[&str],
) -> Result<Vec<TopStock>, String> {
    if !board_code.starts_with("BK") {
        return Err(format!("产业链板块代码非法: {board_code}"));
    }
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
    let json = push2_get_from_hosts(client, &params, hosts).await?;
    parse_laggard_candidates(&json, board_code, limit_codes)
}

/// BR-114: validate the complete constituent batch before filtering and ranking.
fn parse_laggard_candidates(
    json: &serde_json::Value,
    board_code: &str,
    limit_codes: &HashSet<String>,
) -> Result<Vec<TopStock>, String> {
    if !board_code.starts_with("BK") {
        return Err(format!("产业链板块代码非法: {board_code}"));
    }
    let diff = json
        .get("data")
        .and_then(|d| d.get("diff"))
        .and_then(|d| d.as_array())
        .ok_or_else(|| format!("产业链板块 {board_code} 缺少 data.diff"))?;
    if diff.is_empty() {
        return Err(format!("产业链板块 {board_code} 成份股为空"));
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (index, item) in diff.iter().enumerate() {
        let code = item
            .get("f12")
            .and_then(|value| value.as_str())
            .filter(|value| value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit()))
            .ok_or_else(|| format!("产业链板块 {board_code} 第 {index} 行 code 非法"))?;
        let name = item
            .get("f14")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("产业链板块 {board_code} 第 {index} 行 name 非法"))?;
        let pct = item
            .get("f3")
            .and_then(|value| value.as_f64())
            .filter(|value| value.is_finite() && value.abs() <= 20.0)
            .ok_or_else(|| format!("产业链板块 {board_code} 第 {index} 行 pct 非法"))?;
        let price = item
            .get("f2")
            .and_then(|value| value.as_f64())
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| format!("产业链板块 {board_code} 第 {index} 行 price 非法"))?;
        if !seen.insert(code.to_string()) {
            return Err(format!("产业链板块 {board_code} code 重复: {code}"));
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
    out.sort_by(|left, right| {
        right
            .change_pct
            .total_cmp(&left.change_pct)
            .then_with(|| left.code.cmp(&right.code))
    });
    out.truncate(8);
    Ok(out)
}

/// 今日龙虎榜净买入映射 code -> 净买额(万元)，失败返回空。
pub(super) async fn fetch_lhb_map() -> Result<HashMap<String, f64>, String> {
    let fetcher = crate::lhb_analyzer::LhbDataFetcher::new()
        .map_err(|error| format!("产业链龙虎榜抓取器初始化失败: {error}"))?;
    let records = fetcher
        .get_today_lhb()
        .await
        .map_err(|error| format!("产业链龙虎榜获取失败: {error}"))?;
    map_lhb_records(records)
}

fn map_lhb_records(
    records: Vec<crate::lhb_analyzer::LhbRecord>,
) -> Result<HashMap<String, f64>, String> {
    let mut out = HashMap::new();
    for record in records {
        if record.code.trim().is_empty() || !record.net_amount.is_finite() {
            return Err(format!("产业链龙虎榜行非法: code={:?}", record.code));
        }
        if out.insert(record.code.clone(), record.net_amount).is_some() {
            return Err(format!("产业链龙虎榜 code 重复: {}", record.code));
        }
    }
    Ok(out)
}

fn append_after_market_items(
    items: &mut Vec<String>,
    theme: &str,
    results: Vec<crate::search_service::SearchResult>,
) {
    for result in results {
        let date = result.published_date.as_deref().unwrap_or("");
        let snippet: String = result.snippet.chars().take(100).collect();
        let item = format!(
            "- 🔥 **{}** [{}] {}\n  {}",
            result.title, theme, date, snippet
        );
        if !items
            .iter()
            .any(|existing| existing.contains(&result.title))
        {
            items.push(item);
        }
    }
}

fn render_after_market_section(today: &str, time_label: &str, items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    format!(
        "## 🚨 盘后催化追踪（{} {} 最新动态，{} 条）\n\n{}\n",
        today,
        time_label,
        items.len(),
        items.join("\n")
    )
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

    resolve_after_market_catalysts(
        top_themes,
        &today_str,
        time_label,
        std::time::Duration::from_secs(8),
        move |query, limit| Box::pin(async move { svc.search_topic(&query, limit).await }),
    )
    .await
}

async fn resolve_after_market_catalysts<F>(
    top_themes: &[&str],
    today: &str,
    time_label: &str,
    timeout: std::time::Duration,
    mut search: F,
) -> String
where
    F: FnMut(String, usize) -> SearchFuture,
{
    let mut items = Vec::new();
    for theme in top_themes.iter().take(5) {
        if items.len() >= 10 {
            break;
        }
        let query = format!("{today} {theme} 最新 突发 催化");
        let results = tokio::time::timeout(timeout, search(query, 2))
            .await
            .unwrap_or_default();
        append_after_market_items(&mut items, theme, results);
    }
    render_after_market_section(today, time_label, &items)
}

fn build_cluster_query_context(
    cluster: &ChainCluster,
    concepts: &HashMap<String, Vec<String>>,
) -> (Vec<String>, String) {
    let leaders: Vec<&str> = cluster
        .stocks
        .iter()
        .take(2)
        .map(|stock| stock.name.as_str())
        .collect();
    let queries = vec![format!(
        "{} 板块 集体涨停 原因 {}",
        cluster.concept,
        leaders.join(" ")
    )];

    let mut stock_lines = String::new();
    for stock in cluster.stocks.iter().take(10) {
        let tags: Vec<&str> = concepts
            .get(&stock.code)
            .map(|boards| {
                boards
                    .iter()
                    .filter(|board| !is_generic_board(board))
                    .map(|board| board.as_str())
                    .take(6)
                    .collect()
            })
            .unwrap_or_default();
        stock_lines.push_str(&format!("- {}：{}\n", stock.name, tags.join("、")));
    }
    let prompt = format!(
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
    (queries, prompt)
}

fn append_generated_cluster_queries(queries: &mut Vec<String>, text: &str) {
    for line in text.lines() {
        let query = line
            .trim()
            .trim_start_matches(|character: char| {
                character.is_ascii_digit()
                    || character == '.'
                    || character == '-'
                    || character == '、'
                    || character == '*'
            })
            .trim()
            .trim_matches('"');
        let len = query.chars().count();
        let looks_like_sentence = query.contains('。')
            || query.contains('，')
            || query.contains('；')
            || query.contains('？');
        if (4..=40).contains(&len) && !looks_like_sentence && queries.len() < 4 {
            queries.push(query.to_string());
        }
    }
}

fn append_cluster_news_items(
    seen: &mut HashSet<String>,
    items: &mut Vec<String>,
    results: Vec<crate::search_service::SearchResult>,
) {
    for result in results {
        let key: String = result.title.chars().take(20).collect();
        if !seen.insert(key) {
            continue;
        }
        let published = result.published_date.as_deref().unwrap_or("");
        let snippet: String = result.snippet.chars().take(150).collect();
        items.push(format!(
            "- **{}** {}\n  {}",
            result.title, published, snippet
        ));
        if items.len() >= 10 {
            break;
        }
    }
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

    let (queries, q_prompt) = build_cluster_query_context(cluster, concepts);
    let generated_queries = analyzer
        .call_api_mode(
            &q_prompt,
            "你是A股题材挖掘专家，只输出新闻搜索词，每行一条。",
            AgentMode::Quick,
        )
        .await
        .map_err(|error| error.to_string());
    resolve_cluster_news(
        cluster,
        queries,
        generated_queries,
        std::time::Duration::from_secs(15),
        move |query, limit| Box::pin(async move { search.search_topic(&query, limit).await }),
    )
    .await
}

async fn resolve_cluster_news<F>(
    cluster: &ChainCluster,
    mut queries: Vec<String>,
    generated_queries: Result<String, String>,
    timeout: std::time::Duration,
    mut search: F,
) -> String
where
    F: FnMut(String, usize) -> SearchFuture,
{
    match generated_queries {
        Ok(text) => append_generated_cluster_queries(&mut queries, &text),
        Err(error) => warn!(
            "[产业链] 主线「{}」催化搜索词生成失败: {}",
            cluster.concept, error
        ),
    }
    log::debug!("[产业链] 主线「{}」检索词: {:?}", cluster.concept, queries);

    // 执行检索，按标题去重合并
    let mut seen: HashSet<String> = HashSet::new();
    let mut items: Vec<String> = Vec::new();
    for q in &queries {
        let results = match tokio::time::timeout(timeout, search(q.clone(), 4)).await {
            Ok(r) => r,
            Err(_) => {
                warn!("[产业链] 主线「{}」检索词 '{}' 超时", cluster.concept, q);
                continue;
            }
        };
        append_cluster_news_items(&mut seen, &mut items, results);
        if items.len() >= 10 {
            break;
        }
    }
    items.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{
        append_after_market_items, append_cluster_news_items, append_generated_cluster_queries,
        build_cluster_query_context, fetch_board_code_map_from_hosts,
        fetch_board_code_map_with_client, fetch_concepts_cached,
        fetch_laggard_candidates_from_hosts, fetch_laggard_candidates_with_client, map_lhb_records,
        merge_board_code_page, parse_laggard_candidates, parse_push2_http_payload,
        parse_tool_boards, push2_get_from_hosts, render_after_market_section,
        resolve_after_market_catalysts, resolve_cluster_news,
    };
    use serde_json::json;
    use std::collections::{HashMap, HashSet};

    fn search_result(
        title: impl Into<String>,
        snippet: impl Into<String>,
        published_date: Option<&str>,
    ) -> crate::search_service::SearchResult {
        crate::search_service::SearchResult {
            title: title.into(),
            snippet: snippet.into(),
            url: "https://example.invalid/test".to_string(),
            source: "TEST_CODE_SOURCE".to_string(),
            published_date: published_date.map(str::to_string),
            news_type: crate::search_service::NewsType::Industry,
            sentiment: crate::search_service::Sentiment::Neutral,
            importance: 5,
            relevance: 1.0,
            keywords: Vec::new(),
        }
    }

    fn lhb(code: &str, net_amount: f64) -> crate::lhb_analyzer::LhbRecord {
        crate::lhb_analyzer::LhbRecord {
            code: code.to_string(),
            name: "测试龙虎榜".to_string(),
            trade_date: "2026-07-18".to_string(),
            reason: "测试原因".to_string(),
            pct_change: 1.0,
            close_price: 10.0,
            buy_amount: 2.0,
            sell_amount: 1.0,
            net_amount,
            total_amount: 3.0,
            lhb_ratio: 10.0,
            inst_buy_seats: 1,
            inst_sell_seats: 0,
            inst_net_amount: 1.0,
        }
    }

    #[test]
    fn lhb_mapping_rejects_bad_or_duplicate_complete_records() {
        let mapped = map_lhb_records(vec![
            lhb("TEST_CODE_000001", 12.5),
            lhb("TEST_CODE_000002", -3.0),
        ])
        .expect("complete LHB batch");
        assert_eq!(mapped["TEST_CODE_000001"], 12.5);
        assert_eq!(mapped["TEST_CODE_000002"], -3.0);
        assert!(map_lhb_records(vec![lhb("", 1.0)]).is_err());
        assert!(map_lhb_records(vec![lhb("TEST_CODE_000001", f64::NAN)]).is_err());
        assert!(map_lhb_records(vec![
            lhb("TEST_CODE_000001", 1.0),
            lhb("TEST_CODE_000001", 2.0),
        ])
        .is_err());
    }

    #[test]
    fn resolved_catalyst_results_deduplicate_truncate_and_render() {
        let mut items = Vec::new();
        append_after_market_items(
            &mut items,
            "测试主线",
            vec![
                search_result("真实催化A", "甲".repeat(120), Some("2026-07-18")),
                search_result("真实催化A", "重复", None),
                search_result("真实催化B", "乙", None),
            ],
        );
        assert_eq!(items.len(), 2);
        assert!(items[0].contains("测试主线"));
        assert!(!items[0].contains(&"甲".repeat(101)));
        assert!(render_after_market_section("07月18日", "盘后", &[]).is_empty());
        let section = render_after_market_section("07月18日", "盘后", &items);
        assert!(section.contains("2 条"));
        assert!(section.contains("真实催化A"));
    }

    #[tokio::test]
    async fn resolved_after_market_search_enforces_theme_and_item_limits() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let search_calls = std::sync::Arc::clone(&calls);
        let section = resolve_after_market_catalysts(
            &["主题甲", "主题乙", "主题丙", "主题丁", "主题戊", "主题己"],
            "07月18日",
            "盘后",
            std::time::Duration::from_secs(1),
            move |query, limit| {
                search_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Box::pin(async move {
                    assert_eq!(limit, 2);
                    vec![
                        search_result(format!("{query}-A"), "真实摘要A", Some("2026-07-18")),
                        search_result(format!("{query}-B"), "真实摘要B", None),
                    ]
                })
            },
        )
        .await;
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 5);
        assert!(section.contains("10 条"));
        assert!(!section.contains("主题己"));

        let timed_out = resolve_after_market_catalysts(
            &["超时主题"],
            "07月18日",
            "盘中",
            std::time::Duration::ZERO,
            |_query, _limit| Box::pin(futures::future::pending()),
        )
        .await;
        assert!(timed_out.is_empty());
        assert!(resolve_after_market_catalysts(
            &[],
            "07月18日",
            "盘后",
            std::time::Duration::from_secs(1),
            |_query, _limit| Box::pin(async { Vec::new() }),
        )
        .await
        .is_empty());
    }

    #[tokio::test]
    async fn resolved_cluster_search_merges_generated_queries_and_explicit_failures() {
        let cluster = super::super::ChainCluster {
            concept: "TEST_CODE_固态电池".to_string(),
            aliases: Vec::new(),
            stocks: Vec::new(),
            continuation_count: 0,
            streak_days: 0,
            candidates: Vec::new(),
            score: None,
            scenario: None,
        };
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let search_calls = std::sync::Arc::clone(&calls);
        let news = resolve_cluster_news(
            &cluster,
            vec!["默认 主线查询".to_string()],
            Ok("1. 电解质 扩产\n- 原材料 涨价".to_string()),
            std::time::Duration::from_secs(1),
            move |query, limit| {
                search_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Box::pin(async move {
                    assert_eq!(limit, 4);
                    vec![
                        search_result("跨查询重复标题", format!("{query} 摘要"), None),
                        search_result(format!("{query} 独有"), "真实摘要", Some("2026-07-18")),
                    ]
                })
            },
        )
        .await;
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 3);
        assert_eq!(news.matches("跨查询重复标题").count(), 1);
        assert!(news.contains("电解质 扩产 独有"));

        let unavailable = resolve_cluster_news(
            &cluster,
            vec!["超时 查询".to_string()],
            Err("TEST_CODE_模型不可用".to_string()),
            std::time::Duration::ZERO,
            |_query, _limit| Box::pin(futures::future::pending()),
        )
        .await;
        assert!(unavailable.is_empty());
    }

    #[test]
    fn cluster_query_protocol_and_result_dedup_keep_registered_limits() {
        let cluster = super::super::ChainCluster {
            concept: "TEST_CODE_固态电池".to_string(),
            aliases: Vec::new(),
            stocks: vec![
                crate::market_data::TopStock {
                    code: "TEST_CODE_000001".to_string(),
                    name: "测试甲".to_string(),
                    ..Default::default()
                },
                crate::market_data::TopStock {
                    code: "TEST_CODE_000002".to_string(),
                    name: "测试乙".to_string(),
                    ..Default::default()
                },
            ],
            continuation_count: 0,
            streak_days: 0,
            candidates: Vec::new(),
            score: None,
            scenario: None,
        };
        let concepts = HashMap::from([
            (
                "TEST_CODE_000001".to_string(),
                vec!["融资融券".to_string(), "固态电池设备".to_string()],
            ),
            ("TEST_CODE_000002".to_string(), vec!["电解质".to_string()]),
        ]);
        let (mut queries, prompt) = build_cluster_query_context(&cluster, &concepts);
        assert_eq!(queries.len(), 1);
        assert!(queries[0].contains("测试甲 测试乙"));
        assert!(prompt.contains("固态电池设备"));
        assert!(!prompt.contains("融资融券、固态电池设备"));
        append_generated_cluster_queries(
            &mut queries,
            "1. 电解质 扩产\n- 固态电池 政策\n这是完整句子，应该被拒绝。\nx\n* 原材料 涨价",
        );
        assert_eq!(queries.len(), 4);
        assert!(queries.iter().any(|query| query == "电解质 扩产"));
        assert!(!queries.iter().any(|query| query.contains("应该被拒绝")));

        let mut seen = HashSet::new();
        let mut items = Vec::new();
        let mut results: Vec<_> = (0..12)
            .map(|index| search_result(format!("真实产业新闻{index}"), "摘要".repeat(100), None))
            .collect();
        results.insert(
            1,
            search_result("真实产业新闻0", "重复", Some("2026-07-18")),
        );
        append_cluster_news_items(&mut seen, &mut items, results);
        assert_eq!(items.len(), 10);
        assert_eq!(seen.len(), 10);
        assert!(!items[0].contains(&"摘要".repeat(76)));
    }

    #[test]
    fn tool_board_batch_deduplicates_only_complete_nonempty_strings() {
        let boards = parse_tool_boards(
            r#"{"all_boards":["TEST_CODE_机器人","TEST_CODE_算力","TEST_CODE_机器人"]}"#,
            "TEST_CODE_000001",
        )
        .expect("complete tool response");
        assert_eq!(boards, ["TEST_CODE_机器人", "TEST_CODE_算力"]);

        for raw in [
            "not-json",
            r#"{}"#,
            r#"{"all_boards":[]}"#,
            r#"{"all_boards":[""]}"#,
            r#"{"all_boards":[1]}"#,
        ] {
            assert!(parse_tool_boards(raw, "TEST_CODE_000001").is_err(), "{raw}");
        }
    }

    #[test]
    fn board_pages_merge_unique_identity_and_reject_protocol_conflicts() {
        let mut map = HashMap::new();
        let mut codes = HashSet::new();
        let first = json!({"data":{"diff":[
            {"f12":"BK0001","f14":"TEST_CODE_机器人"},
            {"f12":"BK0002","f14":"TEST_CODE_算力"}
        ]}});
        assert_eq!(
            merge_board_code_page(&mut map, &mut codes, &first, 1),
            Ok(2)
        );
        assert_eq!(
            map.get("TEST_CODE_机器人").map(String::as_str),
            Some("BK0001")
        );

        for bad in [
            json!({}),
            json!({"data":{"diff":[{"f12":"0001","f14":"坏代码"}]}}),
            json!({"data":{"diff":[{"f12":"BK0003","f14":""}]}}),
            json!({"data":{"diff":[{"f12":"BK0001","f14":"重复代码"}]}}),
            json!({"data":{"diff":[{"f12":"BK0004","f14":"TEST_CODE_机器人"}]}}),
        ] {
            let mut local_map = map.clone();
            let mut local_codes = codes.clone();
            assert!(merge_board_code_page(&mut local_map, &mut local_codes, &bad, 2).is_err());
        }

        assert_eq!(
            merge_board_code_page(
                &mut HashMap::new(),
                &mut HashSet::new(),
                &json!({"data":{"diff":[]}}),
                3,
            ),
            Ok(0)
        );
    }

    fn constituent(code: &str, name: &str, pct: f64, price: f64) -> serde_json::Value {
        json!({"f12":code,"f14":name,"f3":pct,"f2":price})
    }

    #[test]
    fn laggard_batch_filters_then_stably_ranks_top_eight() {
        // Native six-digit shapes are protocol fixtures only; no order is placed.
        let mut rows = vec![
            constituent("100001", "候选A", 5.0, 10.0),
            constituent("100002", "候选B", 5.0, 11.0),
            constituent("100003", "候选C", 4.0, 12.0),
            constituent("100004", "候选D", 3.0, 13.0),
            constituent("100005", "候选E", 2.0, 14.0),
            constituent("100006", "候选F", 1.0, 15.0),
            constituent("100007", "候选G", 0.0, 16.0),
            constituent("100008", "候选H", -1.0, 17.0),
            constituent("100009", "候选I", -2.0, 18.0),
            constituent("100010", "已涨停", 6.0, 19.0),
            constituent("100011", "ST过滤", 6.0, 20.0),
            constituent("800001", "北交过滤", 6.0, 21.0),
            constituent("400001", "北交过滤2", 6.0, 22.0),
            constituent("900001", "北交过滤3", 6.0, 23.0),
            constituent("100012", "涨幅过高", 7.1, 24.0),
            constituent("100013", "跌幅过低", -3.1, 25.0),
        ];
        rows.reverse();
        let limit_codes = HashSet::from(["100010".to_string()]);
        let result =
            parse_laggard_candidates(&json!({"data":{"diff":rows}}), "BK0001", &limit_codes)
                .expect("complete constituent batch");
        assert_eq!(result.len(), 8);
        assert_eq!(result[0].code, "100001");
        assert_eq!(result[1].code, "100002");
        assert_eq!(result[7].code, "100008");
        assert!(result.iter().all(|stock| stock.price > 0.0));
    }

    #[test]
    fn laggard_batch_rejects_any_bad_or_duplicate_row() {
        let valid = constituent("100001", "候选", 1.0, 10.0);
        for (board, rows) in [
            ("INVALID", vec![valid.clone()]),
            ("BK0001", Vec::new()),
            (
                "BK0001",
                vec![json!({"f12":"BAD","f14":"候选","f3":1.0,"f2":10.0})],
            ),
            (
                "BK0001",
                vec![json!({"f12":"100001","f14":"","f3":1.0,"f2":10.0})],
            ),
            (
                "BK0001",
                vec![json!({"f12":"100001","f14":"候选","f3":21.0,"f2":10.0})],
            ),
            (
                "BK0001",
                vec![json!({"f12":"100001","f14":"候选","f3":1.0,"f2":0.0})],
            ),
            ("BK0001", vec![valid.clone(), valid.clone()]),
        ] {
            assert!(parse_laggard_candidates(
                &json!({"data":{"diff":rows}}),
                board,
                &HashSet::new(),
            )
            .is_err());
        }
        assert!(parse_laggard_candidates(&json!({}), "BK0001", &HashSet::new()).is_err());
    }

    #[test]
    fn push2_http_payload_requires_success_json_and_complete_data() {
        assert!(
            parse_push2_http_payload("TEST_CODE_host", 503, r#"{"data":{}}"#)
                .expect_err("non-success status")
                .contains("HTTP status 503")
        );
        assert!(parse_push2_http_payload("TEST_CODE_host", 200, "not-json")
            .expect_err("invalid JSON")
            .contains("invalid JSON"));
        assert!(
            parse_push2_http_payload("TEST_CODE_host", 200, r#"{"rc":0}"#)
                .expect_err("missing complete data")
                .contains("missing data")
        );
        let parsed = parse_push2_http_payload("TEST_CODE_host", 200, r#"{"data":{"diff":[]}}"#)
            .expect("complete response");
        assert_eq!(parsed["data"]["diff"], json!([]));
    }

    #[tokio::test]
    async fn transport_entrypoints_reject_invalid_or_unavailable_sources() {
        assert!(
            fetch_laggard_candidates_from_hosts("INVALID", &HashSet::new(), &[])
                .await
                .is_err()
        );
        assert!(fetch_board_code_map_from_hosts(&[]).await.is_err());
        assert!(
            fetch_laggard_candidates_from_hosts("BK0001", &HashSet::new(), &[])
                .await
                .is_err()
        );

        let hosts = ["http://127.0.0.1:9"];
        let client = crate::data_provider::unreachable_http_client();
        let push_error = push2_get_from_hosts(&client, &[], &hosts)
            .await
            .expect_err("unreachable push2 transport must fail explicitly");
        assert!(push_error.contains("所有主机失败"));
        assert!(push_error.contains("request error"));
        assert!(fetch_board_code_map_from_hosts(&hosts).await.is_err());
        assert!(
            fetch_laggard_candidates_from_hosts("BK0001", &HashSet::new(), &hosts)
                .await
                .is_err()
        );
        assert!(fetch_concepts_cached(&[]).await.is_err());
        assert!(fetch_concepts_cached(&[String::new()]).await.is_err());
    }

    #[tokio::test]
    async fn loopback_transport_executes_board_pagination_and_constituent_batch() {
        let server = crate::data_provider::TestHttpServer::new(vec![
            crate::data_provider::TestHttpResponse::json(
                r#"{"data":{"diff":[{"f12":"BK0001","f14":"TEST_CODE_板块甲"},{"f12":"BK0002","f14":"TEST_CODE_板块乙"}]}}"#,
            ),
            crate::data_provider::TestHttpResponse::json(
                r#"{"data":{"diff":[{"f12":"100001","f14":"协议候选甲","f3":6.0,"f2":10.0},{"f12":"100002","f14":"协议候选乙","f3":2.0,"f2":8.0},{"f12":"100003","f14":"ST测试","f3":1.0,"f2":5.0}]}}"#,
            ),
        ]);
        let hosts = [server.base_url()];
        let client = crate::data_provider::loopback_http_client();
        let board_map = fetch_board_code_map_with_client(&client, &hosts)
            .await
            .expect("complete board page");
        assert_eq!(
            board_map.get("TEST_CODE_板块甲").map(String::as_str),
            Some("BK0001")
        );

        let candidates = fetch_laggard_candidates_with_client(
            &client,
            "BK0001",
            &HashSet::from(["100002".to_string()]),
            &hosts,
        )
        .await
        .expect("complete constituent page");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].code, "100001");
        assert_eq!(candidates[0].change_pct, 6.0);

        let requests = server.finish();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("/api/qt/clist/get?"));
        assert!(requests[0].contains("fs=m%3A90%2Bt%3A3"));
        assert!(requests[1].contains("fs=b%3ABK0001"));

        let server = crate::data_provider::TestHttpServer::new(vec![
            crate::data_provider::TestHttpResponse::json(
                r#"{"data":{"diff":[{"f12":"BK0100","f14":"TEST_CODE_构造器板块"}]}}"#,
            ),
        ]);
        let hosts = [server.base_url()];
        let board_map = fetch_board_code_map_from_hosts(&hosts)
            .await
            .expect("provider-owned HTTP client board page");
        assert_eq!(board_map["TEST_CODE_构造器板块"], "BK0100");
        assert_eq!(server.finish().len(), 1);

        let server = crate::data_provider::TestHttpServer::new(vec![
            crate::data_provider::TestHttpResponse::json(
                r#"{"data":{"diff":[{"f12":"100100","f14":"构造器候选","f3":1.0,"f2":10.0}]}}"#,
            ),
        ]);
        let hosts = [server.base_url()];
        let candidates = fetch_laggard_candidates_from_hosts("BK0100", &HashSet::new(), &hosts)
            .await
            .expect("provider-owned HTTP client constituent batch");
        assert_eq!(candidates[0].code, "100100");
        assert_eq!(server.finish().len(), 1);
    }

    #[tokio::test]
    async fn cached_concepts_and_parsed_protocols_cover_success_boundaries() {
        crate::database::DatabaseManager::init(None).expect("test database initialization");
        let db = crate::database::DatabaseManager::try_get().expect("test database");
        let cached_code = "TEST_CODE_CHAIN_CACHE_000001";
        let cached = vec!["TEST_CODE_固态电池".to_string()];
        db.save_stock_concepts(cached_code, &cached)
            .expect("cache isolated concepts");
        let concepts = fetch_concepts_cached(&[cached_code.to_string()])
            .await
            .expect("complete cache hit must avoid external transport");
        assert_eq!(concepts.get(cached_code), Some(&cached));

        let board_page = json!({"data":{"diff":[
            {"f12":"BK0001","f14":"TEST_CODE_板块0001"},
            {"f12":"BK0002","f14":"TEST_CODE_板块0002"}
        ]}});
        let mut board_map = HashMap::new();
        let mut board_codes = HashSet::new();
        assert_eq!(
            merge_board_code_page(&mut board_map, &mut board_codes, &board_page, 1),
            Ok(2)
        );
        assert_eq!(board_map.len(), 2);

        let constituent_body = json!({"data":{"diff":[
            {"f12":"100001","f14":"测试候选","f3":3.0,"f2":10.0}
        ]}});
        let candidates = parse_laggard_candidates(&constituent_body, "BK0500", &HashSet::new())
            .expect("complete constituent protocol");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].code, "100001");
    }

    #[tokio::test]
    async fn empty_resolved_search_batches_are_stable_regardless_of_environment_keys() {
        let cluster = super::super::ChainCluster {
            concept: "TEST_CODE_主题".to_string(),
            aliases: Vec::new(),
            stocks: Vec::new(),
            continuation_count: 0,
            streak_days: 0,
            candidates: Vec::new(),
            score: None,
            scenario: None,
        };
        assert!(resolve_after_market_catalysts(
            &["TEST_CODE_主题"],
            "07月19日",
            "盘后",
            std::time::Duration::from_secs(1),
            |_query, _limit| Box::pin(async { Vec::new() }),
        )
        .await
        .is_empty());
        assert!(resolve_cluster_news(
            &cluster,
            vec!["TEST_CODE_查询".into()],
            Ok(String::new()),
            std::time::Duration::from_secs(1),
            |_query, _limit| Box::pin(async { Vec::new() }),
        )
        .await
        .is_empty());
    }
}
