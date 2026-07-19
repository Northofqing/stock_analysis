//! 产业链联动分析：涨停池 → 概念共现聚类 → 产业链定位（LLM）→ 报告。
//!
//! 解决的问题：单股分析只看技术/资金/消息面，看不到"整条产业链上下游联动涨停"。
//! 本模块把当日涨停股按概念聚类识别主线，再用 BOM 拆解法让 LLM 做链上定位与预期推演。
//!
//! 定位：**信息整理 + 风险标注工具，不产生买入信号**。
//! - 概念标签来自东财 F10，存在"蹭概念"污染 → LLM 输出强制带证据等级（A/B/C）。
//! - 主线生命周期通过簇内"昨日涨停/连板"标签数估算，警示高位接力风险。

use anyhow::Result;
use log::{info, warn};
use std::collections::{HashMap, HashSet};

// 修复 Top10#3+#4 (2026-06-29 audit): chain_analysis.rs (1839 行) 拆子模块
mod fetchers;
// 兄弟模块 fetchers.rs 内 fetch_* 用 pub(super) 暴露, 这里 use 让 mod.rs 直接调用
use fetchers::{
    fetch_after_market_catalysts, fetch_board_code_map, fetch_cluster_news, fetch_concepts_cached,
    fetch_laggard_candidates, fetch_lhb_map,
};

use crate::analyzer::{AgentMode, GeminiAnalyzer};
use crate::database::DatabaseManager;
use crate::market_data::TopStock;

/// 泛指数/交易属性类板块黑名单（子串匹配）——不能作为产业链聚类键。
const GENERIC_BOARD_PATTERNS: &[&str] = &[
    "融资融券",
    "转融券",
    "沪股通",
    "深股通",
    "标准普尔",
    "标普",
    "MSCI",
    "富时罗素",
    "沪深300",
    "中证500",
    "上证380",
    "上证180",
    "央视50",
    "创业板综",
    "创业成份",
    "深成500",
    "深证100",
    "茅指数",
    "宁组合",
    "证金持股",
    "基金重仓",
    "机构重仓",
    "预盈预增",
    "预亏预减",
    "昨日涨停",
    "昨日连板",
    "昨日触板",
    "次新股",
    "新股与次新股",
    "ST股",
    "破净股",
    "低价股",
    "高送转",
    "股权激励",
    "AH股",
    "B股",
    "同花顺",
    "举牌",
    "百元股",
    "QFII重仓",
    "社保重仓",
    "国家队",
    "GDR",
    "注册制次新股",
    // 交易属性/形态/风格标签：会把整个涨停池聚成无意义大簇
    "昨日高振幅",
    "昨日首板",
    "昨日炸板",
    "昨日高换手",
    "最近多板",
    "昨日打二板",
    "东方财富热股",
    "近期新高",
    "历史新高",
    "百日新高",
    "趋势股",
    "题材股",
    "小盘股",
    "中盘股",
    "大盘股",
    "小盘成长",
    "小盘价值",
    "中盘成长",
    "中盘价值",
    "大盘成长",
    "大盘价值",
    "破发股",
    "破增发价股",
    "贬值受益",
    "HS300",
    "年报预增",
    "年报预减",
    "年报扭亏",
    "并购重组概念",
    "先进制造风格",
    "专精特新",
    "央国企改革",
    "中字头",
    "独角兽",
    "PPP模式",
    "参股银行",
    // 地域板块：不是产业链
    "板块",
    "特区",
    "自贸",
    "一带一路",
    "西部大开发",
    "成渝",
    "长江三角",
    "粤港",
];

/// 主线簇判定的最小涨停家数（可用 CHAIN_MIN_CLUSTER 环境变量覆盖）。
fn min_cluster_size() -> usize {
    std::env::var("CHAIN_MIN_CLUSTER")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3)
}

/// 主线分级阈值：涨停数 >= TIER1_MIN 为深度分析，TIER2_MIN..TIER1_MIN 为简化分析，其余汇入速览。
const TIER1_MIN: usize = 8;
const TIER2_MIN: usize = 4;
/// 深度分析上限（控制 API 成本与延迟）。
const MAX_DEEP_ANALYSIS: usize = 8;
/// 简化分析上限。
const MAX_SIMPLE_ANALYSIS: usize = 12;

/// Selects where already-resolved news evidence is committed into model prompts.
/// Production is permanently wired to `Live`; `Resolved` is a private test seam
/// for protocol-complete facts and never fabricates market or account data.
enum ChainEvidenceSource {
    Live,
    #[cfg(test)]
    Resolved(ResolvedChainEvidence),
}

#[cfg(test)]
struct ResolvedChainEvidence {
    cluster_news: HashMap<String, String>,
    after_market: String,
}

/// 五维主线量化评分（0-100）。
#[derive(Debug, Clone, Default)]
pub struct ChainScore {
    pub logic_hardness: f64,     // 产业逻辑硬度 25%
    pub sentiment_position: f64, // 情绪位置 25%
    pub fund_consensus: f64,     // 资金共识度 20%
    pub chip_health: f64,        // 筹码健康度 15%
    pub falsify_prob: f64,       // 证伪概率 15%（越低越好）
}

impl ChainScore {
    pub fn total(&self) -> f64 {
        self.logic_hardness * 0.25
            + self.sentiment_position * 0.25
            + self.fund_consensus * 0.20
            + self.chip_health * 0.15
            + (100.0 - self.falsify_prob) * 0.15
    }

    pub fn rating(&self) -> &'static str {
        let t = self.total();
        if t >= 80.0 {
            "积极参与"
        } else if t >= 60.0 {
            "适度参与"
        } else if t >= 40.0 {
            "轻仓试探"
        } else {
            "回避"
        }
    }

    pub fn position_cap(&self) -> &'static str {
        let t = self.total();
        if t >= 80.0 {
            "30%"
        } else if t >= 60.0 {
            "15%"
        } else if t >= 40.0 {
            "5%"
        } else {
            "0%"
        }
    }
}

/// 三情景推演。
#[derive(Debug, Clone, Default)]
pub struct ScenarioAnalysis {
    pub baseline_prob: f64,
    pub baseline_desc: String,
    pub bull_prob: f64,
    pub bull_desc: String,
    pub bear_prob: f64,
    pub bear_desc: String,
}

/// 一个产业链主线簇。
pub struct ChainCluster {
    /// 聚类键概念名
    pub concept: String,
    /// 与本簇成员高度重合而被合并的同义概念
    pub aliases: Vec<String>,
    /// 簇内涨停股
    pub stocks: Vec<TopStock>,
    /// 簇内带"昨日涨停/昨日连板"标签的家数（主线生命周期参考）
    pub continuation_count: usize,
    /// 该主线最近 10 天内上榜天数（含今日，来自 chain_daily 表）
    pub streak_days: i64,
    /// 同概念板块内未涨停的补涨候选（今日涨幅适中，供 LLM 筛选）
    pub candidates: Vec<TopStock>,
    /// 五维量化评分（LLM 分析后填充）
    pub score: Option<ChainScore>,
    /// 三情景推演（LLM 分析后填充）
    pub scenario: Option<ScenarioAnalysis>,
}

fn cluster_and_persist(
    date: &str,
    limit_ups: &[TopStock],
    concepts: &HashMap<String, Vec<String>>,
    min_size: usize,
) -> Result<(Vec<ChainCluster>, Vec<TopStock>)> {
    let (mut clusters, isolated) = cluster_by_concept(limit_ups, concepts, min_size);
    info!(
        "[产业链] 识别主线簇 {} 个，孤立涨停 {} 只",
        clusters.len(),
        isolated.len()
    );

    let db =
        DatabaseManager::try_get().ok_or_else(|| anyhow::anyhow!("产业链主线数据库未初始化"))?;
    let rows: Vec<(String, Vec<String>, i32)> = clusters
        .iter()
        .map(|cluster| {
            Ok((
                cluster.concept.clone(),
                cluster
                    .stocks
                    .iter()
                    .map(|stock| stock.code.clone())
                    .collect(),
                i32::try_from(cluster.continuation_count).map_err(|error| {
                    anyhow::anyhow!("主线 {} continuation_count 溢出: {error}", cluster.concept)
                })?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    db.save_chain_clusters(date, &rows)
        .map_err(anyhow::Error::msg)?;
    for cluster in &mut clusters {
        cluster.streak_days = db
            .get_chain_streak_days_strict(&cluster.concept, 10)
            .map_err(anyhow::Error::msg)?;
    }
    Ok((clusters, isolated))
}

fn resolve_cluster_board_code<'a>(
    cluster: &ChainCluster,
    board_map: &'a HashMap<String, String>,
) -> Result<&'a str> {
    let concept_head = cluster
        .concept
        .split_once('(')
        .map(|(head, _)| head)
        .unwrap_or(&cluster.concept);
    board_map
        .get(cluster.concept.as_str())
        .or_else(|| board_map.get(concept_head))
        .or_else(|| {
            board_map.iter().find_map(|(name, code)| {
                if name.contains(concept_head) || concept_head.contains(name.as_str()) {
                    Some(code)
                } else {
                    None
                }
            })
        })
        .or_else(|| resolve_concept_alias(&cluster.concept, board_map))
        .map(String::as_str)
        .ok_or_else(|| anyhow::anyhow!("产业链主线「{}」未匹配到概念板块代码", cluster.concept))
}

#[allow(clippy::too_many_arguments)]
async fn render_resolved_chain_analysis(
    analyzer: &GeminiAnalyzer,
    date: &str,
    limit_ups: &[TopStock],
    concepts: &HashMap<String, Vec<String>>,
    clusters: &[ChainCluster],
    isolated: &[TopStock],
    position_diags: &[PositionDiag],
    lhb_map: &HashMap<String, f64>,
    macro_ctx: &str,
    evidence_source: ChainEvidenceSource,
) -> Result<String> {
    let llm_ok = analyzer.is_available();
    if !llm_ok {
        warn!("[产业链] AI 模型未配置，仅输出聚类结果");
    }

    let mut cluster_sections: Vec<(String, Option<String>)> = Vec::new();
    let mut deep_count = 0;
    let mut simple_count = 0;
    for cluster in clusters.iter() {
        let stock_count = cluster.stocks.len();
        let analysis = if stock_count >= TIER1_MIN && llm_ok && deep_count < MAX_DEEP_ANALYSIS {
            deep_count += 1;
            let cluster_news = match &evidence_source {
                ChainEvidenceSource::Live => fetch_cluster_news(analyzer, cluster, concepts).await,
                #[cfg(test)]
                ChainEvidenceSource::Resolved(evidence) => evidence
                    .cluster_news
                    .get(&cluster.concept)
                    .cloned()
                    .unwrap_or_default(),
            };
            match analyze_cluster_deep(
                analyzer,
                cluster,
                concepts,
                lhb_map,
                macro_ctx,
                &cluster_news,
                date,
            )
            .await
            {
                Ok(text) => Some(text),
                Err(error) => {
                    warn!(
                        "[产业链] 主线「{}」深度分析失败: {}",
                        cluster.concept, error
                    );
                    None
                }
            }
        } else if stock_count >= TIER2_MIN && llm_ok && simple_count < MAX_SIMPLE_ANALYSIS {
            simple_count += 1;
            match analyze_cluster_simple(analyzer, cluster, concepts, lhb_map, date).await {
                Ok(text) => Some(text),
                Err(error) => {
                    warn!(
                        "[产业链] 主线「{}」简化分析失败: {}",
                        cluster.concept, error
                    );
                    None
                }
            }
        } else {
            None
        };
        cluster_sections.push((cluster.concept.clone(), analysis));
    }
    info!(
        "[产业链] LLM 分析: 深度={} 简化={} 仅聚类={}",
        deep_count,
        simple_count,
        clusters.len().saturating_sub(deep_count + simple_count)
    );

    let top_theme_names: Vec<&str> = clusters
        .iter()
        .take(5)
        .map(|cluster| cluster.concept.as_str())
        .collect();
    let after_market_section = if llm_ok {
        match &evidence_source {
            ChainEvidenceSource::Live => fetch_after_market_catalysts(&top_theme_names).await,
            #[cfg(test)]
            ChainEvidenceSource::Resolved(evidence) => evidence.after_market.clone(),
        }
    } else {
        String::new()
    };

    let overview = if llm_ok && !cluster_sections.is_empty() {
        synthesize_overview(
            analyzer,
            clusters,
            &cluster_sections,
            position_diags,
            date,
            &after_market_section,
        )
        .await
    } else {
        None
    };

    Ok(build_report(
        date,
        limit_ups,
        clusters,
        &cluster_sections,
        isolated,
        overview.as_deref(),
        &after_market_section,
        concepts,
        position_diags,
    ))
}

/// 入口：对当日涨停池做产业链联动分析，返回完整 Markdown 报告。
pub async fn run_chain_analysis(
    limit_ups: Vec<TopStock>,
    macro_news: Option<String>,
) -> Result<String> {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();

    if limit_ups.is_empty() {
        return Ok(format!(
            "# 产业链联动分析报告 {}\n\n涨停池批次成功返回 0 只，无可分析内容。\n",
            date
        ));
    }
    info!(
        "[产业链] 今日涨停池 {} 只，开始拉取概念标签...",
        limit_ups.len()
    );

    // 1. 概念标签（带缓存）
    let codes: Vec<String> = limit_ups.iter().map(|s| s.code.clone()).collect();
    let concepts = fetch_concepts_cached(&codes)
        .await
        .map_err(anyhow::Error::msg)?;

    // 2. 概念共现聚类 + 2.5 主线落库/生命周期
    let (mut clusters, isolated) =
        cluster_and_persist(&date, &limit_ups, &concepts, min_cluster_size())?;

    // 2.6 补涨候选：从东财概念板块成分股中找未涨停、涨幅适中的标的
    {
        let limit_codes: HashSet<String> = limit_ups.iter().map(|s| s.code.clone()).collect();
        let board_map = fetch_board_code_map().await.map_err(anyhow::Error::msg)?;
        info!("[产业链] 概念板块索引 {} 个", board_map.len());
        for c in clusters
            .iter_mut()
            .take(MAX_DEEP_ANALYSIS + MAX_SIMPLE_ANALYSIS)
        {
            let code = resolve_cluster_board_code(c, &board_map)?;
            c.candidates = fetch_laggard_candidates(code, &limit_codes)
                .await
                .map_err(anyhow::Error::msg)?;
        }
    }

    // 2.7 持仓主线诊断
    let position_diags = diagnose_positions(&clusters).await?;

    // 3. 龙虎榜净买入（完整真实批次，空集合仅表示今日确无记录）
    let lhb_map = fetch_lhb_map().await.map_err(anyhow::Error::msg)?;

    // 4. 宏观新闻（未传入则 best-effort 在线搜索）
    let macro_ctx = resolve_macro_news(macro_news).await;

    // 5. LLM 逐簇分析 + 5.5 盘后催化 + 6. 报告组装
    let analyzer = GeminiAnalyzer::from_env();
    render_resolved_chain_analysis(
        &analyzer,
        &date,
        &limit_ups,
        &concepts,
        &clusters,
        &isolated,
        &position_diags,
        &lhb_map,
        &macro_ctx,
        ChainEvidenceSource::Live,
    )
    .await
}

// ============================================================================
// 概念标签获取（DB 缓存 + 东财 F10）
// ============================================================================

// fetch_concepts_cached 已抽到 chain_analysis/fetchers.rs (修复 Top10#3+#4)

// fetch_boards_via_tool 已抽到 chain_analysis/fetchers.rs (修复 Top10#3+#4)

pub(super) fn is_generic_board(name: &str) -> bool {
    GENERIC_BOARD_PATTERNS.iter().any(|p| name.contains(p))
}

// ============================================================================
// 概念共现聚类
// ============================================================================

/// 按概念把涨停股聚类：出现 >= min_size 只涨停的概念视为主线簇。
///
/// 贪心去重：按家数降序选簇，若某概念成员与已选簇重合度 >= 70% 则并入别名。
/// 返回 (主线簇列表, 未进入任何簇的孤立涨停)。
fn cluster_by_concept(
    stocks: &[TopStock],
    concepts: &HashMap<String, Vec<String>>,
    min_size: usize,
) -> (Vec<ChainCluster>, Vec<TopStock>) {
    // 概念 -> 涨停股代码集合（剔除泛指数类概念）
    let mut concept_members: HashMap<&str, HashSet<&str>> = HashMap::new();
    for s in stocks {
        if let Some(boards) = concepts.get(&s.code) {
            for b in boards {
                if !is_generic_board(b) {
                    concept_members.entry(b).or_default().insert(&s.code);
                }
            }
        }
    }

    // 按家数降序、同数按概念名稳定排序
    let mut ranked: Vec<(&str, &HashSet<&str>)> = concept_members
        .iter()
        .filter(|(_, members)| members.len() >= min_size)
        .map(|(k, v)| (*k, v))
        .collect();
    ranked.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));

    let code_map: HashMap<&str, &TopStock> = stocks.iter().map(|s| (s.code.as_str(), s)).collect();

    let mut picked: Vec<ChainCluster> = Vec::new();
    let mut covered: HashSet<String> = HashSet::new();

    for (concept, members) in ranked {
        // 与某个已选簇重合度 >= 70% → 视为同义概念并入别名
        let mut merged = false;
        for cluster in picked.iter_mut() {
            let cluster_codes: HashSet<&str> =
                cluster.stocks.iter().map(|s| s.code.as_str()).collect();
            let overlap = members.intersection(&cluster_codes).count();
            if overlap * 10 >= members.len() * 7 {
                cluster.aliases.push(concept.to_string());
                merged = true;
                break;
            }
        }
        if merged {
            continue;
        }

        let mut cluster_stocks: Vec<TopStock> = members
            .iter()
            .filter_map(|c| code_map.get(c).map(|s| (*s).clone()))
            .collect();
        cluster_stocks.sort_by(|a, b| {
            b.change_pct
                .partial_cmp(&a.change_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // 主线生命周期参考：簇内带"昨日涨停/连板"标签的家数
        let continuation_count = cluster_stocks
            .iter()
            .filter(|s| {
                concepts.get(&s.code).is_some_and(|bs| {
                    bs.iter()
                        .any(|b| b.contains("昨日涨停") || b.contains("昨日连板"))
                })
            })
            .count();

        for s in &cluster_stocks {
            covered.insert(s.code.clone());
        }

        picked.push(ChainCluster {
            concept: concept.to_string(),
            aliases: Vec::new(),
            stocks: cluster_stocks,
            continuation_count,
            streak_days: 0,
            candidates: Vec::new(),
            score: None,
            scenario: None,
        });
    }

    let isolated: Vec<TopStock> = stocks
        .iter()
        .filter(|s| !covered.contains(s.code.as_str()))
        .cloned()
        .collect();

    (picked, isolated)
}

// ============================================================================
// 辅助数据：板块成分股(补涨候选) / 持仓诊断 / 龙虎榜 / 宏观新闻
// ============================================================================

/// push2 子域列表：单主机限流时回退。
pub(crate) const PUSH2_HOSTS: &[&str] = &[
    "https://push2.eastmoney.com",
    "https://17.push2.eastmoney.com",
    "https://82.push2.eastmoney.com",
];

/// 概念名 → 东财板块名的同义词/简称映射。
fn resolve_concept_alias<'a>(
    concept: &str,
    board_map: &'a HashMap<String, String>,
) -> Option<&'a String> {
    let concept_clean = concept.trim();
    // 常见同义词映射
    let aliases: &[&str] = match concept_clean {
        "电池技术" => &["电池", "固态电池", "锂电池", "锂电池概念", "新能源车"],
        "有色金属" => &["有色金属", "小金属", "工业金属", "黄金概念", "稀缺资源"],
        "军工" => &["军工", "国防军工", "航天航空", "军民融合", "商业航天"],
        "新材料" => &["新材料", "化工新材料"],
        "低空经济" => &["低空经济", "飞行汽车", "通用航空"],
        "基础化工" => &["基础化工", "化工", "化学制品"],
        "机器人概念" => &["机器人", "人形机器人", "机器人概念"],
        "华为概念" => &["华为", "华为概念", "华为产业链"],
        "5G概念" => &["5G", "5G概念", "通信"],
        "节能环保" => &["环保", "节能环保"],
        "人工智能" => &["人工智能", "AI"],
        "半导体概念" => &["半导体", "半导体概念", "芯片"],
        "国产芯片" => &["国产芯片", "芯片概念"],
        "汽车" => &["汽车", "汽车整车", "汽车零部件"],
        "电商概念" => &["电商", "电商概念", "网红经济"],
        "电子" => &["电子", "元件"],
        "电网概念" => &["电网", "智能电网", "特高压"],
        "第三代半导体" => &["第三代半导体", "碳化硅", "氮化镓"],
        "互联网金融" => &["互联网金融", "金融科技"],
        "光伏概念" => &["光伏", "光伏概念"],
        "商贸零售" => &["商贸零售", "零售"],
        "建筑装饰" => &["建筑装饰", "建筑"],
        "机械设备" => &["机械设备"],
        "核能核电" => &["核电", "核能"],
        "油气资源" => &["油气", "石油"],
        "网红经济" => &["网红经济", "直播"],
        "轻工制造" => &["轻工制造", "家具"],
        "PCB" => &["PCB", "印制电路板"],
        "信创" => &["信创", "国产软件"],
        "光刻机(胶)" => &["光刻机", "光刻胶"],
        "农业种植" => &["农业", "农业种植"],
        "医药生物" => &["医药", "医药生物"],
        "大数据" => &["大数据", "数据要素"],
        "影视概念" => &["影视", "传媒"],
        "数据中心" => &["数据中心", "算力"],
        "新能源" => &["新能源", "新能源车"],
        "智慧城市" => &["智慧城市"],
        "特斯拉概念" => &["特斯拉", "特斯拉概念"],
        "铁路基建" => &["铁路基建", "基建"],
        _ => &[],
    };
    for alias in aliases {
        if let Some(code) = board_map.get(*alias) {
            return Some(code);
        }
    }
    // 最后尝试子串匹配
    board_map.iter().find_map(|(k, v)| {
        if k.contains(concept_clean) || concept_clean.contains(k.as_str()) {
            Some(v)
        } else {
            None
        }
    })
}

// fetch_board_code_map 已抽到 chain_analysis/fetchers.rs (修复 Top10#3+#4)

// push2_get 已抽到 chain_analysis/fetchers.rs (修复 Top10#3+#4)

// fetch_laggard_candidates 已抽到 chain_analysis/fetchers.rs (修复 Top10#3+#4)

/// 持仓主线诊断结果。
pub struct PositionDiag {
    pub code: String,
    pub name: String,
    pub return_rate: Option<f64>,
    /// 命中的主线概念（含 streak 天数），无则为空
    pub mainline: Option<(String, i64)>,
    /// 今日是否涨停（在主线簇成员中）
    pub in_limit_pool: bool,
}

/// 持仓股与今日主线的归属诊断（确定性本地匹配，不依赖 LLM）。
async fn diagnose_positions(clusters: &[ChainCluster]) -> Result<Vec<PositionDiag>> {
    let db =
        DatabaseManager::try_get().ok_or_else(|| anyhow::anyhow!("持仓主线诊断数据库未初始化"))?;
    let positions = db
        .get_all_open_positions()
        .map_err(|error| anyhow::anyhow!("持仓主线诊断查询失败: {error}"))?;
    if positions.is_empty() {
        return Ok(Vec::new());
    }
    // 持仓股的概念标签（带缓存）
    let codes: Vec<String> = positions.iter().map(|p| p.code.clone()).collect();
    let concept_map = fetch_concepts_cached(&codes)
        .await
        .map_err(anyhow::Error::msg)?;

    Ok(positions
        .iter()
        .map(|p| {
            let in_limit_pool = clusters
                .iter()
                .any(|c| c.stocks.iter().any(|s| s.code == p.code));
            // 优先按簇成员直接匹配，其次按概念标签匹配
            let mainline = clusters
                .iter()
                .find(|c| {
                    c.stocks.iter().any(|s| s.code == p.code)
                        || concept_map.get(&p.code).is_some_and(|tags| {
                            tags.iter()
                                .any(|t| t == &c.concept || c.aliases.contains(t))
                        })
                })
                .map(|c| (c.concept.clone(), c.streak_days));
            PositionDiag {
                code: p.code.clone(),
                name: p.name.clone(),
                return_rate: p.return_rate,
                mainline,
                in_limit_pool,
            }
        })
        .collect())
}

// fetch_lhb_map 已抽到 chain_analysis/fetchers.rs (修复 Top10#3+#4)

// fetch_after_market_catalysts 已抽到 chain_analysis/fetchers.rs (修复 Top10#3+#4)

/// 宏观新闻：优先复用传入文本，否则 15s 超时在线搜索（best-effort）。
async fn resolve_macro_news(prefetched: Option<String>) -> String {
    if let Some(mc) = prefetched {
        if !mc.trim().is_empty() {
            return mc;
        }
    }
    let search = crate::search_service::get_search_service();
    match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        search.search_macro_news(3),
    )
    .await
    {
        Ok(text) => text,
        Err(_) => {
            warn!("[产业链] 宏观新闻搜索超时");
            String::new()
        }
    }
}

// ============================================================================
// LLM 分析
// ============================================================================

const CHAIN_SYSTEM_PROMPT: &str = r#"你是 A 股产业链结构分析专家，任务是对"今日涨停股聚类出的主线"做产业链上下游定位与预期推演。

## 方法论（BOM 拆解法）
以该主线的终端产品/场景为圆心拆解产业链：上游原材料·设备 → 中游制造·组件 → 下游集成·应用。
定位每只涨停股所处环节，判断价值重心正向哪个环节迁移，该环节是否具备"高增长/高利润/高壁垒"。

## 硬性纪律（必须遵守）
1. 禁止编造数据。对每只股票的链上定位必须标注证据等级：
   - [A] 该业务是公司主营（你确知）
   - [B] 公司有该业务布局，但营收占比可能不高
   - [C] 仅凭概念标签，无法确认实际业务占比（蹭概念嫌疑）
2. 概念标签来自东财 F10，存在大量蹭概念，对 [C] 级标的必须明确风险提示。
3. 本报告是供使用者本人决策参考的工具。你需要给出明确的**参与评级**（可关注/谨慎/回避）与倾向性结论，但不给具体仓位、买点价格。评级原则：首日启动+产业逻辑硬 → 可关注；发酵 2-3 天 → 谨慎；高潮/分歧退潮或纯情绪无逻辑 → 回避。
4. 必须判断主线情绪阶段（首日启动/发酵中/高潮/分歧退潮）；若簇内多为连板股，明确警示次日溢价与接力风险。
5. 不确定就写"不确定"，不要为了叙事完整而强行自洽。"#;

// fetch_cluster_news 已抽到 chain_analysis/fetchers.rs (修复 Top10#3+#4)

/// 对单个主线簇做深度 LLM 产业链分析（含五维评分 + 三情景推演）。
async fn analyze_cluster_deep(
    analyzer: &GeminiAnalyzer,
    cluster: &ChainCluster,
    concepts: &HashMap<String, Vec<String>>,
    lhb_map: &HashMap<String, f64>,
    macro_news: &str,
    cluster_news: &str,
    date: &str,
) -> Result<String> {
    let mut table = String::from(
        "| 代码 | 名称 | 涨幅% | 龙虎榜净买入(万) | 其他概念标签 |\n|---|---|---|---|---|\n",
    );
    for s in &cluster.stocks {
        let lhb = lhb_map
            .get(&s.code)
            .map(|v| format!("{:.0}", v))
            .unwrap_or_else(|| "-".to_string());
        let others: Vec<&str> = concepts
            .get(&s.code)
            .map(|bs| {
                bs.iter()
                    .filter(|b| !is_generic_board(b) && b.as_str() != cluster.concept)
                    .map(|b| b.as_str())
                    .take(8)
                    .collect()
            })
            .unwrap_or_default();
        table.push_str(&format!(
            "| {} | {} | {:.2} | {} | {} |\n",
            s.code,
            s.name,
            s.change_pct,
            lhb,
            others.join("、")
        ));
    }

    let aliases = if cluster.aliases.is_empty() {
        String::new()
    } else {
        format!("（同义概念：{}）", cluster.aliases.join("、"))
    };

    let macro_block = if macro_news.trim().is_empty() {
        String::from("（今日无宏观新闻上下文）")
    } else {
        let truncated: String = macro_news.chars().take(2500).collect();
        format!("<macro_news>\n{}\n</macro_news>", truncated)
    };

    let cluster_news_block = if cluster_news.trim().is_empty() {
        String::from("（未检索到该主线的定向新闻）")
    } else {
        let truncated: String = cluster_news.chars().take(1500).collect();
        format!("<主线定向新闻>\n{}\n</主线定向新闻>", truncated)
    };

    // 补涨候选块：同板块内未涨停、涨幅适中的标的
    let candidate_block = if cluster.candidates.is_empty() {
        String::from("（未获取到同板块未涨停成分股，候选评估跳过）")
    } else {
        let mut b = String::from("| 代码 | 名称 | 今日涨幅% |\n|---|---|---|\n");
        for s in &cluster.candidates {
            b.push_str(&format!(
                "| {} | {} | {:.2} |\n",
                s.code, s.name, s.change_pct
            ));
        }
        b
    };

    let prompt = format!(
        r#"今天是 {date}。今日 A 股涨停池中，概念「{concept}」{aliases}聚集了 {n} 只涨停股：

{table}
簇内带"昨日涨停/昨日连板"标签的家数：{cont}；该主线最近 10 个交易日内上榜天数：{streak}（含今日，天数越多越老）

同板块内今日**未涨停**、涨幅适中的股票（补涨候选池，仅供筛选）：
{candidates}

该主线的定向新闻检索结果（产业级事件，优先于宏观新闻作为催化依据）：
{cluster_news}

今日宏观新闻参考：
{macro_block}

请输出（Markdown，总字数 ≤ 1000）：

**第一行必须是一行固定格式的结论（不加标题、不加引号）：**
【结论】阶段=xx｜参与=可关注/谨慎/回避｜候选=名称(代码)、名称(代码)（无合适候选则写"无"）

**第二行必须是五维评分（不加标题、不加引号）：**
【评分】产业逻辑={{lh}}/100｜情绪位置={{sp}}/100｜资金共识={{fc}}/100｜筹码健康={{ch}}/100｜证伪概率={{fp}}/100

### 产业链图谱与个股定位
用一行概括"上游 → 中游 → 下游"，然后把上述每只涨停股归位到对应环节，逐只标注证据等级 [A]/[B]/[C]。

### 本轮催化与价值迁移
催化是什么：**优先引用上面的主线定向新闻**（产业级事件如上游停产、价格暴涨、替代材料、政策落地），其次才是宏观新闻；两者都对不上就写"催化不明，疑似纯情绪/资金行为"。价值重心正向哪个环节迁移；该环节的"三高"（高增长/高利润/高壁垒）成色如何。

### 三情景推演（替代情绪阶段描述）
**基准情景（概率评估）**：触发条件｜板块预期表现｜应对策略
**乐观情景（概率评估）**：触发条件｜板块预期表现｜应对策略
**悲观情景（概率评估）**：触发条件｜板块预期表现｜应对策略
注：三种概率之和应为100%。证伪点融入悲观情景触发条件。

### 补涨候选评估
从上面的候选池中挑出最多 3 只位于"价值迁移指向环节"的标的，每只一行：名称(代码)｜所处环节｜证据等级｜一句理由。候选池里没有真正在链上的公司就写"候选池内无合适标的"，不要凑数。[C] 级不得入选。这些候选必须与第一行【结论】中的候选一致。"#,
        date = date,
        concept = cluster.concept,
        aliases = aliases,
        n = cluster.stocks.len(),
        table = table,
        cont = cluster.continuation_count,
        streak = cluster.streak_days.max(1),
        candidates = candidate_block,
        cluster_news = cluster_news_block,
        macro_block = macro_block,
    );

    analyzer
        .call_api_mode(&prompt, CHAIN_SYSTEM_PROMPT, AgentMode::Deep)
        .await
}

/// 对 tier2 主线簇做简化 LLM 分析（三句话 + 评分 + 补涨候选）。
async fn analyze_cluster_simple(
    analyzer: &GeminiAnalyzer,
    cluster: &ChainCluster,
    _concepts: &HashMap<String, Vec<String>>,
    lhb_map: &HashMap<String, f64>,
    date: &str,
) -> Result<String> {
    let leaders: Vec<String> = cluster
        .stocks
        .iter()
        .take(5)
        .map(|s| format!("{}({})", s.name, s.code))
        .collect();

    let lhb_note: Vec<String> = cluster
        .stocks
        .iter()
        .filter_map(|s| {
            lhb_map
                .get(&s.code)
                .map(|v| format!("{}({}):龙虎榜净买入{}万", s.name, s.code, v))
        })
        .take(3)
        .collect();

    let candidate_block = if cluster.candidates.is_empty() {
        String::from("（无补涨候选）")
    } else {
        let mut b = String::from("| 代码 | 名称 | 今日涨幅% |\n|---|---|---|\n");
        for s in cluster.candidates.iter().take(5) {
            b.push_str(&format!(
                "| {} | {} | {:.2} |\n",
                s.code, s.name, s.change_pct
            ));
        }
        b
    };

    let prompt = format!(
        r#"今天是 {date}。概念「{concept}」聚集了 {n} 只涨停股，领头羊：{leaders}。
连板家数：{cont}，近10日上榜 {streak} 天。

补涨候选池：
{candidates}

龙虎榜（仅列出净买入>0的）：
{lhb}

请用 ≤200 字简析（Markdown）：
第一行：【简评】阶段=xx｜参与=可关注/谨慎/回避｜候选=代码（无则写无）
第二行：【评分】产业逻辑={{lh}}/100｜情绪位置={{sp}}/100｜资金共识={{fc}}/100｜筹码健康={{ch}}/100｜证伪概率={{fp}}/100
然后一段话：本轮催化 + 龙头定位 + 明日预判（分歧/一致/退潮）+ 风险警示。"#,
        date = date,
        concept = cluster.concept,
        n = cluster.stocks.len(),
        leaders = leaders.join("、"),
        cont = cluster.continuation_count,
        streak = cluster.streak_days.max(1),
        candidates = candidate_block,
        lhb = if lhb_note.is_empty() {
            "无".to_string()
        } else {
            lhb_note.join("；")
        },
    );

    analyzer
        .call_api_mode(&prompt, CHAIN_SYSTEM_PROMPT, AgentMode::Quick)
        .await
}

/// 从分析文本中解析五维评分。
fn parse_chain_score(analysis: &str) -> Option<ChainScore> {
    let line = analysis
        .lines()
        .map(|l| l.trim())
        .find(|l| l.starts_with("【评分】") || l.starts_with("【简评】"))?;

    // 从简评行里提取评分（简评的评分在第二行，这里只处理评分行）
    let score_line = if line.starts_with("【评分】") {
        line
    } else {
        // 找下一行
        analysis
            .lines()
            .find(|l| l.trim().starts_with("【评分】"))?
            .trim()
    };

    let mut lh = None;
    let mut sp = None;
    let mut fc = None;
    let mut ch = None;
    let mut fp = None;

    for part in score_line.split(['｜', '|']) {
        let part = part.trim().trim_start_matches("【评分】").trim();
        if let Some(rest) = part.strip_prefix("产业逻辑=") {
            lh = rest.trim().trim_end_matches("/100").parse().ok();
        } else if let Some(rest) = part.strip_prefix("情绪位置=") {
            sp = rest.trim().trim_end_matches("/100").parse().ok();
        } else if let Some(rest) = part.strip_prefix("资金共识=") {
            fc = rest.trim().trim_end_matches("/100").parse().ok();
        } else if let Some(rest) = part.strip_prefix("筹码健康=") {
            ch = rest.trim().trim_end_matches("/100").parse().ok();
        } else if let Some(rest) = part.strip_prefix("证伪概率=") {
            fp = rest.trim().trim_end_matches("/100").parse().ok();
        }
    }

    Some(ChainScore {
        logic_hardness: lh?,
        sentiment_position: sp?,
        fund_consensus: fc?,
        chip_health: ch?,
        falsify_prob: fp?,
    })
}

/// 全景研判：综合各主线分析，输出跨链关系、主线强弱排序与持仓诊断。
async fn synthesize_overview(
    analyzer: &GeminiAnalyzer,
    clusters: &[ChainCluster],
    sections: &[(String, Option<String>)],
    position_diags: &[PositionDiag],
    date: &str,
    after_market_section: &str,
) -> Option<String> {
    let mut ctx = String::new();
    for ((concept, analysis), cluster) in sections.iter().zip(clusters.iter()) {
        ctx.push_str(&format!(
            "### 主线「{}」（{} 只涨停，昨日涨停标签 {} 家，近 10 日上榜 {} 天）\n",
            concept,
            cluster.stocks.len(),
            cluster.continuation_count,
            cluster.streak_days.max(1)
        ));
        if let Some(a) = analysis {
            let truncated: String = a.chars().take(1200).collect();
            ctx.push_str(&truncated);
        }
        ctx.push('\n');
    }

    let position_block = if position_diags.is_empty() {
        String::from("（当前无持仓）")
    } else {
        let mut b = String::from(
            "| 代码 | 名称 | 持仓收益% | 主线归属 | 今日是否涨停 |\n|---|---|---|---|---|\n",
        );
        for d in position_diags {
            let return_rate = d
                .return_rate
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "暂无".to_string());
            let ml = d
                .mainline
                .as_ref()
                .map(|(c, days)| format!("{}（上榜{}天）", c, days.max(&1)))
                .unwrap_or_else(|| "无（不在今日任何主线）".to_string());
            b.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                d.code,
                d.name,
                return_rate,
                ml,
                if d.in_limit_pool { "是" } else { "否" }
            ));
        }
        b
    };

    let after_market_block = if after_market_section.trim().is_empty() {
        String::from("（无盘后催化信息）")
    } else {
        let truncated: String = after_market_section.chars().take(800).collect();
        truncated
    };

    let prompt = format!(
        r#"今天是 {date}。以下是今日各涨停主线的产业链分析摘要（含五维评分）：

{ctx}

<盘后催化追踪（收盘后至现在的最新消息，时效性最高，优先参考）>
{after_market}
</盘后催化追踪>

使用者当前持仓与主线归属诊断：
{positions}

请输出四部分（Markdown，总字数 ≤ 1000，**不要自己加一级标题**，直接输出内容）：

### 盘后催化更新（如有）
如果盘后催化中有与今日主线直接相关的消息，简要说明它对主线逻辑的强化/削弱。如果盘后催化为空或无相关内容，跳过此节。

### 核心矛盾与主线优先级
一句话点出今日最大矛盾。然后按总评分从高到低列出TOP5主线：名称(评分/100)｜理由（一句）。**盘后催化中如有相关新信息，应据此调整评分和排序。** 低于40分的主线不列出。

### 跨链关系与情绪推演
1. 哪些主线是同一产业链的上下游？真主线是什么、跟风/延伸是什么？
2. 整体情绪温度（极热/偏热/中性/偏冷/冰点）与明日预判（一致加速/弱分歧/强分歧/退潮）。**盘后催化中如有重大利空/利多，应据此修正明日预判方向。**
3. 关键观察点：明日前需要盯盘的事件/信号（大宗价格、竞价封单、政策发布等），不超过3条。

### 持仓诊断
逐只持仓一行结论：名称(代码)｜倾向=继续持有/减仓观察/离场｜建议仓位上限(%)｜一句理由。
判断依据：是否在今日主线上、主线阶段与评分、持仓盈亏状态。不在任何主线的持仓写"与今日主线无关，按个股逻辑处理"。

### 组合行动建议
一句话总结今天的组合调整方向（例："减仓4支→空出60%资金→如强分歧后回暖则下午低吸军工龙头"）。这是倾向性参考，不是交易指令。"#,
        date = date,
        ctx = ctx,
        after_market = after_market_block,
        positions = position_block,
    );

    match analyzer
        .call_api_mode(&prompt, CHAIN_SYSTEM_PROMPT, AgentMode::Quick)
        .await
    {
        Ok(text) if !text.trim().is_empty() => Some(text),
        Ok(_) => None,
        Err(e) => {
            warn!("[产业链] 全景研判失败: {}", e);
            None
        }
    }
}

// ============================================================================
// 报告组装
// ============================================================================

/// 从分析文本中解析结论行（兼容【结论】和【简评】两种格式）。
fn parse_conclusion(analysis: &str) -> Option<(String, String, String)> {
    // 先尝试深度分析格式【结论】
    let line = analysis
        .lines()
        .map(|l| l.trim())
        .find(|l| l.starts_with("【结论】") || l.starts_with("【简评】"))?;
    let prefix = if line.starts_with("【结论】") {
        "【结论】"
    } else {
        "【简评】"
    };
    let body = line.trim_start_matches(prefix);
    let mut stage = None;
    let mut rating = None;
    let mut cands = None;
    for part in body.split(['｜', '|']) {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("阶段=") {
            stage = Some(v.trim().to_string());
        } else if let Some(v) = part.strip_prefix("参与=") {
            rating = Some(v.trim().to_string());
        } else if let Some(v) = part.strip_prefix("候选=") {
            cands = Some(v.trim().to_string());
        }
    }
    Some((
        stage.unwrap_or_else(|| "-".into()),
        rating.unwrap_or_else(|| "-".into()),
        cands.unwrap_or_else(|| "-".into()),
    ))
}

#[allow(clippy::too_many_arguments)]
fn build_report(
    date: &str,
    limit_ups: &[TopStock],
    clusters: &[ChainCluster],
    sections: &[(String, Option<String>)],
    isolated: &[TopStock],
    overview: Option<&str>,
    after_market_section: &str,
    concepts: &HashMap<String, Vec<String>>,
    position_diags: &[PositionDiag],
) -> String {
    // 解析每条主线的评分
    let scores: Vec<Option<ChainScore>> = sections
        .iter()
        .map(|(_, analysis)| analysis.as_deref().and_then(parse_chain_score))
        .collect();

    let mut md = String::new();
    md.push_str(&format!("# 产业链主线决策参考 {}\n\n", date));
    md.push_str(&format!(
        "> 涨停 **{}** 只｜主线 **{}** 条（深度分析 {} 条 + 简化分析 {} 条）｜孤立 **{}** 只。概念标签来自东财 F10 存在蹭概念污染，结论为倾向性参考而非交易指令。\n\n",
        limit_ups.len(),
        clusters.len(),
        sections.iter().filter(|(_, a)| a.is_some() && a.as_ref().is_some_and(|t| t.contains("【结论】"))).count(),
        sections.iter().filter(|(_, a)| a.is_some() && a.as_ref().is_some_and(|t| t.contains("【简评】"))).count(),
        isolated.len()
    ));

    // ---- 盘后催化追踪（置顶，最高时效性）----
    if !after_market_section.trim().is_empty() {
        md.push_str(after_market_section.trim());
        md.push_str("\n\n");
    }

    // ---- 一页纸决策摘要（含五维评分）----
    md.push_str("## 一页纸决策摘要\n\n");
    md.push_str("| 主线 | 涨停数 | 上榜天 | 评分 | 阶段 | 参与 | 补涨候选 |\n|---|---|---|---|---|---|---|\n");
    for (i, ((concept, analysis), cluster)) in sections.iter().zip(clusters.iter()).enumerate() {
        let (stage, rating, cands) = analysis
            .as_deref()
            .and_then(parse_conclusion)
            .unwrap_or_else(|| ("-".into(), "-".into(), "-".into()));
        let score_str = scores
            .get(i)
            .and_then(|s| s.as_ref())
            .map(|s| format!("{:.0}", s.total()))
            .unwrap_or_else(|| "-".into());
        let rating_display = if rating == "-" { "-" } else { &rating };
        md.push_str(&format!(
            "| {} | {} | {}天 | {} | {} | {} | {} |\n",
            concept,
            cluster.stocks.len(),
            cluster.streak_days.max(1),
            score_str,
            stage,
            rating_display,
            if cands == "-" || cands == "无" {
                "-".to_string()
            } else {
                cands
            }
        ));
    }
    md.push('\n');

    // ---- 评分分布概览 ----
    {
        let scored: Vec<&ChainScore> = scores.iter().filter_map(|s| s.as_ref()).collect();
        if !scored.is_empty() {
            let avg = scored.iter().map(|s| s.total()).sum::<f64>() / scored.len() as f64;
            let top = scored.iter().map(|s| s.total()).fold(f64::NAN, f64::max);
            let bot = scored.iter().map(|s| s.total()).fold(f64::NAN, f64::min);
            let active = scored.iter().filter(|s| s.total() >= 60.0).count();
            md.push_str(&format!(
                "> 📊 主线评分概览：均值 **{:.0}** / 最高 **{:.0}** / 最低 **{:.0}** | ≥60分（可参与）：**{}** 条 | <40分（回避）：**{}** 条\n\n",
                avg,
                top,
                bot,
                active,
                scored.iter().filter(|s| s.total() < 40.0).count()
            ));
        }
    }

    // ---- 持仓主线诊断（本地确定性匹配）----
    if !position_diags.is_empty() {
        md.push_str("## 持仓主线诊断\n\n");
        md.push_str("| 持仓 | 收益% | 今日涨停 | 主线归属 | 主线评分 | 上榜天数 |\n|---|---|---|---|---|---|\n");
        for d in position_diags {
            let return_rate = d
                .return_rate
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "暂无".to_string());
            let (ml, score_str, days) = match &d.mainline {
                Some((c, days)) => {
                    let sc = clusters
                        .iter()
                        .position(|cl| &cl.concept == c)
                        .and_then(|idx| scores.get(idx))
                        .and_then(|s| s.as_ref())
                        .map(|s| format!("{:.0}", s.total()))
                        .unwrap_or_else(|| "-".into());
                    (c.as_str(), sc, format!("{}天", days.max(&1)))
                }
                None => ("无（不在今日主线）", "-".to_string(), "-".to_string()),
            };
            md.push_str(&format!(
                "| {}({}) | {} | {} | {} | {} | {} |\n",
                d.name,
                d.code,
                return_rate,
                if d.in_limit_pool { "✅" } else { "—" },
                ml,
                score_str,
                days
            ));
        }
        md.push_str("\n持有/离场倾向见下方「全景研判」持仓诊断部分。\n\n");
    }

    // ---- 全景研判（含 LLM 持仓倾向）----
    if let Some(ov) = overview {
        md.push_str("## 今日主线全景研判\n\n");
        let trimmed = ov.trim();
        let cleaned = trimmed
            .lines()
            .skip_while(|l| {
                let t = l.trim();
                t.is_empty() || (t.starts_with('#') && t.contains("全景研判"))
            })
            .collect::<Vec<_>>()
            .join("\n");
        md.push_str(cleaned.trim());
        md.push_str("\n\n");
    }

    // ---- 主线成员总表 ----
    if !clusters.is_empty() {
        md.push_str("## 主线成员一览\n\n");
        md.push_str("| 主线概念 | 涨停家数 | 昨日涨停标签 | 成员 |\n|---|---|---|---|\n");
        for c in clusters {
            let members: Vec<String> = c
                .stocks
                .iter()
                .map(|s| format!("{}({})", s.name, s.code))
                .collect();
            md.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                c.concept,
                c.stocks.len(),
                c.continuation_count,
                members.join("、")
            ));
        }
        md.push('\n');
    }

    // ---- 各主线详情（深度/简化分析者展示，仅聚类者汇入速览表）----
    let mut hot_list: Vec<(&ChainCluster, usize)> = Vec::new();
    for (i, ((concept, analysis), cluster)) in sections.iter().zip(clusters.iter()).enumerate() {
        let has_analysis = analysis.is_some();
        if !has_analysis {
            hot_list.push((cluster, i));
            continue;
        }

        let alias_note = if cluster.aliases.is_empty() {
            String::new()
        } else {
            format!("（同义概念：{}）", cluster.aliases.join("、"))
        };
        md.push_str(&format!("## 主线{}：{}{}\n\n", i + 1, concept, alias_note));

        // 展示评分卡
        if let Some(score) = scores.get(i).and_then(|s| s.as_ref()) {
            md.push_str(
                "| 维度 | 产业逻辑 | 情绪位置 | 资金共识 | 筹码健康 | 证伪概率 | **总评分** |\n",
            );
            md.push_str("|---|---|---|---|---|---|---|\n");
            md.push_str(&format!(
                "| 得分 | {:.0}/100 | {:.0}/100 | {:.0}/100 | {:.0}/100 | {:.0}/100 | **{:.0}/100** |\n",
                score.logic_hardness,
                score.sentiment_position,
                score.fund_consensus,
                score.chip_health,
                score.falsify_prob,
                score.total()
            ));
            md.push_str(&format!(
                "| 评级 | | | | | | **{}** (仓位上限 {}) |\n\n",
                score.rating(),
                score.position_cap()
            ));
        }

        if let Some(a) = analysis {
            // 去掉评分行避免在正文中重复显示（已在表格中展示）
            let cleaned_analysis: String = a
                .lines()
                .filter(|l| {
                    let t = l.trim();
                    !t.starts_with("【评分】") && !t.starts_with("【结论】")
                })
                .collect::<Vec<_>>()
                .join("\n");
            md.push_str(cleaned_analysis.trim());
            md.push_str("\n\n");
        }
    }

    // ---- 其他热点速览（tier3，未展开分析的主线）----
    if !hot_list.is_empty() {
        md.push_str("## 其他热点速览（未形成强产业链联动，仅展示聚类）\n\n");
        md.push_str("| 主线 | 涨停数 | 连板数 | 龙头股 |\n|---|---|---|---|\n");
        for (c, _idx) in &hot_list {
            let leaders: Vec<String> = c
                .stocks
                .iter()
                .take(3)
                .map(|s| format!("{}({})", s.name, s.code))
                .collect();
            md.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                c.concept,
                c.stocks.len(),
                c.continuation_count,
                leaders.join("、")
            ));
        }
        md.push('\n');
    }

    // 孤立涨停
    if !isolated.is_empty() {
        md.push_str("## 孤立涨停（未形成产业链联动，按个股逻辑对待）\n\n");
        md.push_str("| 代码 | 名称 | 涨幅% | 概念标签（前5） |\n|---|---|---|---|\n");
        for s in isolated {
            let tags: Vec<&str> = concepts
                .get(&s.code)
                .map(|bs| {
                    bs.iter()
                        .filter(|b| !is_generic_board(b))
                        .map(|b| b.as_str())
                        .take(5)
                        .collect()
                })
                .unwrap_or_default();
            md.push_str(&format!(
                "| {} | {} | {:.2} | {} |\n",
                s.code,
                s.name,
                s.change_pct,
                tags.join("、")
            ));
        }
        md.push('\n');
    }

    md.push_str("---\n*报告由产业链联动分析模块自动生成。评分基于 LLM 对五维指标的评估，为倾向性参考而非交易指令。孤立涨停的一日游风险显著高于主线内涨停；主线内 [C] 级标的请勿当作产业链逻辑成立的依据。*\n");
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::prelude::*;
    use once_cell::sync::Lazy;
    use std::sync::Mutex;

    static CHAIN_ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    fn top(code: &str, name: &str, change_pct: f64) -> TopStock {
        TopStock {
            code: code.into(),
            name: name.into(),
            change_pct,
            price: 10.0,
            ..Default::default()
        }
    }

    fn unavailable_analyzer() -> GeminiAnalyzer {
        GeminiAnalyzer::new(crate::analyzer::GeminiConfig {
            max_retries: 1,
            retry_delay: 0.0,
            request_delay: 0.0,
            ..crate::analyzer::GeminiConfig::default()
        })
    }

    #[test]
    fn test_chain_score_total() {
        let score = ChainScore {
            logic_hardness: 80.0,
            sentiment_position: 60.0,
            fund_consensus: 75.0,
            chip_health: 50.0,
            falsify_prob: 40.0,
        };
        // 80*0.25 + 60*0.25 + 75*0.20 + 50*0.15 + (100-40)*0.15
        // = 20 + 15 + 15 + 7.5 + 9 = 66.5
        let total = score.total();
        assert!((total - 66.5).abs() < 0.01, "expected 66.5, got {}", total);
    }

    #[test]
    fn test_chain_score_rating() {
        let high = ChainScore {
            logic_hardness: 85.0,
            sentiment_position: 85.0,
            fund_consensus: 85.0,
            chip_health: 85.0,
            falsify_prob: 10.0,
        };
        assert_eq!(high.rating(), "积极参与");

        let mid = ChainScore {
            logic_hardness: 65.0,
            sentiment_position: 65.0,
            fund_consensus: 65.0,
            chip_health: 65.0,
            falsify_prob: 35.0,
        };
        assert_eq!(mid.rating(), "适度参与");

        let low = ChainScore {
            logic_hardness: 45.0,
            sentiment_position: 45.0,
            fund_consensus: 45.0,
            chip_health: 45.0,
            falsify_prob: 55.0,
        };
        assert_eq!(low.rating(), "轻仓试探");

        let avoid = ChainScore {
            logic_hardness: 30.0,
            sentiment_position: 30.0,
            fund_consensus: 30.0,
            chip_health: 30.0,
            falsify_prob: 80.0,
        };
        assert_eq!(avoid.rating(), "回避");
    }

    #[test]
    fn test_parse_chain_score_from_deep_analysis() {
        let analysis = "【结论】阶段=高潮｜参与=谨慎｜候选=无\n【评分】产业逻辑=75/100｜情绪位置=30/100｜资金共识=60/100｜筹码健康=45/100｜证伪概率=55/100\n### 产业链图谱";
        let score = parse_chain_score(analysis).expect("should parse score");
        assert!((score.logic_hardness - 75.0).abs() < 0.01);
        assert!((score.sentiment_position - 30.0).abs() < 0.01);
        assert!((score.fund_consensus - 60.0).abs() < 0.01);
        assert!((score.chip_health - 45.0).abs() < 0.01);
        assert!((score.falsify_prob - 55.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_chain_score_from_simple_analysis() {
        let analysis = "【简评】阶段=发酵中｜参与=可关注｜候选=600123\n【评分】产业逻辑=80/100｜情绪位置=65/100｜资金共识=70/100｜筹码健康=55/100｜证伪概率=30/100\n催化：新能源政策利好";
        let score = parse_chain_score(analysis).expect("should parse score from simple analysis");
        assert!((score.logic_hardness - 80.0).abs() < 0.01);
        assert!((score.sentiment_position - 65.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_conclusion_deep_format() {
        let analysis = "【结论】阶段=高潮｜参与=谨慎｜候选=无\n一些正文...";
        let (stage, rating, cands) = parse_conclusion(analysis).expect("should parse");
        assert_eq!(stage, "高潮");
        assert_eq!(rating, "谨慎");
        assert_eq!(cands, "无");
    }

    #[test]
    fn test_parse_conclusion_simple_format() {
        // Protocol-format exception: this parser consumes provider text whose
        // stock symbols are native six-digit values.
        let analysis = "【简评】阶段=发酵中｜参与=可关注｜候选=600123\n催化...";
        let (stage, rating, cands) =
            parse_conclusion(analysis).expect("should parse simple format");
        assert_eq!(stage, "发酵中");
        assert_eq!(rating, "可关注");
        assert_eq!(cands, "600123");
    }

    #[test]
    fn test_parse_conclusion_with_candidates() {
        let analysis =
            "【结论】阶段=首日启动｜参与=可关注｜候选=恩捷股份(002812)、雄韬股份(002733)\n内容";
        let (stage, rating, cands) = parse_conclusion(analysis).expect("should parse");
        assert_eq!(stage, "首日启动");
        assert_eq!(rating, "可关注");
        assert!(cands.contains("恩捷股份"));
        assert!(cands.contains("雄韬股份"));
    }

    #[test]
    fn test_resolve_concept_alias_direct_match() {
        let mut map = HashMap::new();
        map.insert("固态电池".to_string(), "BK0001".to_string());
        let result = resolve_concept_alias("电池技术", &map);
        assert_eq!(result, Some(&"BK0001".to_string()));
    }

    #[test]
    fn test_resolve_concept_alias_substring_match() {
        let mut map = HashMap::new();
        map.insert("机器人概念板块".to_string(), "BK0002".to_string());
        let result = resolve_concept_alias("机器人概念", &map);
        assert_eq!(result, Some(&"BK0002".to_string()));
    }

    #[test]
    fn test_resolve_concept_alias_no_match() {
        let map: HashMap<String, String> = HashMap::new();
        let result = resolve_concept_alias("不存在的概念", &map);
        assert!(result.is_none());
    }

    #[test]
    fn all_registered_concept_alias_families_resolve_without_network() {
        for (concept, board) in [
            ("电池技术", "电池"),
            ("有色金属", "有色金属"),
            ("军工", "军工"),
            ("新材料", "新材料"),
            ("低空经济", "低空经济"),
            ("基础化工", "基础化工"),
            ("机器人概念", "机器人"),
            ("华为概念", "华为"),
            ("5G概念", "5G"),
            ("节能环保", "环保"),
            ("人工智能", "人工智能"),
            ("半导体概念", "半导体"),
            ("国产芯片", "国产芯片"),
            ("汽车", "汽车"),
            ("电商概念", "电商"),
            ("电子", "电子"),
            ("电网概念", "电网"),
            ("第三代半导体", "第三代半导体"),
            ("互联网金融", "互联网金融"),
            ("光伏概念", "光伏"),
            ("商贸零售", "商贸零售"),
            ("建筑装饰", "建筑装饰"),
            ("机械设备", "机械设备"),
            ("核能核电", "核电"),
            ("油气资源", "油气"),
            ("网红经济", "网红经济"),
            ("轻工制造", "轻工制造"),
            ("PCB", "PCB"),
            ("信创", "信创"),
            ("光刻机(胶)", "光刻机"),
            ("农业种植", "农业"),
            ("医药生物", "医药"),
            ("大数据", "大数据"),
            ("影视概念", "影视"),
            ("数据中心", "数据中心"),
            ("新能源", "新能源"),
            ("智慧城市", "智慧城市"),
            ("特斯拉概念", "特斯拉"),
            ("铁路基建", "铁路基建"),
        ] {
            let board_map = HashMap::from([(board.to_string(), "BK_TEST".to_string())]);
            assert_eq!(
                resolve_concept_alias(concept, &board_map).map(String::as_str),
                Some("BK_TEST"),
                "{concept}"
            );
        }
    }

    #[test]
    fn test_chain_score_position_cap() {
        let high = ChainScore {
            logic_hardness: 85.0,
            sentiment_position: 85.0,
            fund_consensus: 85.0,
            chip_health: 85.0,
            falsify_prob: 10.0,
        };
        assert_eq!(high.position_cap(), "30%");

        let mid = ChainScore {
            logic_hardness: 65.0,
            sentiment_position: 65.0,
            fund_consensus: 65.0,
            chip_health: 65.0,
            falsify_prob: 35.0,
        };
        assert_eq!(mid.position_cap(), "15%");

        let low = ChainScore {
            logic_hardness: 45.0,
            sentiment_position: 45.0,
            fund_consensus: 45.0,
            chip_health: 45.0,
            falsify_prob: 55.0,
        };
        assert_eq!(low.position_cap(), "5%");

        let avoid = ChainScore {
            logic_hardness: 30.0,
            sentiment_position: 30.0,
            fund_consensus: 30.0,
            chip_health: 30.0,
            falsify_prob: 80.0,
        };
        assert_eq!(avoid.position_cap(), "0%");
    }

    #[test]
    fn test_build_report_with_scores() {
        let date = "2026-06-13";
        let stocks = vec![
            TopStock {
                code: "TEST_CODE_000001".into(),
                name: "测试A".into(),
                change_pct: 10.0,
                price: 10.0,
                ..Default::default()
            },
            TopStock {
                code: "TEST_CODE_000002".into(),
                name: "测试B".into(),
                change_pct: 9.5,
                price: 20.0,
                ..Default::default()
            },
        ];
        let cluster = ChainCluster {
            concept: "测试概念".into(),
            aliases: vec![],
            stocks: stocks.clone(),
            continuation_count: 2,
            streak_days: 1,
            candidates: vec![],
            score: Some(ChainScore {
                logic_hardness: 70.0,
                sentiment_position: 60.0,
                fund_consensus: 65.0,
                chip_health: 50.0,
                falsify_prob: 40.0,
            }),
            scenario: None,
        };
        let analysis = "【结论】阶段=高潮｜参与=谨慎｜候选=无\n【评分】产业逻辑=70/100｜情绪位置=60/100｜资金共识=65/100｜筹码健康=50/100｜证伪概率=40/100\n### 产业链图谱\n测试内容";
        let sections = vec![("测试概念".into(), Some(analysis.to_string()))];
        let concepts: HashMap<String, Vec<String>> = HashMap::new();
        let diags: Vec<PositionDiag> = vec![];

        let report = build_report(
            date,
            &stocks,
            &[cluster],
            &sections,
            &[],
            None,
            "",
            &concepts,
            &diags,
        );
        assert!(report.contains("一页纸决策摘要"));
        assert!(report.contains("62")); // total = 70*0.25+60*0.25+65*0.2+50*0.15+60*0.15 = 62
        assert!(report.contains("适度参与"));
        assert!(report.contains("评分概览"));
        assert!(!report.contains("【评分】")); // score line should be stripped from detail
    }

    #[test]
    fn test_build_report_tier3_hot_list() {
        let date = "2026-06-13";
        let stocks = vec![
            TopStock {
                code: "TEST_CODE_000001".into(),
                name: "小概念A".into(),
                change_pct: 10.0,
                price: 10.0,
                ..Default::default()
            },
            TopStock {
                code: "TEST_CODE_000002".into(),
                name: "小概念B".into(),
                change_pct: 9.0,
                price: 20.0,
                ..Default::default()
            },
            TopStock {
                code: "TEST_CODE_000003".into(),
                name: "小概念C".into(),
                change_pct: 8.0,
                price: 30.0,
                ..Default::default()
            },
        ];
        let cluster = ChainCluster {
            concept: "弱主线".into(),
            aliases: vec![],
            stocks: stocks.clone(),
            continuation_count: 0,
            streak_days: 1,
            candidates: vec![],
            score: None,
            scenario: None,
        };
        let sections = vec![("弱主线".into(), None::<String>)]; // no analysis = tier 3
        let concepts: HashMap<String, Vec<String>> = HashMap::new();
        let diags: Vec<PositionDiag> = vec![];

        let report = build_report(
            date,
            &stocks,
            &[cluster],
            &sections,
            &[],
            None,
            "",
            &concepts,
            &diags,
        );
        assert!(
            report.contains("其他热点速览"),
            "should have hot list for unanalyzed clusters"
        );
        assert!(report.contains("弱主线"));
        assert!(report.contains("小概念A"));
    }

    #[test]
    fn test_build_report_position_diag_with_scores() {
        let date = "2026-06-13";
        let stocks = vec![TopStock {
            code: "TEST_CODE_000001".into(),
            name: "测试持仓".into(),
            change_pct: 10.0,
            price: 10.0,
            ..Default::default()
        }];
        let cluster = ChainCluster {
            concept: "电池技术".into(),
            aliases: vec![],
            stocks: stocks.clone(),
            continuation_count: 1,
            streak_days: 2,
            candidates: vec![],
            score: Some(ChainScore {
                logic_hardness: 70.0,
                sentiment_position: 60.0,
                fund_consensus: 65.0,
                chip_health: 50.0,
                falsify_prob: 40.0,
            }),
            scenario: None,
        };
        let analysis = "【结论】阶段=发酵中｜参与=可关注｜候选=无\n【评分】产业逻辑=70/100｜情绪位置=60/100｜资金共识=65/100｜筹码健康=50/100｜证伪概率=40/100\n### 产业链图谱\n测试";
        let sections = vec![("电池技术".into(), Some(analysis.to_string()))];
        let concepts: HashMap<String, Vec<String>> = HashMap::new();
        let diags = vec![PositionDiag {
            code: "TEST_CODE_000001".into(),
            name: "测试持仓".into(),
            return_rate: Some(5.0),
            mainline: Some(("电池技术".into(), 2)),
            in_limit_pool: true,
        }];

        let report = build_report(
            date,
            &stocks,
            &[cluster],
            &sections,
            &[],
            None,
            "",
            &concepts,
            &diags,
        );
        assert!(report.contains("持仓主线诊断"));
        assert!(report.contains("测试持仓"));
        assert!(report.contains("62")); // total score from analysis text
        assert!(report.contains("2天"));
    }

    #[test]
    fn cluster_size_env_and_generic_board_contract_are_deterministic() {
        let _guard = CHAIN_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let previous = std::env::var("CHAIN_MIN_CLUSTER").ok();
        std::env::set_var("CHAIN_MIN_CLUSTER", "4");
        assert_eq!(min_cluster_size(), 4);
        std::env::set_var("CHAIN_MIN_CLUSTER", "bad");
        assert_eq!(min_cluster_size(), 3);
        if let Some(value) = previous {
            std::env::set_var("CHAIN_MIN_CLUSTER", value);
        } else {
            std::env::remove_var("CHAIN_MIN_CLUSTER");
        }

        assert!(is_generic_board("沪深300成份"));
        assert!(is_generic_board("广东板块"));
        assert!(!is_generic_board("固态电池"));
    }

    #[test]
    fn concept_clustering_filters_generic_tags_merges_aliases_and_keeps_isolated_stocks() {
        let stocks = vec![
            top("TEST_CODE_000001", "甲", 10.0),
            top("TEST_CODE_000002", "乙", 9.0),
            top("TEST_CODE_000003", "丙", 8.0),
            top("TEST_CODE_000004", "丁", 7.0),
            top("TEST_CODE_000005", "戊", 6.0),
        ];
        let concepts = HashMap::from([
            (
                "TEST_CODE_000001".into(),
                vec!["固态电池".into(), "电池技术".into(), "昨日涨停".into()],
            ),
            (
                "TEST_CODE_000002".into(),
                vec!["固态电池".into(), "电池技术".into()],
            ),
            (
                "TEST_CODE_000003".into(),
                vec!["固态电池".into(), "电池技术".into(), "储能".into()],
            ),
            (
                "TEST_CODE_000004".into(),
                vec!["储能".into(), "融资融券".into()],
            ),
            ("TEST_CODE_000005".into(), vec!["融资融券".into()]),
        ]);

        let (clusters, isolated) = cluster_by_concept(&stocks, &concepts, 2);

        assert_eq!(clusters.len(), 2);
        let battery = clusters
            .iter()
            .find(|cluster| cluster.concept == "固态电池" || cluster.concept == "电池技术")
            .expect("battery cluster");
        assert_eq!(battery.stocks.len(), 3);
        assert_eq!(battery.aliases.len(), 1);
        assert_eq!(battery.continuation_count, 1);
        assert_eq!(battery.stocks[0].code, "TEST_CODE_000001");
        assert_eq!(isolated.len(), 1);
        assert_eq!(isolated[0].code, "TEST_CODE_000005");
    }

    #[tokio::test]
    async fn empty_limit_up_batch_returns_explicit_empty_report_without_external_calls() {
        let report = run_chain_analysis(Vec::new(), Some("本地宏观上下文".into()))
            .await
            .expect("empty complete batch");
        assert!(report.contains("涨停池批次成功返回 0 只"));
    }

    #[tokio::test]
    async fn prefetched_macro_context_is_preserved_without_search() {
        let context = resolve_macro_news(Some("  TEST_CODE_真实宏观上下文  ".to_string())).await;
        assert_eq!(context, "  TEST_CODE_真实宏观上下文  ");
    }

    #[tokio::test]
    async fn resolved_chain_facts_persist_match_and_render_without_external_sources() {
        crate::database::DatabaseManager::init(None).expect("isolated test database");
        let stocks = vec![
            top("TEST_CODE_CHAIN_230001", "测试链甲", 10.0),
            top("TEST_CODE_CHAIN_230002", "测试链乙", 9.9),
            top("TEST_CODE_CHAIN_230003", "测试孤立", 9.8),
        ];
        let concepts = HashMap::from([
            (
                "TEST_CODE_CHAIN_230001".to_string(),
                vec!["TEST_CODE_固态电池".to_string(), "昨日涨停".to_string()],
            ),
            (
                "TEST_CODE_CHAIN_230002".to_string(),
                vec!["TEST_CODE_固态电池".to_string()],
            ),
            (
                "TEST_CODE_CHAIN_230003".to_string(),
                vec!["TEST_CODE_独立逻辑".to_string()],
            ),
        ]);
        let (mut clusters, isolated) = cluster_and_persist("2026-07-19", &stocks, &concepts, 2)
            .expect("complete parsed cluster facts");
        assert_eq!(clusters.len(), 1);
        assert_eq!(isolated.len(), 1);
        assert_eq!(clusters[0].streak_days, 1);

        let exact = HashMap::from([(
            "TEST_CODE_固态电池".to_string(),
            "BK_TEST_230001".to_string(),
        )]);
        assert_eq!(
            resolve_cluster_board_code(&clusters[0], &exact).expect("exact board match"),
            "BK_TEST_230001"
        );
        let substring = HashMap::from([(
            "TEST_CODE_固态电池概念".to_string(),
            "BK_TEST_230002".to_string(),
        )]);
        assert_eq!(
            resolve_cluster_board_code(&clusters[0], &substring).expect("substring board match"),
            "BK_TEST_230002"
        );
        assert!(resolve_cluster_board_code(&clusters[0], &HashMap::new()).is_err());

        clusters[0].candidates = vec![top("TEST_CODE_CHAIN_230004", "测试补涨候选", 3.0)];
        assert_eq!(clusters[0].candidates.len(), 1);
        let positions = vec![PositionDiag {
            code: "TEST_CODE_CHAIN_230001".to_string(),
            name: "测试链甲".to_string(),
            return_rate: Some(1.5),
            mainline: Some(("TEST_CODE_固态电池".to_string(), 1)),
            in_limit_pool: true,
        }];
        let report = render_resolved_chain_analysis(
            &unavailable_analyzer(),
            "2026-07-19",
            &stocks,
            &concepts,
            &clusters,
            &isolated,
            &positions,
            &HashMap::from([("TEST_CODE_CHAIN_230001".to_string(), 100.0)]),
            "TEST_CODE_真实宏观事实",
            ChainEvidenceSource::Live,
        )
        .await
        .expect("resolved report without model");
        assert!(report.contains("TEST_CODE_固态电池"));
        assert!(report.contains("测试链甲"));
        assert!(report.contains("持仓主线诊断"));
        assert!(report.contains("其他热点速览"));
    }

    #[tokio::test]
    async fn resolved_evidence_commits_deep_simple_and_overview_model_protocols() {
        fn cluster(concept: &str, base: usize, count: usize) -> ChainCluster {
            ChainCluster {
                concept: concept.to_string(),
                aliases: vec![format!("{concept}别名")],
                stocks: (0..count)
                    .map(|offset| {
                        top(
                            &format!("TEST_CODE_CHAIN_MODEL_{:06}", base + offset),
                            &format!("协议股{offset}"),
                            10.0 - offset as f64 / 10.0,
                        )
                    })
                    .collect(),
                continuation_count: 1,
                streak_days: 2,
                candidates: vec![top(
                    &format!("TEST_CODE_CHAIN_MODEL_{:06}", base + count),
                    "协议候选",
                    3.0,
                )],
                score: None,
                scenario: None,
            }
        }

        let deep = cluster("TEST_CODE_深度主线", 100, TIER1_MIN);
        let simple = cluster("TEST_CODE_简化主线", 200, TIER2_MIN);
        let limit_ups: Vec<TopStock> = deep
            .stocks
            .iter()
            .chain(simple.stocks.iter())
            .cloned()
            .collect();
        let concepts: HashMap<String, Vec<String>> = limit_ups
            .iter()
            .map(|stock| {
                let concept = if stock.code.contains("0001") {
                    deep.concept.clone()
                } else {
                    simple.concept.clone()
                };
                (stock.code.clone(), vec![concept, "TEST_CODE_设备".into()])
            })
            .collect();
        let server = crate::data_provider::TestHttpServer::new(vec![
            crate::data_provider::TestHttpResponse::json(
                r####"{"choices":[{"message":{"content":"【结论】阶段=启动｜参与=可关注｜候选=TEST_CODE_CHAIN_MODEL_000108\n【评分】产业逻辑=80/100｜情绪位置=70/100｜资金共识=60/100｜筹码健康=50/100｜证伪概率=20/100\n### 产业链图谱\n协议深度正文"}}]}"####,
            ),
            crate::data_provider::TestHttpResponse::json(
                r####"{"choices":[{"message":{"content":"【简评】阶段=发酵｜参与=谨慎｜候选=无\n【评分】产业逻辑=60/100｜情绪位置=50/100｜资金共识=40/100｜筹码健康=30/100｜证伪概率=50/100\n协议简化正文"}}]}"####,
            ),
            crate::data_provider::TestHttpResponse::json(
                r####"{"choices":[{"message":{"content":"### 核心矛盾与主线优先级\nTEST_CODE_全景协议正文"}}]}"####,
            ),
        ]);
        let analyzer = GeminiAnalyzer::with_loopback_client(crate::analyzer::GeminiConfig {
            doubao_api_key: Some("TEST_CODE_LOCAL_PROTOCOL_KEY".into()),
            doubao_base_url: Some(server.base_url().to_string()),
            doubao_model: "TEST_CODE_MODEL".into(),
            max_retries: 1,
            retry_delay: 0.0,
            request_delay: 0.0,
            agent_pipeline: false,
            ..crate::analyzer::GeminiConfig::default()
        });
        let evidence = ResolvedChainEvidence {
            cluster_news: HashMap::from([(
                deep.concept.clone(),
                "TEST_CODE_已验证主线新闻".to_string(),
            )]),
            after_market: "## 盘后催化\nTEST_CODE_已验证盘后证据".to_string(),
        };

        let report = render_resolved_chain_analysis(
            &analyzer,
            "2026-07-19",
            &limit_ups,
            &concepts,
            &[deep, simple],
            &[],
            &[],
            &HashMap::new(),
            "TEST_CODE_已验证宏观事实",
            ChainEvidenceSource::Resolved(evidence),
        )
        .await
        .expect("complete resolved model protocol must commit report");

        assert!(report.contains("深度分析 1 条 + 简化分析 1 条"));
        assert!(report.contains("协议深度正文"));
        assert!(report.contains("协议简化正文"));
        assert!(report.contains("TEST_CODE_已验证盘后证据"));
        assert!(report.contains("TEST_CODE_全景协议正文"));
        let requests = server.finish();
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|path| path == "/chat/completions"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn position_diagnosis_uses_complete_sqlite_positions_and_cached_concepts() {
        crate::database::DatabaseManager::init(None).expect("isolated test database");
        let db = crate::database::DatabaseManager::get();
        let suffix = format!(
            "{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        );
        let member = format!("TEST_CODE_CHAIN_MEMBER_{suffix}");
        let alias = format!("TEST_CODE_CHAIN_ALIAS_{suffix}");
        let unrelated = format!("TEST_CODE_CHAIN_OTHER_{suffix}");
        let codes = [&member, &alias, &unrelated];

        let mut conn = db.get_conn().expect("test database connection");
        // Previous isolated tests may deliberately leave a malformed cache row. It is
        // not part of this complete input batch and must not trigger a real fetch here.
        diesel::sql_query("DELETE FROM stock_concepts WHERE json_valid(concepts) = 0")
            .execute(&mut conn)
            .expect("remove deliberately malformed isolated fixture rows");
        for existing in db
            .get_all_open_positions()
            .expect("existing isolated positions")
        {
            db.save_stock_concepts(&existing.code, &[format!("TEST_CODE_UNRELATED_{suffix}")])
                .expect("complete cache for isolated existing position");
        }
        for (index, code) in codes.iter().enumerate() {
            diesel::sql_query(
                "INSERT INTO stock_position
                 (code, name, buy_date, buy_price, quantity, status, chain_name)
                 VALUES (?, ?, '2026-07-17', 10.0, 100, 'open', NULL)",
            )
            .bind::<diesel::sql_types::Text, _>(*code)
            .bind::<diesel::sql_types::Text, _>(format!("TEST_CODE_持仓{index}"))
            .execute(&mut conn)
            .expect("insert isolated position evidence");
        }
        drop(conn);

        db.save_stock_concepts(&member, &["TEST_CODE_主线".to_string()])
            .expect("member concept cache");
        db.save_stock_concepts(&alias, &["TEST_CODE_别名".to_string()])
            .expect("alias concept cache");
        db.save_stock_concepts(&unrelated, &["TEST_CODE_无关".to_string()])
            .expect("unrelated concept cache");

        let cluster = ChainCluster {
            concept: "TEST_CODE_主线".to_string(),
            aliases: vec!["TEST_CODE_别名".to_string()],
            stocks: vec![top(&member, "TEST_CODE_成员", 10.0)],
            continuation_count: 0,
            streak_days: 3,
            candidates: Vec::new(),
            score: None,
            scenario: None,
        };
        let diagnosed = diagnose_positions(&[cluster])
            .await
            .expect("complete cached position diagnosis");
        let member_diag = diagnosed
            .iter()
            .find(|item| item.code == member)
            .expect("member diagnosis");
        assert!(member_diag.in_limit_pool);
        assert_eq!(
            member_diag.mainline,
            Some(("TEST_CODE_主线".to_string(), 3))
        );
        let alias_diag = diagnosed
            .iter()
            .find(|item| item.code == alias)
            .expect("alias diagnosis");
        assert!(!alias_diag.in_limit_pool);
        assert_eq!(alias_diag.mainline, Some(("TEST_CODE_主线".to_string(), 3)));
        let other_diag = diagnosed
            .iter()
            .find(|item| item.code == unrelated)
            .expect("unrelated diagnosis");
        assert_eq!(other_diag.mainline, None);

        let mut conn = db.get_conn().expect("cleanup database connection");
        for code in codes {
            diesel::sql_query("DELETE FROM stock_position WHERE code = ?")
                .bind::<diesel::sql_types::Text, _>(code)
                .execute(&mut conn)
                .expect("cleanup isolated position");
            diesel::sql_query("DELETE FROM stock_concepts WHERE code = ?")
                .bind::<diesel::sql_types::Text, _>(code)
                .execute(&mut conn)
                .expect("cleanup isolated concept cache");
        }
    }

    #[test]
    fn malformed_score_and_conclusion_protocols_remain_unavailable() {
        assert!(parse_chain_score("无评分").is_none());
        assert!(parse_chain_score("【评分】产业逻辑=bad/100").is_none());
        assert!(parse_conclusion("无结论").is_none());
        assert_eq!(
            parse_conclusion("【结论】阶段=启动｜参与=谨慎"),
            Some(("启动".into(), "谨慎".into(), "-".into()))
        );
    }

    #[test]
    fn report_covers_catalyst_overview_alias_isolated_and_unmapped_position_branches() {
        let clustered = top("TEST_CODE_000001", "主线股", 10.0);
        let isolated = top("TEST_CODE_000002", "孤立股", 9.0);
        let cluster = ChainCluster {
            concept: "固态电池".into(),
            aliases: vec!["电池技术".into()],
            stocks: vec![clustered.clone()],
            continuation_count: 0,
            streak_days: 0,
            candidates: vec![],
            score: None,
            scenario: None,
        };
        let sections = vec![(
            "固态电池".into(),
            Some("【简评】阶段=启动｜参与=谨慎｜候选=无\n正文".into()),
        )];
        let concepts = HashMap::from([(
            isolated.code.clone(),
            vec!["融资融券".into(), "独立逻辑".into()],
        )]);
        let positions = vec![PositionDiag {
            code: "TEST_CODE_000009".into(),
            name: "未映射持仓".into(),
            return_rate: None,
            mainline: None,
            in_limit_pool: false,
        }];

        let report = build_report(
            "2026-07-18",
            &[clustered, isolated.clone()],
            &[cluster],
            &sections,
            &[isolated],
            Some("# 今日主线全景研判\n\n全景正文"),
            "## 盘后催化\n真实催化",
            &concepts,
            &positions,
        );

        assert!(report.contains("盘后催化"));
        assert!(report.contains("全景正文"));
        assert!(report.contains("同义概念：电池技术"));
        assert!(report.contains("无（不在今日主线）"));
        assert!(report.contains("暂无"));
        assert!(report.contains("孤立涨停"));
        assert!(report.contains("独立逻辑"));
        assert!(!report.contains("融资融券、独立逻辑"));
    }

    #[tokio::test]
    async fn llm_prompt_paths_preserve_evidence_and_fail_closed_without_a_provider() {
        let analyzer = unavailable_analyzer();
        assert!(!analyzer.is_available());
        let concepts = HashMap::from([
            (
                "TEST_CODE_000001".to_string(),
                vec!["测试主线".to_string(), "相关设备".to_string()],
            ),
            (
                "TEST_CODE_000002".to_string(),
                vec!["测试主线".to_string(), "昨日涨停".to_string()],
            ),
        ]);
        let cluster = ChainCluster {
            concept: "测试主线".to_string(),
            aliases: vec!["测试别名".to_string()],
            stocks: vec![
                top("TEST_CODE_000001", "甲", 10.0),
                top("TEST_CODE_000002", "乙", 9.8),
            ],
            continuation_count: 1,
            streak_days: 2,
            candidates: vec![top("TEST_CODE_000003", "候选", 3.0)],
            score: None,
            scenario: None,
        };
        let lhb = HashMap::from([("TEST_CODE_000001".to_string(), 1234.0)]);

        let deep_error = analyze_cluster_deep(
            &analyzer,
            &cluster,
            &concepts,
            &lhb,
            "真实宏观上下文",
            "真实主线新闻",
            "2026-07-18",
        )
        .await
        .expect_err("missing provider must be explicit");
        assert!(deep_error.to_string().contains("API Key 未配置"));

        let simple_error =
            analyze_cluster_simple(&analyzer, &cluster, &concepts, &lhb, "2026-07-18")
                .await
                .expect_err("missing provider must be explicit");
        assert!(simple_error.to_string().contains("API Key 未配置"));

        let positions = vec![PositionDiag {
            code: "TEST_CODE_000001".to_string(),
            name: "甲".to_string(),
            return_rate: None,
            mainline: Some(("测试主线".to_string(), 2)),
            in_limit_pool: true,
        }];
        assert!(synthesize_overview(
            &analyzer,
            &[cluster],
            &[("测试主线".to_string(), Some("已验证分析".to_string()))],
            &positions,
            "2026-07-18",
            "盘后真实催化",
        )
        .await
        .is_none());
    }

    #[tokio::test]
    async fn llm_prompt_empty_optional_evidence_stays_explicitly_missing() {
        let analyzer = unavailable_analyzer();
        let cluster = ChainCluster {
            concept: "测试主线".to_string(),
            aliases: Vec::new(),
            stocks: vec![top("TEST_CODE_000001", "甲", 10.0)],
            continuation_count: 0,
            streak_days: 0,
            candidates: Vec::new(),
            score: None,
            scenario: None,
        };
        assert!(analyze_cluster_deep(
            &analyzer,
            &cluster,
            &HashMap::new(),
            &HashMap::new(),
            "",
            "",
            "2026-07-18",
        )
        .await
        .is_err());
        assert!(analyze_cluster_simple(
            &analyzer,
            &cluster,
            &HashMap::new(),
            &HashMap::new(),
            "2026-07-18",
        )
        .await
        .is_err());
        assert!(synthesize_overview(
            &analyzer,
            &[cluster],
            &[("测试主线".to_string(), None)],
            &[],
            "2026-07-18",
            "",
        )
        .await
        .is_none());
    }
}

#[cfg(test)]
#[path = "../../gate_d_chain_analysis_regression.rs"]
mod gate_d_regression;
