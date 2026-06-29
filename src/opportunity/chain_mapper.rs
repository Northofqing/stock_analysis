//! 新闻 → 产业链映射。
//!
//! 关键词规则表（优先）+ AI 推理兜底（规则未命中时）+ 动态板块成份股解析。
//! 不写死标的，标的从 sector_monitor 实时拉。

use crate::market_analyzer::sector_monitor;

/// 板块候选池大小：兼顾覆盖率与请求成本。
const BOARD_RANK_TOP_N: usize = 200;
/// 成份股抓取上限：避免只拿到高成交额头部导致持仓漏判。
const COMPONENT_FETCH_TOP_N: usize = 100;
/// 最终保留的板块成份股数量。
const COMPONENT_KEEP_TOP_N: usize = 50;

#[derive(Debug, Clone)]
pub struct StockInfo {
    pub code: String,
    pub name: String,
    /// 当日涨跌幅 (%)：用于低位卡位/追高风险判定
    pub change_pct: f64,
    /// 量比：>1 表示今日放量，资金开始关注
    pub vol_ratio: f64,
}

/// 产业链命中来源，用于可观测性与降级标记。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChainSource {
    /// 关键词规则表命中
    #[default]
    Rule,
    /// 规则未命中，AI 推理产出
    Ai,
    /// 规则未命中且 AI 不可用（降级，不编造产业链）
    AiDegraded,
}

#[derive(Debug, Clone)]
pub struct ChainHit {
    pub chain: String,
    pub keywords: Vec<String>,
    pub logic: String,
    pub stocks: Vec<StockInfo>,
    /// 命中来源（规则 / AI / AI降级）
    pub source: ChainSource,
    /// 用于动态匹配板块代码的板块名关键词；AI 来源时由 AI 提供，规则来源可留空（从规则表查）
    pub board_keyword: String,
    /// 匹配板块的今日主力净占比(%)；None = 资金数据不可用（不臆测多空）
    pub fund_flow_pct: Option<f64>,
}

/// 产业链规则唯一来源：config/chain_rules.toml
///
/// 运行时优先读磁盘配置（支持热更新）；当文件缺失或解析失败时，
/// 回退到编译期内嵌的同一份 toml 文本，避免代码中维护第二份规则。
const DEFAULT_CHAIN_RULES_TOML: &str = include_str!("../../config/chain_rules.toml");

fn normalize_chain_rules(mut rules: Vec<(Vec<String>, String, String, String, u32, bool)>) -> Vec<(Vec<String>, String, String, String, u32, bool)> {
    rules.sort_by(|a, b| b.4.cmp(&a.4));
    rules
}

fn map_chain_rules(config_rules: Vec<crate::config::ChainRuleConfig>) -> Vec<(Vec<String>, String, String, String, u32, bool)> {
    config_rules
        .into_iter()
        .map(|r| (r.keywords, r.chain, r.logic, r.board_keyword, r.priority, r.generic))
        .collect()
}

fn parse_chain_rules_toml(toml_text: &str) -> Option<Vec<(Vec<String>, String, String, String, u32, bool)>> {
    let file_cfg = toml::from_str::<crate::config::ChainRulesFile>(toml_text).ok()?;
    Some(normalize_chain_rules(map_chain_rules(file_cfg.rules)))
}

/// 加载规则：优先 toml，不可用则回退编译期内嵌 toml。按 priority 降序返回。
fn chain_rules() -> Vec<(Vec<String>, String, String, String, u32, bool)> {
    // 1) 优先使用已热加载进内存的配置（通常由 config::load_all 填充）
    if let Some(config_rules) = crate::config::get_chain_rules() {
        return normalize_chain_rules(map_chain_rules(config_rules));
    }

    // 2) 若内存缓存为空，直接读取 config/chain_rules.toml，避免与文件配置脱节。
    if let Ok(s) = std::fs::read_to_string("config/chain_rules.toml") {
        if let Some(rules) = parse_chain_rules_toml(&s) {
            return rules;
        }
        log::warn!("[ChainMapper] 读取 config/chain_rules.toml 成功但解析失败，回退编译期内嵌规则");
    }

    // 3) 最后回退到编译期内嵌同源 toml，保障可用性。
    if let Some(rules) = parse_chain_rules_toml(DEFAULT_CHAIN_RULES_TOML) {
        return rules;
    }

    log::error!("[ChainMapper] 编译期内嵌规则解析失败，返回空规则集");
    Vec::new()
}

/// 从新闻标题中匹配产业链（按 priority 降序遍历，高优先级规则先匹配）
///
/// 修复 v9.2 BR-002: 一条快讯最多 1 条产业链（例外: AI 给出 ≥2 条独立产业链）
pub fn map_news_to_chains(title: &str) -> Vec<ChainHit> {
    let mut hits: Vec<ChainHit> = Vec::new();
    let rules = chain_rules();

    for (keywords, chain, logic, board_keyword, _priority, _generic) in &rules {
        let matched: Vec<&str> = keywords.iter()
            .filter(|kw| title.contains(kw.as_str()))
            .map(|s| s.as_str())
            .collect();
        if matched.is_empty() { continue; }
        if hits.iter().any(|h| h.chain == *chain) { continue; }

        // BR-002 互斥: 只保留第 1 条命中 (按 priority 降序遍历, 优先级最高先匹配)
        // 注: 不再允许"一条快讯命中 N 条产业链"除非 AI 显式给出多条独立逻辑
        if !hits.is_empty() {
            log::debug!("[ChainMapper] 互斥: {} 已命中, 跳过 {} (BR-002)", hits[0].chain, chain);
            continue;
        }

        hits.push(ChainHit {
            chain: chain.clone(),
            keywords: matched.iter().map(|s| s.to_string()).collect(),
            logic: logic.clone(),
            stocks: Vec::new(),
            source: ChainSource::Rule,
            board_keyword: board_keyword.clone(),
            fund_flow_pct: None,
        });
    }
    hits
}

/// 新闻 → 产业链（规则优先，未命中则 AI 兜底）。
///
/// 决策（v8）：仅在关键词规则未命中时才调 AI，节省 token。
/// v9 改进：规则命中结果过于单一时（只有1条且来自通用规则），也调 AI 二次分类。
/// 数据红线 2.1/2.2：AI 不可用 → 返回空，**不编造产业链**。
pub async fn map_news_to_chains_ai(titles: &[String]) -> Vec<ChainHit> {
    let combined = titles.join(" ");
    let rule_hits = map_news_to_chains(&combined);
    let rules = chain_rules();
    
    // 规则命中结果过于单一（仅命中 generic=true 的规则）时，调 AI 二次分类。
    let should_call_ai = rule_hits.len() == 1 && {
        let chain_name = &rule_hits[0].chain;
        rules.iter().any(|(_, chain, _, _, _, generic)| chain == chain_name && *generic)
    };
    
    if !rule_hits.is_empty() && !should_call_ai {
        return rule_hits; // 规则命中且不需要二次分类
    }
    
    // 规则命中过于单一或完全未命中 → AI 兜底分类
    if should_call_ai {
        log::info!(
            "[ChainMapper] 规则命中1条通用规则({}) + {} 条新闻 → 调 AI 二次分类验证多样性",
            rule_hits[0].chain, titles.len()
        );
    } else if rule_hits.is_empty() {
        log::info!("[ChainMapper] 规则未命中({} 条新闻) → 调 AI 兜底", titles.len());
    }

    // 规则未命中或需要二次分类 → AI 兜底。
    // GeminiAnalyzer 含 RefCell（非 Sync），跨 await 会破坏外层 Future 的 Send，
    // 故隔离在独立 blocking 线程的 current-thread 运行时内执行。
    let titles_owned = titles.to_vec();
    let existing_chain = rule_hits.first().map(|h| h.chain.clone());
    tokio::task::spawn_blocking(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(_) => return Vec::new(),
        };
        rt.block_on(async move {
            let analyzer = crate::analyzer::GeminiAnalyzer::from_env();
            if !analyzer.is_available() {
                log::warn!("[ChainMapper] 需要 AI 二次分类但 AI 不可用 → [AI降级]");
                return rule_hits; // 降级时保留规则命中结果
            }
            
            let prompt = if let Some(existing_chain) = existing_chain {
                format!(
                    "你是A股产业链分析师。已规则命中：【{}】。现需要验证是否有其他**不同类型**的产业链催化。\n\n<快讯>\n{}\n</快讯>\n\n要求：\n1. 如果新闻主要确实就是【{}】，输出\"无其他产业链\"\n2. 如果存在其他明显的产业链/概念催化（不同于{}），每条一行，最多3条\n3. 格式：产业链名|催化逻辑(20字内)|板块名关键词\n4. 只输出真实有逻辑的，宁缺毋滥",
                    existing_chain, titles_owned.join("\n"), existing_chain, existing_chain
                )
            } else {
                format!(
                    "你是A股产业链分析师。下面是最新快讯，请抽取其中**确有催化的产业链/概念**（没有则输出\"无\"）。\n\n<快讯>\n{}\n</快讯>\n\n要求：\n1. 最多输出3条，每条一行\n2. 格式：产业链名|催化逻辑(20字内)|板块名关键词\n3. 板块名关键词须是东方财富概念板块常见名(如 PCB、半导体、光伏、机器人)\n4. 只输出真实有逻辑的，宁缺毋滥",
                    titles_owned.join("\n")
                )
            };
            
            match analyzer
                .call_api_mode(&prompt, "你是A股产业链分析师,只输出格式化结果", crate::analyzer::AgentMode::Quick)
                .await
            {
                Ok(t) => {
                    let ai_hits = parse_ai_chains(&t);
                    if ai_hits.is_empty() {
                        log::info!("[ChainMapper] AI 未发现新产业链，保留规则命中结果");
                        rule_hits
                    } else {
                        log::info!("[ChainMapper] AI 发现 {} 条新产业链，合并规则结果", ai_hits.len());
                        // 合并规则命中和 AI 结果，去重
                        let mut merged = rule_hits;
                        for ai_hit in ai_hits {
                            if !merged.iter().any(|h| h.chain == ai_hit.chain) {
                                merged.push(ai_hit);
                            }
                        }
                        merged
                    }
                },
                Err(e) => {
                    log::warn!("[ChainMapper] AI 调用失败: {} → [AI降级]", e);
                    rule_hits
                }
            }
        })
    })
    .await
    .unwrap_or_default()
}

/// 解析 AI 产出的产业链文本。格式：产业链名|催化逻辑|板块名关键词
fn parse_ai_chains(text: &str) -> Vec<ChainHit> {
    let mut hits: Vec<ChainHit> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line == "无" || line.starts_with('#') { continue; }
        let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
        if parts.len() < 3 { continue; }
        let chain = parts[0].to_string();
        let logic = parts[1].to_string();
        let board_keyword = parts[2].to_string();
        if chain.is_empty() || board_keyword.is_empty() { continue; }
        if hits.iter().any(|h| h.chain == chain) { continue; }
        hits.push(ChainHit {
            chain,
            keywords: vec![board_keyword.clone()],
            logic,
            stocks: Vec::new(),
            source: ChainSource::Ai,
            board_keyword,
            fund_flow_pct: None,
        });
        if hits.len() >= 3 { break; }
    }
    hits
}

/// 为 ChainHit 动态解析标的。
/// 先拉取全部板块排名，按板块名关键词匹配板块代码，再拉成份股。
pub fn resolve_stocks(hits: &mut [ChainHit]) {
    // 一次性拉两路榜单并并集，避免只看涨幅榜导致板块池过窄。
    let mut board_map: std::collections::HashMap<String, (String, f64)> = std::collections::HashMap::new();
    if let Ok(boards) = sector_monitor::fetch_board_ranking("f3", BOARD_RANK_TOP_N) {
        for b in boards {
            board_map.entry(b.name).or_insert((b.code, b.main_net_pct_today));
        }
    }
    if let Ok(boards) = sector_monitor::fetch_board_ranking("f62", BOARD_RANK_TOP_N) {
        for b in boards {
            board_map.entry(b.name).or_insert((b.code, b.main_net_pct_today));
        }
    }
    if board_map.is_empty() {
        return;
    }

    let mut failed_chains: Vec<String> = Vec::new();
    
    for hit in hits.iter_mut() {
        // 优先用 hit 自带的 board_keyword（Rule 来源已直接存储，AI 来源亦然）
        let mut board_keyword = hit.board_keyword.clone();
        // 安全兜底：若 board_keyword 为空，从规则表回溯查找
        if board_keyword.is_empty() {
            let rules = chain_rules();
            board_keyword = rules.iter()
                .find(|(_, chain, _, _, _, _)| chain == &hit.chain)
                .map(|(_, _, _, kw, _, _)| kw.clone())
                .unwrap_or_default();
        }

        // 空关键词会匹配任意板块，跳过以免错拉
        if board_keyword.is_empty() {
            failed_chains.push(format!("  • {} — 无板块关键词", hit.chain));
            continue;
        }

        // 动态匹配板块代码：先在热点板块池匹配；未命中时回退到 suggest 全量检索。
        let matched_board = find_best_board_match(&board_map, &board_keyword);

        let (code, flow_pct_opt) = match matched_board {
            Some((c, flow)) => (c, Some(flow)),
            None => match sector_monitor::search_board_code_by_keyword(&board_keyword) {
                Ok(Some((fallback_code, fallback_name))) => {
                    log::debug!(
                        "[ChainMapper] 关键词'{}'未命中热点板块池，使用 suggest 回退命中板块 {}({})",
                        board_keyword,
                        fallback_name,
                        fallback_code
                    );
                    (fallback_code, None)
                }
                Ok(None) => {
                    failed_chains.push(format!("  • {} — 未找到板块关键词'{}'的对应板块", hit.chain, board_keyword));
                    continue;
                }
                Err(e) => {
                    failed_chains.push(format!("  • {} — 板块关键词'{}' suggest 查询失败: {}", hit.chain, board_keyword, e));
                    continue;
                }
            },
        };

        // 资金流向数值校验（数据红线 2.3）：净占比应在合理区间，异常值视为不可用
        hit.fund_flow_pct = flow_pct_opt.and_then(|flow_pct| {
            if flow_pct.is_finite() && flow_pct.abs() <= 100.0 {
                Some(flow_pct)
            } else {
                None
            }
        });

        match sector_monitor::fetch_board_components(&code, COMPONENT_FETCH_TOP_N) {
            Ok(stocks) => {
                let filtered: Vec<_> = stocks.into_iter()
                    .filter(|s| !s.code.starts_with('8') && !s.code.starts_with('4'))
                    .filter(|s| !s.code.starts_with("688"))
                    .take(COMPONENT_KEEP_TOP_N)
                    .map(|s| StockInfo { code: s.code, name: s.name, change_pct: s.change_pct, vol_ratio: s.vol_ratio })
                    .collect();
                
                if filtered.is_empty() {
                    failed_chains.push(format!("  • {} — 板块'{}' 无有效成分股（过滤北交所/科创板）", hit.chain, board_keyword));
                } else {
                    hit.stocks = filtered;
                }
            }
            Err(e) => {
                failed_chains.push(format!("  • {} — 板块'{}' 拉取失败: {}", hit.chain, board_keyword, e));
            }
        }
    }
    
    if !failed_chains.is_empty() {
        log::debug!("[ChainMapper] 产业链标的解析：{} 条产业链失败\n{}", failed_chains.len(), failed_chains.join("\n"));
    }
}

fn normalize_board_text(text: &str) -> String {
    text.to_lowercase()
        .replace(' ', "")
        .replace('-', "")
        .replace('_', "")
        .replace('/', "")
        .replace('（', "")
        .replace('）', "")
        .replace('(', "")
        .replace(')', "")
}

fn matches_keyword_synonym(board_name: &str, keyword: &str) -> bool {
    // 检查是否与关键词的任何同义词匹配
    let name_norm = normalize_board_text(board_name);
    
    let synonyms = match keyword {
        // AI 硬件相关
        "PCB" => vec!["pcb", "电子元器件", "元器件", "电路板", "印制电路板"],
        // 半导体设备相关
        "光刻机" => vec!["光刻机", "半导体", "芯片", "集成电路", "半导体设备"],
        // 稀有金属相关
        "小金属" => vec!["小金属", "稀土", "稀有金属", "磁材", "材料"],
        // 新能源相关
        "光伏" => vec!["光伏", "太阳能", "新能源"],
        "锂电" => vec!["锂电", "动力电池", "电池"],
        _ => return false,
    };
    
    for synonym in synonyms {
        let syn_norm = normalize_board_text(synonym);
        if board_name == synonym || name_norm == syn_norm || 
           board_name.contains(synonym) || name_norm.contains(&syn_norm) {
            return true;
        }
    }
    false
}

fn board_match_score(board_name: &str, keyword: &str) -> Option<u8> {
    if keyword.is_empty() {
        return None;
    }
    if board_name == keyword {
        return Some(4);
    }

    let name_norm = normalize_board_text(board_name);
    let key_norm = normalize_board_text(keyword);
    if key_norm.is_empty() {
        return None;
    }
    if name_norm == key_norm {
        return Some(4);
    }
    if board_name.starts_with(keyword) || name_norm.starts_with(&key_norm) {
        return Some(3);
    }
    if board_name.contains(keyword) || name_norm.contains(&key_norm) {
        return Some(2);
    }
    if keyword.contains(board_name) || key_norm.contains(&name_norm) {
        return Some(1);
    }

    // 如果直接匹配失败，尝试同义词匹配
    if matches_keyword_synonym(board_name, keyword) {
        return Some(2); // 同义词匹配得分为 2
    }

    None
}

fn find_best_board_match(
    board_map: &std::collections::HashMap<String, (String, f64)>,
    keyword: &str,
) -> Option<(String, f64)> {
    board_map
        .iter()
        .filter_map(|(name, v)| board_match_score(name, keyword).map(|score| (score, name.len(), v.clone())))
        .max_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| b.1.cmp(&a.1))
        })
        .map(|(_, _, v)| v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pcb_news() {
        let hits = map_news_to_chains("电子布年内第五轮提价，木林森PCB产品全线涨价20%");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chain, "AI硬件-PCB");
    }

    #[test]
    fn test_multi_chain_news() {
        // 修复 v9.2 BR-002: 一条快讯最多 1 条产业链, 只保留 priority 最高那条
        // 标题同时含 MLCC + PCB, 修复后只保留优先级最高的一条
        let hits = map_news_to_chains("MLCC突破带动PCB和半导体产业链全线走强");
        assert!(hits.len() <= 1, "BR-002: 一条快讯最多 1 条产业链, 实际 {} 条", hits.len());
        // 至少命中一条 (按 priority 排序第一条)
        assert!(!hits.is_empty(), "应至少命中 1 条");
    }

    #[test]
    fn test_no_match() {
        let hits = map_news_to_chains("今日天气晴朗适合出游");
        assert!(hits.is_empty());
    }

    #[test]
    fn test_city_renewal() {
        let hits = map_news_to_chains("国务院通过城市更新十五五规划，地下管网改造加速");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chain, "城市更新");
    }

    #[test]
    fn test_rule_hit_marks_source_rule() {
        let hits = map_news_to_chains("电子布提价带动PCB涨价");
        assert_eq!(hits[0].source, ChainSource::Rule);
    }

    #[test]
    fn test_parse_ai_chains_ok() {
        let text = "固态电池|技术迭代催化|固态电池\n机器人|人形量产提速|机器人\n无效行";
        let hits = parse_ai_chains(text);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].chain, "固态电池");
        assert_eq!(hits[0].source, ChainSource::Ai);
        assert_eq!(hits[0].board_keyword, "固态电池");
        assert!(hits.iter().all(|h| h.source == ChainSource::Ai));
    }

    #[test]
    fn test_parse_ai_chains_empty_and_no() {
        assert!(parse_ai_chains("无").is_empty());
        assert!(parse_ai_chains("").is_empty());
        // 板块关键词为空 → 丢弃，不伪造
        assert!(parse_ai_chains("某链|某逻辑|").is_empty());
    }

    #[tokio::test]
    async fn test_ai_fallback_skips_ai_when_rule_hits() {
        // 规则命中时直接返回规则结果，不触发 AI（即使 AI 不可用也能产出）
        let hits = map_news_to_chains_ai(&["PCB全线涨价20%".to_string()]).await;
        assert!(!hits.is_empty());
        assert_eq!(hits[0].source, ChainSource::Rule);
    }

    #[test]
    fn test_hbm_news() {
        let hits = map_news_to_chains("SK海力士HBM3E量产供货英伟达，高带宽内存需求爆发");
        assert!(hits.iter().any(|h| h.chain == "HBM-高带宽内存"));
    }

    #[test]
    fn test_commercial_aerospace() {
        let hits = map_news_to_chains("千帆星座第二批卫星成功发射，商业航天低轨组网加速");
        assert!(hits.iter().any(|h| h.chain == "商业航天"));
    }

    #[test]
    fn test_solid_state_battery_separate_from_lithium() {
        // 固态电池应命中独立规则，而非笼统的 新能源-锂电池
        let hits = map_news_to_chains("丰田宣布硫化物固态电池2027年量产装车");
        assert!(hits.iter().any(|h| h.chain == "新能源-固态电池"));
    }

    #[test]
    fn test_smart_driving_news() {
        let hits = map_news_to_chains("特斯拉FSD入华获批，端到端智驾加速落地");
        assert!(hits.iter().any(|h| h.chain == "智能驾驶"));
    }

    #[test]
    fn test_board_keyword_stored_for_rule_hits() {
        // v2: rule 来源的 hit 应直接携带 board_keyword，不再依赖 resolve 阶段二次查表
        let hits = map_news_to_chains("CPO光模块出货量翻倍，1.6T产品验证通过");
        let cpo_hit = hits.iter().find(|h| h.chain == "AI硬件-CPO").unwrap();
        assert_eq!(cpo_hit.board_keyword, "CPO");
        assert!(!cpo_hit.board_keyword.is_empty());
    }

    #[test]
    fn test_new_energy_hydrogen() {
        let hits = map_news_to_chains("绿氢项目批量获批，PEM电解槽需求爆发在即");
        assert!(hits.iter().any(|h| h.chain == "新能源-氢能"));
    }

    #[test]
    fn test_rare_earth_magnets() {
        // 修复 v9.2 BR-002: 标题同时含 稀土/机器人 关键词, 只保留 priority 最高那条
        // 实际规则表: 机器人(80) 与 稀土永磁(80) priority 相同, 按 toml 出现顺序排序
        // 调整断言: 最多 1 条, 不强求具体哪个 (避免规则表顺序耦合)
        let hits = map_news_to_chains("稀土配额收紧叠加人形机器人放量，钕铁硼磁材供需缺口扩大");
        assert!(hits.len() <= 1, "BR-002: 一条快讯最多 1 条产业链, 实际 {} 条", hits.len());
        // 至少命中一条
        assert!(!hits.is_empty(), "应至少命中 1 条");
    }

    #[test]
    fn test_quantum_computing() {
        let hits = map_news_to_chains("中国量子计算原型机实现1000量子比特突破");
        assert!(hits.iter().any(|h| h.chain == "量子计算"));
    }

    #[test]
    fn test_board_match_prefers_exact() {
        let mut m = std::collections::HashMap::new();
        m.insert("PCB".to_string(), ("BK001".to_string(), 1.0));
        m.insert("PCB概念".to_string(), ("BK002".to_string(), 2.0));
        let got = find_best_board_match(&m, "PCB").unwrap();
        assert_eq!(got.0, "BK001");
    }

    #[test]
    fn test_board_match_works_with_contains() {
        let mut m = std::collections::HashMap::new();
        m.insert("印制电路板".to_string(), ("BK888".to_string(), 1.0));
        // PCB = Printed Circuit Board = 印制电路板，应能匹配
        let got = find_best_board_match(&m, "PCB");
        assert!(got.is_some(), "PCB should match 印制电路板 (Printed Circuit Board)");
        assert_eq!(got.unwrap().0, "BK888");

        let got2 = find_best_board_match(&m, "印制电路").unwrap();
        assert_eq!(got2.0, "BK888");
    }
}
