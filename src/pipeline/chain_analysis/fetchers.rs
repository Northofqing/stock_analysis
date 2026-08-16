//! 修复 Top10#3+#4: chain_analysis.rs (1839 行) 拆 3 子模块
//!
//! 这个文件: `chain_analysis/fetchers.rs` — 数据获取 helpers
//!
//! 包含产业链附加证据获取 helpers。
//! 拆分后 mod.rs 从 1839 → 1469 行 (-20%)

//! 子模块互见: 在 mod.rs 把 fetchers 声明为 super 模块, 这里用 super::xxx 调入 fetchers.

use futures::stream::{self, StreamExt};
use log::{info, warn};
use serde_json::json;
use std::collections::{HashMap, HashSet};

use crate::agent::tool::Tool;
use crate::agent::tools_sector::FetchSectorTool;
use crate::analyzer::{AgentMode, GeminiAnalyzer};
use crate::data_gateway::{
    BatchEvidence, BoardDataGateway, BoardKind, DragonTigerGateway, DragonTigerStockReview,
    GatewayBatch,
};
use crate::database::DatabaseManager;
use crate::market_data::TopStock;

use super::ChainCluster;
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

pub(super) struct BoardCodeMapBatch {
    pub(super) codes: HashMap<String, String>,
    pub(super) evidence: Vec<BatchEvidence>,
}

/// 通过统一 Magic TDX Gateway 拉取行业和概念目录，并保留每个完整批次证据。
pub(super) async fn fetch_board_code_map() -> Result<BoardCodeMapBatch, String> {
    let mut map = HashMap::new();
    let mut evidence = Vec::new();
    for kind in [BoardKind::Industry, BoardKind::Concept] {
        let batch = BoardDataGateway::new()
            .directory(kind, 10_000)
            .await
            .map_err(|error| format!("产业链板块目录不可用 ({kind:?}): {error}"))?;
        let records = match batch {
            GatewayBatch::Available {
                records,
                evidence: batch_evidence,
            } => {
                evidence.push(batch_evidence);
                records
            }
            GatewayBatch::VerifiedEmpty(evidence) => {
                return Err(format!(
                    "产业链板块目录已验证为空 ({kind:?}): provider={:?} source={} \
                     observed_at={} batch_id={}",
                    evidence.provider, evidence.source, evidence.observed_at, evidence.batch_id
                ));
            }
        };
        for record in records {
            match map.insert(record.name.clone(), record.code.clone()) {
                Some(previous) if previous != record.code => {
                    return Err(format!(
                        "产业链板块名称跨类别冲突: {} => {previous}/{}",
                        record.name, record.code
                    ));
                }
                _ => {}
            }
        }
    }
    if map.is_empty() {
        Err("Magic TDX 板块目录没有可用记录".to_string())
    } else {
        Ok(BoardCodeMapBatch {
            codes: map,
            evidence,
        })
    }
}

/// 当前发布的 Magic TDX 成分合同没有同批次价格、涨幅和证券名称。
/// 补涨筛选必须等待上游发布完整合同，不能再拼接旧行情源。
pub(super) async fn fetch_laggard_candidates(
    board_code: &str,
    _limit_codes: &HashSet<String>,
) -> Result<GatewayBatch<TopStock>, String> {
    Err(format!(
        "产业链补涨候选 unsupported: board={board_code}; \
         Magic TDX 当前只提供成分身份，不提供同批次名称/价格/涨幅，禁止跨源拼接"
    ))
}

/// 今日龙虎榜净买入映射 code -> 净买额(万元)。
/// 2026-08-06 实证: disclosure_limit 原传 5_000, R-04 gateway 上限 100
/// (invalid request: market dragon-tiger limit must be at most 100) —
/// 断点 A 接线后首次暴露, 改为上限内值。
pub(super) async fn fetch_lhb_map() -> Result<HashMap<String, f64>, String> {
    let batch = match DragonTigerGateway::new()
        .market_review(chrono::Local::now().date_naive(), 100, 5_000)
        .await
    {
        Ok(batch) => batch,
        // 2026-08-07: 龙虎榜是 LLM 分析的增强背景 (净买入), 缺失不阻断核心
        // 聚类/落库/报告 — Gateway Err 降级为 warn + 空背景 (与 VerifiedEmpty
        // 同语义, 出声不静默)。盘前时段 Eastmoney 接口常返回 no usable
        // records (09:07 实证), 原 map_err 硬失败导致整条链分析推送失败。
        Err(error) => {
            log::warn!(
                "[产业链][BR-164] 龙虎榜 Gateway 不可用, 降级为空背景 (LLM 无净买入): {error}"
            );
            return Ok(HashMap::new());
        }
    };
    match batch {
        GatewayBatch::Available { records, .. } => map_lhb_reviews(records),
        GatewayBatch::VerifiedEmpty(evidence) => {
            log::info!(
                "[产业链][BR-164] 龙虎榜已验证为空 provider={:?} source={} \
                 observed_at={} batch_id={}",
                evidence.provider,
                evidence.source,
                evidence.observed_at,
                evidence.batch_id
            );
            Ok(HashMap::new())
        }
    }
}

fn map_lhb_reviews(records: Vec<DragonTigerStockReview>) -> Result<HashMap<String, f64>, String> {
    let mut out = HashMap::new();
    for record in records {
        if record.code.trim().is_empty() || !record.ranking_net_amount_yuan.is_finite() {
            return Err(format!("产业链龙虎榜行非法: code={:?}", record.code));
        }
        let net_amount_wan = record.ranking_net_amount_yuan / 10_000.0;
        if out.insert(record.code.clone(), net_amount_wan).is_some() {
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
        build_cluster_query_context, fetch_concepts_cached, fetch_laggard_candidates,
        map_lhb_reviews, parse_tool_boards, render_after_market_section,
        resolve_after_market_catalysts, resolve_cluster_news,
    };
    use crate::data_gateway::DragonTigerStockReview;
    use crate::magic_compat::Exchange;
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
            evidence: crate::search_service::SearchEvidence::Unverified,
        }
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
    fn dragon_tiger_mapping_preserves_yuan_units_and_rejects_ambiguous_rows() {
        let mapped = map_lhb_reviews(vec![
            DragonTigerStockReview {
                exchange: Exchange::Shanghai,
                code: "TEST_CODE_600001".to_string(),
                ranking_net_amount_yuan: 123_450_000.0,
                disclosures: Vec::new(),
            },
            DragonTigerStockReview {
                exchange: Exchange::Shenzhen,
                code: "TEST_CODE_000002".to_string(),
                ranking_net_amount_yuan: -50_000.0,
                disclosures: Vec::new(),
            },
        ])
        .expect("complete gateway records");
        assert_eq!(mapped.get("TEST_CODE_600001"), Some(&12_345.0));
        assert_eq!(mapped.get("TEST_CODE_000002"), Some(&-5.0));

        for record in [
            DragonTigerStockReview {
                exchange: Exchange::Shanghai,
                code: String::new(),
                ranking_net_amount_yuan: 1.0,
                disclosures: Vec::new(),
            },
            DragonTigerStockReview {
                exchange: Exchange::Shanghai,
                code: "TEST_CODE_NAN".to_string(),
                ranking_net_amount_yuan: f64::NAN,
                disclosures: Vec::new(),
            },
            DragonTigerStockReview {
                exchange: Exchange::Shanghai,
                code: "TEST_CODE_INFINITY".to_string(),
                ranking_net_amount_yuan: f64::INFINITY,
                disclosures: Vec::new(),
            },
        ] {
            assert!(map_lhb_reviews(vec![record]).is_err());
        }

        let duplicate = DragonTigerStockReview {
            exchange: Exchange::Shanghai,
            code: "TEST_CODE_DUPLICATE".to_string(),
            ranking_net_amount_yuan: 10_000.0,
            disclosures: Vec::new(),
        };
        assert!(map_lhb_reviews(vec![duplicate.clone(), duplicate]).is_err());
    }

    #[tokio::test]
    async fn cached_concepts_and_parsed_protocols_cover_success_boundaries() {
        assert!(fetch_concepts_cached(&[]).await.is_err());
        assert!(fetch_concepts_cached(&[" ".to_string()]).await.is_err());
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
        assert!(
            fetch_laggard_candidates("tdx:concept:TEST_CODE_板块", &HashSet::new())
                .await
                .expect_err("released contract has no same-batch prices")
                .contains("unsupported")
        );
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
