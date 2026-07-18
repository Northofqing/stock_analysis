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
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| format!("产业链板块 HTTP client 初始化失败: {error}"))?;
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
        let json = push2_get_from_hosts(&client, &params, hosts).await?;
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
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| format!("产业链成份 HTTP client 初始化失败: {error}"))?;
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
    let json = push2_get_from_hosts(&client, &params, hosts).await?;
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
        if items.len() >= 10 {
            break;
        }
        let q = format!("{} {} 最新 突发 催化", today_str, theme);
        let results =
            tokio::time::timeout(std::time::Duration::from_secs(8), svc.search_topic(&q, 2))
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
        Err(e) => warn!(
            "[产业链] 主线「{}」催化搜索词生成失败: {}",
            cluster.concept, e
        ),
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

#[cfg(test)]
mod tests {
    use super::{
        fetch_board_code_map_from_hosts, fetch_laggard_candidates_from_hosts,
        merge_board_code_page, parse_laggard_candidates, parse_push2_http_payload,
        parse_tool_boards,
    };
    use serde_json::json;
    use std::collections::{HashMap, HashSet};

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
    }
}
