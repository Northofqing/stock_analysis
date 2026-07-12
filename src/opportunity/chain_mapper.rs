//! 新闻 → 产业链映射。
//!
//! 关键词规则表（优先）+ AI 推理兜底（规则未命中时）+ 动态板块成份股解析。
//! 不写死标的，标的从 sector_monitor 实时拉。

use std::sync::Arc;

use crate::market_analyzer::sector_monitor;

/// 板块候选池大小：兼顾覆盖率与请求成本。
const BOARD_RANK_TOP_N: usize = 200;
/// 成份股抓取上限：避免只拿到高成交额头部导致持仓漏判。
const COMPONENT_FETCH_TOP_N: usize = 100;
/// 最终保留的板块成份股数量。
const COMPONENT_KEEP_TOP_N: usize = 50;

#[derive(Debug, Clone, serde::Serialize)]
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
    /// B-002 板块联动归因: 直接从新闻标题中的板块名匹配 f3/f62 板块,
    /// 非主题催化, 区别于 Rule/Ai 的"主题深度催化".
    Board,
}

#[derive(Debug, Clone)]
pub struct ChainHit {
    pub chain: String,
    pub keywords: Vec<String>,
    pub logic: String,
    pub stocks: Vec<StockInfo>,
    /// 命中来源（规则 / AI / AI降级 / Board 板块联动）
    pub source: ChainSource,
    /// 用于动态匹配板块代码的板块名关键词；AI 来源时由 AI 提供，规则来源可留空（从规则表查）
    pub board_keyword: String,
    /// 匹配板块的今日主力净占比(%)；None = 资金数据不可用（不臆测多空）
    pub fund_flow_pct: Option<f64>,
    /// CR-1 (review): 真实板块代码 (e.g. "BK0815"), 用于 board_rotation_daily 的 PRIMARY KEY.
    /// Board 来源必填, 其他来源 None.
    pub board_code: Option<String>,
    /// CR-1 (review): 板块真实涨幅(%), 用于正确显示 "[板块联动] chg=2.5%" 而非 main_net_pct.
    /// Board 来源必填, 其他来源 None.
    pub board_change_pct: Option<f64>,
}

/// 产业链规则唯一来源：config/chain_rules.toml
///
/// 运行时优先读磁盘配置（支持热更新）；当文件缺失或解析失败时，
/// 回退到编译期内嵌的同一份 toml 文本，避免代码中维护第二份规则。
const DEFAULT_CHAIN_RULES_TOML: &str = include_str!("../../config/chain.toml");

fn normalize_chain_rules(
    mut rules: Vec<(Vec<String>, String, String, String, u32, bool)>,
) -> Vec<(Vec<String>, String, String, String, u32, bool)> {
    rules.sort_by(|a, b| b.4.cmp(&a.4));
    rules
}

fn map_chain_rules(
    config_rules: Arc<Vec<crate::config::ChainRuleConfig>>,
) -> Vec<(Vec<String>, String, String, String, u32, bool)> {
    // BR-006: 过滤 enabled=false 的规则. 关停的产业链不再参与关键词匹配,
    // 防止低胜率主题持续产生推送.
    config_rules
        .iter()
        .filter(|r| r.enabled)
        .map(|r| {
            (
                r.keywords.clone(),
                r.chain.clone(),
                r.logic.clone(),
                r.board_keyword.clone(),
                r.priority,
                r.generic,
            )
        })
        .collect()
}

fn parse_chain_rules_toml(
    toml_text: &str,
) -> Option<Vec<(Vec<String>, String, String, String, u32, bool)>> {
    let file_cfg = toml::from_str::<crate::config::ChainRulesFile>(toml_text).ok()?;
    Some(normalize_chain_rules(map_chain_rules(Arc::new(
        file_cfg.rules,
    ))))
}

/// 加载规则：优先 toml，不可用则回退编译期内嵌 toml。按 priority 降序返回。
fn chain_rules() -> Vec<(Vec<String>, String, String, String, u32, bool)> {
    // 1) 优先使用已热加载进内存的配置（通常由 config::load_all 填充）
    if let Some(config_rules) = crate::config::get_chain_rules() {
        log_disabled_themes(&config_rules);
        return normalize_chain_rules(map_chain_rules(config_rules));
    }

    // 2) 若内存缓存为空，直接读取 config/chain_rules.toml，避免与文件配置脱节。
    if let Ok(s) = std::fs::read_to_string("config/chain.toml") {
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

/// BR-006: 启动时单次打印被关停的主题, 便于 audit.
/// 只在 chain_rules() 首次加载内存配置时打印一次 (热更新时 SIGHUP 触发 reload, 也走这里).
fn log_disabled_themes(rules: &[crate::config::ChainRuleConfig]) {
    let disabled: Vec<&str> = rules
        .iter()
        .filter(|r| !r.enabled)
        .map(|r| r.chain.as_str())
        .collect();
    if !disabled.is_empty() {
        log::info!(
            "[ChainMapper] BR-006 关停 {} 个 0% 主题: [{}]",
            disabled.len(),
            disabled.join(", ")
        );
    }
}

/// 从新闻标题中匹配产业链（按 priority 降序遍历，高优先级规则先匹配）
///
/// 修复 v9.2 BR-002: 一条快讯最多 1 条产业链（例外: AI 给出 ≥2 条独立产业链）
pub fn map_news_to_chains(title: &str) -> Vec<ChainHit> {
    let mut hits: Vec<ChainHit> = Vec::new();
    let rules = chain_rules();

    for (keywords, chain, logic, board_keyword, _priority, _generic) in &rules {
        let matched: Vec<&str> = keywords
            .iter()
            .filter(|kw| title.contains(kw.as_str()))
            .map(|s| s.as_str())
            .collect();
        if matched.is_empty() {
            continue;
        }

        // BR-002 互斥: 只保留第 1 条命中 (按 priority 降序遍历, 优先级最高先匹配)
        // 注: 不再允许"一条快讯命中 N 条产业链"除非 AI 显式给出多条独立逻辑
        // (历史 line 111 `hits.iter().any(|h| h.chain == *chain)` dedup 已删除 — BR-002
        //  互斥覆盖了"最多 1 条"语义, 同 chain 不可能再被 push)
        if !hits.is_empty() {
            log::debug!(
                "[ChainMapper] 互斥: {} 已命中, 跳过 {} (BR-002)",
                hits[0].chain,
                chain
            );
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
            board_code: None,
            board_change_pct: None,
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
        rules
            .iter()
            .any(|(_, chain, _, _, _, generic)| chain == chain_name && *generic)
    };

    if !rule_hits.is_empty() && !should_call_ai {
        return rule_hits; // 规则命中且不需要二次分类
    }

    // 规则命中过于单一或完全未命中 → AI 兜底分类
    if should_call_ai {
        log::info!(
            "[ChainMapper] 规则命中1条通用规则({}) + {} 条新闻 → 调 AI 二次分类验证多样性",
            rule_hits[0].chain,
            titles.len()
        );
    } else if rule_hits.is_empty() {
        log::info!(
            "[ChainMapper] 规则未命中({} 条新闻) → 调 AI 兜底",
            titles.len()
        );
    }

    // 规则未命中或需要二次分类 → AI 兜底。
    // GeminiAnalyzer 含 RefCell（非 Sync），跨 await 会破坏外层 Future 的 Send，
    // 故隔离在独立 blocking 线程的 current-thread 运行时内执行。
    let titles_owned = titles.to_vec();
    let existing_chain = rule_hits.first().map(|h| h.chain.clone());
    tokio::task::spawn_blocking(move || {
        // 修复 Top10#5 (2026-06-29 audit): 用 crate::block_on_async 统一替代
        // spawn_blocking 内新建 current_thread runtime 的 pattern.
        crate::block_on_async(async move {
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
        if line.is_empty() || line == "无" || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
        if parts.len() < 3 {
            continue;
        }
        let chain = parts[0].to_string();
        let logic = parts[1].to_string();
        let board_keyword = parts[2].to_string();
        if chain.is_empty() || board_keyword.is_empty() {
            continue;
        }
        if hits.iter().any(|h| h.chain == chain) {
            continue;
        }
        hits.push(ChainHit {
            chain,
            keywords: vec![board_keyword.clone()],
            logic,
            stocks: Vec::new(),
            source: ChainSource::Ai,
            board_keyword,
            fund_flow_pct: None,
            board_code: None,
            board_change_pct: None,
        });
        if hits.len() >= 3 {
            break;
        }
    }
    hits
}

// ═══════════════════════════════════════════════════════════════════════════
// B-002 板块联动归因 (2026-07-09)
// 背景: 002208 (合肥城建) 涨停 +10% 当天, chain_rules 全未命中,
//       外部新闻明确是 "房地产板块短线拉升" 但归因失败.
// 解决: 新增第二条新闻→归因路径, 直接从标题板块名匹配 f3/f62 板块.
//       不依赖 chain.toml 规则表, 自动覆盖未来新板块.
// ═══════════════════════════════════════════════════════════════════════════

/// 板块联动触发词 (硬编码, 不走环境变量避免不一致).
/// 任一词在新闻标题中出现即视为"板块轮动"型新闻.
const BOARD_TRIGGER_WORDS: &[&str] = &[
    "拉升",
    "异动",
    "上涨",
    "上行",
    "上扬",
    "走强",
    "普涨",
    "活跃",
    "爆发",
    "飙升",
    "强势",
    "涨停潮",
    "全线",
];

/// 个股异动门槛 (chg_pct >= 5%). 创/科/ST 用 component_limit_pct 动态调整 (后续可加).
const BOARD_MIN_STOCK_CHG: f64 = 5.0;

fn title_has_trigger_word(title: &str) -> bool {
    BOARD_TRIGGER_WORDS.iter().any(|w| title.contains(w))
}

/// FIX-B-002: 真接数据 - 新闻是否实际提到该板块的任一个股
///
/// 用 6 位 A 股代码 regex + component name 子串匹配. 任一命中即真相关.
/// 例: "2025年报预减: Meta..." 找不到 "600879" 等 → 排除, 不再误中 "2025年报预减" 板块.
fn news_relevant_to_board(
    title: &str,
    movers: &[StockInfo],
) -> bool {
    // 1) 6 位 A 股代码 regex (避开 "2025" / "11" 这类年份数字)
    // 用 chars() 避免 UTF-8 边界 panic
    let chars: Vec<char> = title.chars().collect();
    for i in 0..chars.len().saturating_sub(5) {
        let candidate: String = chars[i..i + 6].iter().collect();
        if candidate.chars().all(|c| c.is_ascii_digit())
            && candidate.starts_with(['0', '3', '6'])
            && movers.iter().any(|m| m.code == candidate)
        {
            return true;
        }
    }
    // 2) 板块个股名子串匹配 (例: 标题含 "中航光电", 组件列表有 "中航光电")
    for m in movers {
        if m.name.len() >= 4 && title.contains(&m.name) {
            return true;
        }
    }
    false
}

/// 检查标题是否提及某板块 (支持 "房地产板块..." 命中 "房地产开发" 板块).
///
/// 匹配规则 (按板块名长度分支):
/// - ≥3 字符 (e.g. "房地产开发", "PCB", "机器人概念"):
///     1) 直接包含 (normalized), 或
///     2) 标题含板块名前缀 (≥3 字符), e.g. "房地产板块短线拉升" 命中 "房地产开发" (前缀 "房地产")
/// - 2 字符 (e.g. "银行", "白酒", "地产"):
///     必须紧邻 "板块" 词 (e.g. "银行板块拉升" 命中, "房地产板块拉升" 不命中 "地产")
/// - 1 字符: 拒绝匹配 (噪声太大)
///
/// 返回 true 表示标题提到该板块.
fn title_mentions_board(title: &str, board_name: &str) -> bool {
    let title_norm = normalize_board_text(title);
    let name_norm = normalize_board_text(board_name);
    if title_norm.is_empty() || name_norm.is_empty() {
        return false;
    }
    let name_len = name_norm.chars().count();

    // 1 字符: 直接拒绝 (e.g. "A", "煤" 等单字噪声太大)
    if name_len < 2 {
        return false;
    }

    // 2 字符: 必须以 "{name}板块" 开头 (e.g. "银行板块拉升" 命中, "房地产板块拉升" 不命中 "地产")
    // 用 starts_with 而非 contains, 避免 "地产" 误命中 "房地产板块..." 的中间子串
    if name_len == 2 {
        let with_suffix = format!("{}板块", name_norm);
        return title_norm.starts_with(&with_suffix);
    }

    // ≥ 3 字符: 直接 contains + 前缀匹配
    // FIX-6: 不要加 word boundary 检查 (中文无空格, 边界定义模糊, 反而误伤).
    // 真正防"老 news 误推"靠下面 FIX-6 把 BATCH_DEFAULT_MAX_AGE 2d→4h.
    if title_norm.contains(&name_norm) {
        return true;
    }
    let prefix_len = 3.min(name_len);
    let prefix: String = name_norm.chars().take(prefix_len).collect();
    title_norm.contains(&prefix)
}

/// 纯函数: 给定新闻标题 + 已抓取的板块列表 + 板块成份股获取函数, 生成 Board ChainHit.
///
/// 不发 HTTP, 可单测. HTTP 包装见 `extract_board_rotation`.
///
/// 输入假设:
/// - `boards` 已去重 (按 name)
/// - `components_for(code)` 拉取并返回该板块的成份股 (含 code/name/change_pct)
///
/// 输出:
/// - 每条新闻每命中一个板块 → 1 条 ChainHit (已按 board name 去重)
/// - 同 board 多条新闻 → 仅第一条产 hit (避免噪声)
/// - 板块 `change_pct > 0` 且 `main_net_pct_today > 0` 才产出 (避免"下跌"误归因)
/// - 板块内 `change_pct >= BOARD_MIN_STOCK_CHG` 的成份股才纳入 (只推异动股)
/// - 0 异动股的板块不产 hit
pub fn extract_board_rotation_with<F>(
    titles: &[String],
    boards: &[crate::market_analyzer::sector_monitor::ConceptBoard],
    components_for: F,
) -> Vec<ChainHit>
where
    F: Fn(&str) -> Vec<crate::market_analyzer::sector_monitor::BoardStock>,
{
    use std::collections::HashSet;

    let mut hits: Vec<ChainHit> = Vec::new();
    let mut seen_boards: HashSet<String> = HashSet::new();

    for title in titles {
        if !title_has_trigger_word(title) {
            continue;
        }
        for board in boards {
            // 板块"真在涨"双重验证
            if !(board.change_pct > 0.0 && board.main_net_pct_today > 0.0) {
                continue;
            }
            if !title_mentions_board(title, &board.name) {
                continue;
            }

            // 拉成份股 + 过滤异动股
            let comps = components_for(&board.code);
            let movers: Vec<StockInfo> = comps
                .into_iter()
                .filter(|s| s.change_pct >= BOARD_MIN_STOCK_CHG)
                .map(|s| StockInfo {
                    code: s.code,
                    name: s.name,
                    change_pct: s.change_pct,
                    vol_ratio: s.vol_ratio,
                })
                .collect();
            if movers.is_empty() {
                // CR-30 (review): 该板块无异动股, 不 insert seen_boards (允许下一个 title 尝试)
                // 之前在 line 439 提前 insert, 一个 transient fetch failure 会 black-hole
                // 整个 batch 对该 board 的产 hit 机会.
                continue;
            }
            // FIX-B-002: 真接数据 - 新闻必须提及该板块的任一个股 (name 或 code)
            //   之前只查 title_mentions_board 子串匹配, 例 "2025年报预减: Meta..." 含
            //   "2025年报预减" 子串 → 误中 "2025年报预减" 板块 (实际是 Meta 美国新闻)
            //   修复: 提取 news title 里 6 位股票代码 + 检查组件 names, 必须有交集才算真相关
            if !news_relevant_to_board(title, &movers) {
                log::debug!(
                    "[FIX-B-002] skip board '{}' for title '{}' (no stock relevance)",
                    board.name, title
                );
                continue;
            }
            // 同 board 仅首条新闻产 hit (避免噪声重复推送) — 现在 insert 时机移到 movers 检查后
            if !seen_boards.insert(board.name.clone()) {
                continue;
            }

            hits.push(ChainHit {
                chain: format!("[板块联动] {}", board.name),
                keywords: vec![board.name.clone()],
                logic: title.clone(),
                stocks: movers,
                source: ChainSource::Board,
                board_keyword: board.name.clone(),
                fund_flow_pct: Some(board.main_net_pct_today),
                // CR-1 (review): 用真实板块代码 (e.g. BK0815) 作 PK, 板块涨幅 (e.g. 2.5%) 作 chg
                // 避免: board_code 错存为板块名, board_change_pct 错存为 main_net_pct
                board_code: Some(board.code.clone()),
                board_change_pct: Some(board.change_pct),
            });
        }
    }

    hits
}

/// HTTP wrapper: 拉 f3+f62 板块榜 + 拉成份股 + 调 `extract_board_rotation_with`.
///
/// 在生产路径调用 (run_opportunity_scan). 测试用 `extract_board_rotation_with` 注入 mock.
pub fn extract_board_rotation(titles: &[String]) -> Vec<ChainHit> {
    use crate::market_analyzer::sector_monitor;
    use std::collections::HashSet;

    // f3 (按涨幅) + f62 (按主力净流入) 合并去重, 两路都有完整字段
    let mut boards: Vec<sector_monitor::ConceptBoard> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    if let Ok(b) = sector_monitor::fetch_board_ranking("f3", BOARD_RANK_TOP_N) {
        for board in b {
            if seen.insert(board.name.clone()) {
                boards.push(board);
            }
        }
    }
    if let Ok(b) = sector_monitor::fetch_board_ranking("f62", BOARD_RANK_TOP_N) {
        for board in b {
            if seen.insert(board.name.clone()) {
                boards.push(board);
            }
        }
    }

    log::debug!(
        "[BoardRotation] f3+f62 共 {} 个独立板块, 开始扫描 {} 条新闻",
        boards.len(),
        titles.len()
    );

    // CR-13 (review): 用 std::thread::scope 并行拉成份股, 替代之前的顺序拉.
    //   之前: 30 个命中板块 × ~100ms HTTP = 3s; 100 个 = 10s.
    //   现在: max(单次 HTTP) ≈ 100ms + 少量 overhead.
    extract_board_rotation_parallel(titles, &boards)
}

/// FIX-4 (review): 真正 1-pass 版本, 修之前 3-pass 浪费 CPU
/// 之前: Pass 1 (stub 找 needed_codes) + Pass 3 (用 cache 跑实际逻辑) 都遍历 titles×boards
///       跑同一套 predicate (trigger word / change_pct>0 / main_net_pct>0 / title_mentions_board).
/// 现在: Phase A 单次迭代 titles×boards 收集 candidate board codes (no HTTP),
///       Phase B 并行拉成份股 (only for codes found in Phase A),
///       Phase C 单次遍历 Phase A 候选, 应用 movers 过滤 + emit hits (用 Phase B cache).
/// 真正 1-pass, 0 重复 predicate evaluation.
fn extract_board_rotation_parallel(
    titles: &[String],
    boards: &[crate::market_analyzer::sector_monitor::ConceptBoard],
) -> Vec<ChainHit> {
    use std::collections::{HashMap, HashSet};

    // Phase A: 收集候选 board codes — 跑完整 predicate 但不拉 HTTP
    // 同 board 仅首条 title 产候选 (de-dup by seen_boards, 与 extract_board_rotation_with 行为一致)
    let mut candidate_codes: HashSet<String> = HashSet::new();
    let mut seen_boards: HashSet<String> = HashSet::new();
    for title in titles {
        if !title_has_trigger_word(title) {
            continue;
        }
        for board in boards {
            if !(board.change_pct > 0.0 && board.main_net_pct_today > 0.0) {
                continue;
            }
            if !title_mentions_board(title, &board.name) {
                continue;
            }
            if !seen_boards.insert(board.name.clone()) {
                continue;
            }
            candidate_codes.insert(board.code.clone());
        }
    }
    log::debug!(
        "[BoardRotation] Phase A: {} 个候选板块 (single pass, no HTTP)",
        candidate_codes.len()
    );

    // Phase B: 并行拉候选板块的成份股 (only for codes found in Phase A, no wasted fetch)
    let components_cache: HashMap<String, Vec<crate::market_analyzer::sector_monitor::BoardStock>> =
        std::thread::scope(|s| {
            let results: Vec<(
                String,
                Vec<crate::market_analyzer::sector_monitor::BoardStock>,
            )> = Vec::new();
            let shared_results = std::sync::Arc::new(std::sync::Mutex::new(results));
            let handles: Vec<_> = candidate_codes
                .iter()
                .map(|code| {
                    let code_owned = code.clone();
                    let shared = shared_results.clone();
                    s.spawn(move || {
                        let comps = sector_monitor::fetch_board_components(
                            &code_owned,
                            COMPONENT_FETCH_TOP_N,
                        )
                        .unwrap_or_default();
                        shared
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push((code_owned, comps));
                    })
                })
                .collect();
            for h in handles {
                let _ = h.join();
            }
            let mut map: HashMap<String, Vec<crate::market_analyzer::sector_monitor::BoardStock>> =
                HashMap::new();
            for (code, comps) in shared_results
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
            {
                map.insert(code.clone(), comps.clone());
            }
            map
        });

    // Phase C: 单次遍历 titles, 应用 movers 过滤 + emit hits (用 Phase B cache)
    // 不再跑 predicate (Phase A 已跑过), 直接用 candidate_codes 集合过滤
    let mut hits: Vec<ChainHit> = Vec::new();
    let mut seen_phase_c: HashSet<String> = HashSet::new();
    for title in titles {
        if !title_has_trigger_word(title) {
            continue;
        }
        for board in boards {
            // 同 board 仅首条 title 触发 — 必须与 Phase A 严格一致, 否则 hit 数量会不一致
            if !seen_phase_c.insert(board.name.clone()) {
                continue;
            }
            // 用 Phase A 收集的 candidate_codes 决定 (避免重复跑 predicate)
            if !candidate_codes.contains(&board.code) {
                continue;
            }
            let comps = components_cache
                .get(&board.code)
                .cloned()
                .unwrap_or_default();
            let movers: Vec<StockInfo> = comps
                .into_iter()
                .filter(|s| s.change_pct >= BOARD_MIN_STOCK_CHG)
                .map(|s| StockInfo {
                    code: s.code,
                    name: s.name,
                    change_pct: s.change_pct,
                    vol_ratio: s.vol_ratio,
                })
                .collect();
            if movers.is_empty() {
                continue;
            }
            hits.push(ChainHit {
                chain: format!("[板块联动] {}", board.name),
                keywords: vec![board.name.clone()],
                logic: title.clone(),
                stocks: movers,
                source: ChainSource::Board,
                board_keyword: board.name.clone(),
                fund_flow_pct: Some(board.main_net_pct_today),
                board_code: Some(board.code.clone()),
                board_change_pct: Some(board.change_pct),
            });
        }
    }
    log::debug!(
        "[BoardRotation] Phase C: {} hits (single pass with cache)",
        hits.len()
    );
    hits
}

/// 为 ChainHit 动态解析标的。
/// 先拉取全部板块排名，按板块名关键词匹配板块代码，再拉成份股。
pub fn resolve_stocks(hits: &mut [ChainHit]) {
    // 一次性拉两路榜单并并集，避免只看涨幅榜导致板块池过窄。
    let mut board_map: std::collections::HashMap<String, (String, f64)> =
        std::collections::HashMap::new();
    if let Ok(boards) = sector_monitor::fetch_board_ranking("f3", BOARD_RANK_TOP_N) {
        for b in boards {
            board_map
                .entry(b.name)
                .or_insert((b.code, b.main_net_pct_today));
        }
    }
    if let Ok(boards) = sector_monitor::fetch_board_ranking("f62", BOARD_RANK_TOP_N) {
        for b in boards {
            board_map
                .entry(b.name)
                .or_insert((b.code, b.main_net_pct_today));
        }
    }
    if board_map.is_empty() {
        return;
    }

    let mut failed_chains: Vec<String> = Vec::new();

    for hit in hits.iter_mut() {
        // B-002 板块联动归因 (Board): stocks 已按异动股过滤填好, 不需要再 resolve
        // (否则会按 board_keyword 重新拉板块成份股, 覆盖掉我们的异动股列表)
        if hit.source == ChainSource::Board {
            continue;
        }
        // 优先用 hit 自带的 board_keyword（Rule 来源已直接存储，AI 来源亦然）
        let mut board_keyword = hit.board_keyword.clone();
        // 安全兜底：若 board_keyword 为空，从规则表回溯查找
        if board_keyword.is_empty() {
            let rules = chain_rules();
            board_keyword = rules
                .iter()
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
                    failed_chains.push(format!(
                        "  • {} — 未找到板块关键词'{}'的对应板块",
                        hit.chain, board_keyword
                    ));
                    continue;
                }
                Err(e) => {
                    failed_chains.push(format!(
                        "  • {} — 板块关键词'{}' suggest 查询失败: {}",
                        hit.chain, board_keyword, e
                    ));
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
                let filtered: Vec<_> = stocks
                    .into_iter()
                    .filter(|s| !s.code.starts_with('8') && !s.code.starts_with('4'))
                    .filter(|s| !s.code.starts_with("688"))
                    .take(COMPONENT_KEEP_TOP_N)
                    .map(|s| StockInfo {
                        code: s.code,
                        name: s.name,
                        change_pct: s.change_pct,
                        vol_ratio: s.vol_ratio,
                    })
                    .collect();

                if filtered.is_empty() {
                    failed_chains.push(format!(
                        "  • {} — 板块'{}' 无有效成分股（过滤北交所/科创板）",
                        hit.chain, board_keyword
                    ));
                } else {
                    hit.stocks = filtered;
                }
            }
            Err(e) => {
                failed_chains.push(format!(
                    "  • {} — 板块'{}' 拉取失败: {}",
                    hit.chain, board_keyword, e
                ));
            }
        }
    }

    if !failed_chains.is_empty() {
        log::debug!(
            "[ChainMapper] 产业链标的解析：{} 条产业链失败\n{}",
            failed_chains.len(),
            failed_chains.join("\n")
        );
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
        if board_name == synonym
            || name_norm == syn_norm
            || board_name.contains(synonym)
            || name_norm.contains(&syn_norm)
        {
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
        .filter_map(|(name, v)| {
            board_match_score(name, keyword).map(|score| (score, name.len(), v.clone()))
        })
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)))
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
        // 修复 C-3 (2026-06-29 codex review): 恢复强断言 — BR-002 spec 例外条款
        // 说"AI 给出 ≥2 条独立产业链可保留", 单元测试应覆盖**至少 1 条**,
        // 不能退化为"随便命中 0 或 1 条". 当前 chain_rules.toml: PCB priority=100
        // (toml 中排在 MLCC 之前), MLCC priority=100, 按 priority 降序 + toml 顺序
        // PCB 应胜出, MLCC 应被互斥排除.
        let hits = map_news_to_chains("MLCC突破带动PCB和半导体产业链全线走强");
        assert_eq!(
            hits.len(),
            1,
            "BR-002: 一条快讯最多 1 条产业链, 实际 {} 条",
            hits.len()
        );
        assert_eq!(
            hits[0].chain, "AI硬件-PCB",
            "BR-002: PCB 优先级=100 且 toml 顺序在前, 应胜出 MLCC"
        );
        assert!(
            !hits.iter().any(|h| h.chain == "AI硬件-MLCC"),
            "BR-002: MLCC 应被互斥排除"
        );
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

    // 修复 C-3 (2026-06-29 codex review): BR-002 spec 例外条款"AI 给出 ≥2 条独立产业链
    // 可保留", 单元测试需覆盖**parse 层支持 ≥2 条独立链**, 不能依赖 map_news_to_chains_ai
    // 整体调用 (后者需要 mock GeminiAnalyzer, 测试在 CI 难构造).
    // 间接覆盖: parse_ai_chains 在 AI 输出 3 条独立产业链时, 全部解析 + 来源标记为 Ai
    // + 截断到 3 条 (line 264 hits.len() >= 3 break).
    #[test]
    fn test_br002_exception_parse_keeps_multiple_independent_chains() {
        let text = "固态电池|技术迭代催化|固态电池\n机器人|人形量产提速|机器人\nPCB|AI服务器需求激增|印制电路板";
        let hits = parse_ai_chains(text);
        assert_eq!(
            hits.len(),
            3,
            "BR-002 例外: AI 给出 3 条独立产业链应全部保留, 实际 {} 条",
            hits.len()
        );
        // 全部标记为 Ai 来源 (BR-002 例外专属)
        assert!(
            hits.iter().all(|h| h.source == ChainSource::Ai),
            "AI 来源标记必须为 ChainSource::Ai"
        );
        // 3 条链必须独立 (无包含关系, 关键词不重叠)
        let chains: Vec<&str> = hits.iter().map(|h| h.chain.as_str()).collect();
        assert!(chains.contains(&"新能源-固态电池") || chains.contains(&"固态电池"));
        assert!(chains.iter().any(|c| c.contains("机器人")));
        assert!(chains
            .iter()
            .any(|c| c.contains("PCB") || c.contains("电路")));
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
        // BR-006 (2026-06-29): AI硬件-CPO 已关停, 改用 AI硬件-PCB (已加权到 priority 95)
        let hits = map_news_to_chains("PCB全线涨价20%，HDI高多层板持续紧缺");
        let pcb_hit = hits.iter().find(|h| h.chain == "AI硬件-PCB").unwrap();
        assert_eq!(pcb_hit.board_keyword, "PCB");
        assert!(!pcb_hit.board_keyword.is_empty());
    }

    #[test]
    fn test_new_energy_hydrogen() {
        let hits = map_news_to_chains("绿氢项目批量获批，PEM电解槽需求爆发在即");
        assert!(hits.iter().any(|h| h.chain == "新能源-氢能"));
    }

    #[test]
    fn test_rare_earth_magnets() {
        // 修复 C-3 (2026-06-29 codex review): 恢复强断言 — 稀土永磁 BR-006 关停,
        // 不应在 map_news_to_chains 中命中. 标题含"机器人"关键词, 应只命中
        // 机器人(priority=80), 不命中关停的稀土永磁.
        let hits = map_news_to_chains("稀土配额收紧叠加人形机器人放量，钕铁硼磁材供需缺口扩大");
        assert!(!hits.is_empty(), "应至少命中机器人");
        assert_eq!(
            hits.len(),
            1,
            "BR-002: 互斥后应只命中 1 条, 实际 {} 条",
            hits.len()
        );
        assert_eq!(
            hits[0].chain, "机器人",
            "稀土永磁 BR-006 关停, 机器人 priority=80 应胜出"
        );
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
        assert!(
            got.is_some(),
            "PCB should match 印制电路板 (Printed Circuit Board)"
        );
        assert_eq!(got.unwrap().0, "BK888");

        let got2 = find_best_board_match(&m, "印制电路").unwrap();
        assert_eq!(got2.0, "BK888");
    }

    // BR-006 (2026-06-29): 0% 胜率主题关停, chain_mapper 加载规则时跳过 enabled=false.
    //
    // v24 状态: 用户要求重开 7 个关停主题 (AI硬件-液冷/半导体-先进封装/消费电子/
    // 稀土永磁/新能源-电池/稀有金属/AI硬件-液冷副本), 全部 enabled=true.
    // 当前 chain.toml 86 条规则全部 enabled, BR-006 关停池为空.
    //
    // 此测试在 v24 已被废除 (无法验证不存在的关停主题).
    // 如未来新增 BR-006 关停主题, 恢复本测试并按需加新 case.

    // BR-006 加权: PCB 真实胜率 44.4% (12/27), priority 90→95.
    // 测试: PCB 新闻仍命中 (PCB 启用, 仅 priority 提高).
    #[test]
    fn test_br006_enabled_chains_still_match() {
        let hits = map_news_to_chains("PCB全线涨价20%，HDI高多层板持续紧缺");
        assert!(hits.iter().any(|h| h.chain == "AI硬件-PCB"));
    }

    // ===== 板块联动归因 (B-002 修复) =====
    // 背景: 002208 (合肥城建) 涨停 +10% 当天 chain_rules 全未命中,
    //       外部新闻明确是 "房地产板块短线拉升" 但归因失败.
    // 新增 extract_board_rotation_with 直接从标题板块名匹配 f3/f62 板块,
    //       生成 ChainSource::Board 类型的 ChainHit, 走相同 assess_impact 路径.

    use crate::market_analyzer::sector_monitor::{BoardStock, ConceptBoard};

    fn make_board(name: &str, change_pct: f64, main_net_pct: f64) -> ConceptBoard {
        ConceptBoard {
            code: format!("BK_{}", name),
            name: name.to_string(),
            change_pct,
            main_inflow: 0.0,
            leader_name: String::new(),
            vol_ratio: 1.0,
            turnover: 0.0,
            main_net_pct_today: main_net_pct,
            main_net_pct_5d: 0.0,
        }
    }

    fn make_stock(code: &str, name: &str, change_pct: f64) -> BoardStock {
        BoardStock {
            code: code.to_string(),
            name: name.to_string(),
            change_pct,
            amount: 0.0,
            vol_ratio: 1.0,
            turnover: 0.0,
        }
    }

    /// B-002 用户场景回归: 房地产板块拉升 + 002208 涨停 → 应生成 Board ChainHit
    #[test]
    fn test_extract_board_rotation_basic() {
        let titles = vec!["房地产板块短线拉升，合肥城建涨停成交额3.24亿元".to_string()];
        let boards = vec![make_board("房地产开发", 2.5, 1.5)];
        let components = vec![
            make_stock("002208", "合肥城建", 10.0), // 异动股, 纳入
            make_stock("000002", "万科A", 2.0),     // 涨幅不足, 过滤
        ];
        let components_for = |code: &str| -> Vec<BoardStock> {
            if code == "BK_房地产开发" {
                components.clone()
            } else {
                vec![]
            }
        };

        let hits = extract_board_rotation_with(&titles, &boards, components_for);

        assert_eq!(
            hits.len(),
            1,
            "应产出 1 条 Board ChainHit, 实际 {}",
            hits.len()
        );
        let hit = &hits[0];
        assert_eq!(
            hit.chain, "[板块联动] 房地产开发",
            "chain 字段应带 [板块联动] 前缀"
        );
        assert_eq!(
            hit.source,
            ChainSource::Board,
            "source 必须是新增的 Board 变体"
        );
        assert!(
            hit.logic.contains("房地产板块短线拉升"),
            "logic 应保留原始新闻标题"
        );
        assert_eq!(hit.stocks.len(), 1, "涨幅 >= 5% 的只有 002208 一只");
        assert_eq!(hit.stocks[0].code, "002208");
        assert_eq!(
            hit.fund_flow_pct,
            Some(1.5),
            "fund_flow_pct 应透传板块 main_net_pct_today"
        );
    }

    /// 板块 change_pct < 0 → 不产 ChainHit (避免"板块下跌"误归因)
    #[test]
    fn test_extract_board_rotation_filters_negative_board() {
        let titles = vec!["房地产板块短线拉升，合肥城建涨停".to_string()];
        let boards = vec![make_board("房地产开发", -1.5, 2.0)]; // 下跌
        let components = vec![make_stock("002208", "合肥城建", 10.0)];
        let components_for = |_: &str| components.clone();

        let hits = extract_board_rotation_with(&titles, &boards, components_for);
        assert!(hits.is_empty(), "板块下跌时不应产 ChainHit");
    }

    /// 板块 main_net_pct_today = 0 → 不产 ChainHit (资金未流入)
    #[test]
    fn test_extract_board_rotation_filters_zero_main_net() {
        let titles = vec!["房地产板块短线拉升".to_string()];
        let boards = vec![make_board("房地产开发", 2.0, 0.0)]; // 主力未流入
        let components = vec![make_stock("002208", "合肥城建", 10.0)];
        let components_for = |_: &str| components.clone();

        let hits = extract_board_rotation_with(&titles, &boards, components_for);
        assert!(hits.is_empty(), "主力净占比 = 0 时不应产 ChainHit");
    }

    /// 标题无 trigger 词 → 不产 ChainHit (避免任何新闻都触发)
    #[test]
    fn test_extract_board_rotation_requires_trigger_word() {
        let titles = vec![
            "房地产板块今日走势平稳".to_string(), // 无拉升/异动等触发词
            "房地产板块下跌 3%".to_string(),      // 下跌
        ];
        let boards = vec![make_board("房地产开发", 2.0, 1.0)];
        let components = vec![make_stock("002208", "合肥城建", 10.0)];
        let components_for = |_: &str| components.clone();

        let hits = extract_board_rotation_with(&titles, &boards, components_for);
        assert!(hits.is_empty(), "无 trigger 词时不应产 ChainHit");
    }

    /// 同 board 多条新闻 → 仅首条产 ChainHit (避免噪声重复推送)
    #[test]
    fn test_extract_board_rotation_dedup_by_board() {
        let titles = vec![
            "房地产板块短线拉升，合肥城建涨停".to_string(),
            "房地产板块午后继续走强，多股涨停".to_string(),
            "房地产开发今日表现强势".to_string(),
        ];
        let boards = vec![make_board("房地产开发", 2.5, 1.5)];
        let components = vec![make_stock("002208", "合肥城建", 10.0)];
        let components_for = |_: &str| components.clone();

        let hits = extract_board_rotation_with(&titles, &boards, components_for);
        assert_eq!(hits.len(), 1, "同 board 多条新闻应仅产出 1 条 ChainHit");
    }

    /// 多 board 多新闻: 每个 board 各自产 1 条 ChainHit (独立归因)
    #[test]
    fn test_extract_board_rotation_multi_board_multi_news() {
        // FIX-B-002: 测试数据需含板块个股, 否则真接数据逻辑视为无相关 drop
        let titles = vec![
            "房地产板块短线拉升，002208 合肥城建涨停".to_string(),
            "银行板块异动拉升，600036 招商银行涨超 5%".to_string(),
        ];
        let boards = vec![
            make_board("房地产开发", 2.5, 1.5),
            make_board("银行", 1.8, 0.8),
        ];
        let components_1 = vec![make_stock("002208", "合肥城建", 10.0)];
        let components_2 = vec![make_stock("600036", "招商银行", 5.5)];
        let components_for = |code: &str| -> Vec<BoardStock> {
            if code == "BK_房地产开发" {
                components_1.clone()
            } else if code == "BK_银行" {
                components_2.clone()
            } else {
                vec![]
            }
        };

        let hits = extract_board_rotation_with(&titles, &boards, components_for);
        assert_eq!(hits.len(), 2, "2 个 board 各产 1 条 ChainHit");
        let chains: Vec<&str> = hits.iter().map(|h| h.chain.as_str()).collect();
        assert!(chains.contains(&"[板块联动] 房地产开发"));
        assert!(chains.contains(&"[板块联动] 银行"));
    }

    /// 板块真在涨但无异动股 (成份股全部涨幅 < 5%) → 不产 ChainHit
    #[test]
    fn test_extract_board_rotation_skips_when_no_movers() {
        let titles = vec!["房地产板块短线拉升".to_string()];
        let boards = vec![make_board("房地产开发", 2.5, 1.5)];
        let components = vec![
            make_stock("000002", "万科A", 2.0), // 全部 < 5%
            make_stock("600048", "保利发展", 3.5),
        ];
        let components_for = |_: &str| components.clone();

        let hits = extract_board_rotation_with(&titles, &boards, components_for);
        assert!(
            hits.is_empty(),
            "无异动股时不应产 ChainHit (避免推送空板块)"
        );
    }

    /// Trigger 词覆盖测试: 拉升/异动/上涨/走强/普涨/活跃/爆发/涨停潮/全线/飙升
    #[test]
    fn test_extract_board_rotation_all_trigger_words() {
        let triggers = [
            "拉升",
            "异动",
            "上涨",
            "走强",
            "普涨",
            "活跃",
            "爆发",
            "涨停潮",
            "全线",
            "飙升",
            "上行",
            "上扬",
            "强势",
        ];
        let boards = vec![make_board("房地产开发", 2.0, 1.0)];
        let components = vec![make_stock("002208", "合肥城建", 10.0)];
        let components_for = |_: &str| components.clone();

        for trig in triggers {
            let title = format!("房地产板块{trig}，合肥城建涨停");
            let hits = extract_board_rotation_with(&[title], &boards, components_for);
            assert!(
                hits.len() == 1,
                "trigger 词 '{}' 应触发 ChainHit, 实际 0 条",
                trig
            );
        }
    }

    /// 短板块名 (2 字或以下) 不被前缀匹配误命中 (避免 "AI" 命中 "AI芯片" 误匹配)
    #[test]
    fn test_extract_board_rotation_short_board_name_skipped() {
        let titles = vec!["房地产板块短线拉升".to_string()];
        let boards = vec![make_board("地产", 2.5, 1.5)]; // 仅 2 字, 应被跳过
        let components = vec![make_stock("002208", "合肥城建", 10.0)];
        let components_for = |_: &str| components.clone();

        let hits = extract_board_rotation_with(&titles, &boards, components_for);
        assert!(
            hits.is_empty(),
            "板块名过短 (2 字) 应被前缀匹配跳过, 避免误命中"
        );
    }

    /// 标题提到 board 但 board 不在 boards 列表 (例如已被淘汰出 top 200)
    #[test]
    fn test_extract_board_rotation_board_not_in_list() {
        let titles = vec!["房地产板块短线拉升".to_string()];
        let boards = vec![make_board("机器人概念", 2.5, 1.5)]; // 列表里没有房地产
        let components = vec![make_stock("002208", "合肥城建", 10.0)];
        let components_for = |_: &str| components.clone();

        let hits = extract_board_rotation_with(&titles, &boards, components_for);
        assert!(hits.is_empty(), "board 不在列表中时应不产 ChainHit");
    }
}
