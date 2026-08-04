//! Registered business rules: BR-067, BR-174, BR-181.
//! 新闻 → 产业链映射。
//!
//! 关键词规则表（优先）+ AI 推理兜底（规则未命中时）。
//! BR-174 后，本模块只负责产业链语义映射，不再做模糊板块检索、Top-N
//! 截断或板块成份股抓取；正式候选必须由 selection schema-v2 精确绑定。

use std::sync::Arc;

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
    /// 历史板块联动来源的兼容标记；BR-174 正式事件级选股不再生成此来源。
    Board,
}

#[derive(Debug, Clone)]
pub struct ChainHit {
    pub chain: String,
    pub keywords: Vec<String>,
    pub logic: String,
    pub stocks: Vec<StockInfo>,
    /// 命中来源（规则 / AI / AI降级 / 历史 Board 兼容值）
    pub source: ChainSource,
    /// 产业链规则或 AI 给出的精确板块语义关键词；不得在本模块中用于模糊检索。
    pub board_keyword: String,
    /// 匹配板块的今日主力净占比(%)；None = 资金数据不可用（不臆测多空）
    pub fund_flow_pct: Option<f64>,
    /// CR-1 (review): 真实板块代码 (e.g. "BK0815"), 用于 board_rotation_daily 的 PRIMARY KEY.
    /// 历史 Board 来源必填，正式 BR-174 路径由 selection evidence 持有。
    pub board_code: Option<String>,
    /// CR-1 (review): 板块真实涨幅(%), 用于正确显示 "[板块联动] chg=2.5%" 而非 main_net_pct.
    /// 历史 Board 来源必填，正式 BR-174 路径由 selection evidence 持有。
    pub board_change_pct: Option<f64>,
}

/// The startup owner did not activate `config/chain.toml` rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("chain rules unavailable: config::load_all() did not activate config/chain.toml")]
pub struct ChainRulesUnavailable;

/// Typed failures from the asynchronous rule/AI mapping path.
#[derive(Debug, thiserror::Error)]
pub enum ChainMapperFailure {
    #[error(transparent)]
    ChainRulesUnavailable(#[from] ChainRulesUnavailable),
    #[error("chain mapper AI worker join failed: {0}")]
    AiWorkerJoin(#[source] tokio::task::JoinError),
}

fn normalize_chain_rules(
    mut rules: Vec<(Vec<String>, String, String, String, u32, bool)>,
) -> Vec<(Vec<String>, String, String, String, u32, bool)> {
    rules.sort_by_key(|rule| std::cmp::Reverse(rule.4));
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

type ChainRuleTuple = (Vec<String>, String, String, String, u32, bool);

/// Returns the startup-activated rule snapshot in priority order.
fn chain_rules() -> Result<Vec<ChainRuleTuple>, ChainRulesUnavailable> {
    let config_rules = crate::config::get_chain_rules().ok_or(ChainRulesUnavailable)?;
    log_disabled_themes(&config_rules);
    Ok(normalize_chain_rules(map_chain_rules(config_rules)))
}

/// BR-006: 启动时单次打印被关停的主题, 便于 audit.
/// 只在消费启动时已激活的内存快照时打印。
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
pub fn map_news_to_chains(title: &str) -> Result<Vec<ChainHit>, ChainRulesUnavailable> {
    let mut hits: Vec<ChainHit> = Vec::new();
    let rules = chain_rules()?;

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
    Ok(hits)
}

/// Returns whether a deterministic rule hit came from a generic fallback rule.
///
/// `ChainHit` predates the `generic` configuration flag, so resolve it against
/// the same canonical rule table instead of guessing from display text.
pub fn is_generic_rule_hit(hit: &ChainHit) -> Result<bool, ChainRulesUnavailable> {
    Ok(hit.source == ChainSource::Rule
        && chain_rules()?
            .iter()
            .find(|(_, chain, _, _, _, _)| chain == &hit.chain)
            .map(|(_, _, _, _, _, generic)| *generic)
            .unwrap_or(false))
}

/// 新闻 → 产业链（规则优先，未命中则 AI 兜底）。
///
/// 决策（v8）：仅在关键词规则未命中时才调 AI，节省 token。
/// v9 改进：规则命中结果过于单一时（只有1条且来自通用规则），也调 AI 二次分类。
/// 数据红线 2.1/2.2：AI 不可用 → 返回空，**不编造产业链**。
/// AI worker 无法完成时返回 typed failure，不得伪装为零命中。
pub async fn map_news_to_chains_ai(titles: &[String]) -> Result<Vec<ChainHit>, ChainMapperFailure> {
    let combined = titles.join(" ");
    let rule_hits = map_news_to_chains(&combined)?;
    let rules = chain_rules()?;

    // 规则命中结果过于单一（仅命中 generic=true 的规则）时，调 AI 二次分类。
    let should_call_ai = rule_hits.len() == 1 && {
        let chain_name = &rule_hits[0].chain;
        rules
            .iter()
            .any(|(_, chain, _, _, _, generic)| chain == chain_name && *generic)
    };

    if !rule_hits.is_empty() && !should_call_ai {
        return Ok(rule_hits); // 规则命中且不需要二次分类
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
    let worker = tokio::task::spawn_blocking(move || {
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
                .call_api_mode(
                    &prompt,
                    "你是A股产业链分析师,只输出格式化结果",
                    crate::analyzer::AgentMode::Quick,
                )
                .await
            {
                Ok(t) => {
                    let ai_hits = parse_ai_chains(&t);
                    if ai_hits.is_empty() {
                        log::info!("[ChainMapper] AI 未发现新产业链，保留规则命中结果");
                        rule_hits
                    } else {
                        log::info!(
                            "[ChainMapper] AI 发现 {} 条新产业链，合并规则结果",
                            ai_hits.len()
                        );
                        // 合并规则命中和 AI 结果，去重
                        let mut merged = rule_hits;
                        for ai_hit in ai_hits {
                            if !merged.iter().any(|h| h.chain == ai_hit.chain) {
                                merged.push(ai_hit);
                            }
                        }
                        merged
                    }
                }
                Err(e) => {
                    log::warn!("[ChainMapper] AI 调用失败: {} → [AI降级]", e);
                    rule_hits
                }
            }
        })
    });
    await_ai_worker(worker).await
}

async fn await_ai_worker(
    worker: tokio::task::JoinHandle<Vec<ChainHit>>,
) -> Result<Vec<ChainHit>, ChainMapperFailure> {
    worker.await.map_err(ChainMapperFailure::AiWorkerJoin)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn with_repository_rules(
        test: impl FnOnce() -> Result<Vec<ChainHit>, ChainRulesUnavailable>,
    ) -> Vec<ChainHit> {
        let parsed: crate::config::ChainRulesFile =
            toml::from_str(include_str!("../../config/chain.toml"))
                .expect("repository chain rules must parse");
        let _snapshot = crate::config::replace_chain_rules_for_test(Some(parsed.rules));
        test().expect("explicit in-memory chain-rule snapshot must be available")
    }

    #[test]
    fn chain_rules_unavailable_is_not_reported_as_zero_matches() {
        let _snapshot = crate::config::replace_chain_rules_for_test(None);

        let error = map_news_to_chains("今日天气晴朗适合出游")
            .expect_err("missing startup-owned rules must be explicit");

        assert_eq!(error, ChainRulesUnavailable);
    }

    #[test]
    fn generic_rule_lookup_preserves_chain_rules_unavailable() {
        let _snapshot = crate::config::replace_chain_rules_for_test(None);
        let hit = ChainHit {
            chain: "TEST_CODE_CHAIN".into(),
            keywords: vec!["TEST_CODE_KEYWORD".into()],
            logic: "TEST_CODE_LOGIC".into(),
            stocks: Vec::new(),
            source: ChainSource::Rule,
            board_keyword: "TEST_CODE_BOARD".into(),
            fund_flow_pct: None,
            board_code: None,
            board_change_pct: None,
        };

        assert_eq!(
            is_generic_rule_hit(&hit).expect_err("generic lookup requires activated rules"),
            ChainRulesUnavailable
        );
    }

    #[tokio::test]
    async fn async_mapper_returns_before_ai_when_chain_rules_are_unavailable() {
        let _snapshot = crate::config::replace_chain_rules_for_test(None);

        let error = map_news_to_chains_ai(&["TEST_CODE_NEWS_WITHOUT_RULES".into()])
            .await
            .expect_err("AI must not replace unavailable deterministic configuration");

        assert!(matches!(
            error,
            ChainMapperFailure::ChainRulesUnavailable(ChainRulesUnavailable)
        ));
    }

    #[tokio::test]
    async fn ai_worker_join_failure_is_typed_instead_of_becoming_zero_matches() {
        let worker = tokio::task::spawn_blocking(|| -> Vec<ChainHit> {
            panic!("TEST_CODE_FORCE_CHAIN_MAPPER_WORKER_FAILURE")
        });

        let error = await_ai_worker(worker)
            .await
            .expect_err("worker failure must remain explicit");

        assert!(matches!(error, ChainMapperFailure::AiWorkerJoin(_)));
    }

    #[test]
    fn test_pcb_news() {
        let hits = with_repository_rules(|| {
            map_news_to_chains("电子布年内第五轮提价，木林森PCB产品全线涨价20%")
        });
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chain, "AI硬件-PCB");
    }

    #[test]
    fn test_multi_chain_news() {
        // 修复 C-3 (2026-06-29 codex review): 恢复强断言 — BR-002 spec 例外条款
        // 说"AI 给出 ≥2 条独立产业链可保留", 单元测试应覆盖**至少 1 条**,
        // 不能退化为"随便命中 0 或 1 条". 当前 chain.toml: PCB priority=100
        // (toml 中排在 MLCC 之前), MLCC priority=100, 按 priority 降序 + toml 顺序
        // PCB 应胜出, MLCC 应被互斥排除.
        let hits =
            with_repository_rules(|| map_news_to_chains("MLCC突破带动PCB和半导体产业链全线走强"));
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
        let hits = with_repository_rules(|| map_news_to_chains("今日天气晴朗适合出游"));
        assert!(hits.is_empty());
    }

    #[test]
    fn test_city_renewal() {
        let hits = with_repository_rules(|| {
            map_news_to_chains("国务院通过城市更新十五五规划，地下管网改造加速")
        });
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chain, "城市更新");
    }

    #[test]
    fn test_rule_hit_marks_source_rule() {
        let hits = with_repository_rules(|| map_news_to_chains("电子布提价带动PCB涨价"));
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
        let parsed: crate::config::ChainRulesFile =
            toml::from_str(include_str!("../../config/chain.toml"))
                .expect("repository chain rules must parse");
        let _snapshot = crate::config::replace_chain_rules_for_test(Some(parsed.rules));
        let hits = map_news_to_chains_ai(&["PCB全线涨价20%".to_string()])
            .await
            .expect("explicit in-memory chain-rule snapshot must be available");
        assert!(!hits.is_empty());
        assert_eq!(hits[0].source, ChainSource::Rule);
    }

    #[test]
    fn test_hbm_news() {
        let hits = with_repository_rules(|| {
            map_news_to_chains("SK海力士HBM3E量产供货英伟达，高带宽内存需求爆发")
        });
        assert!(hits.iter().any(|h| h.chain == "HBM-高带宽内存"));
    }

    #[test]
    fn test_commercial_aerospace() {
        let hits = with_repository_rules(|| {
            map_news_to_chains("千帆星座第二批卫星成功发射，商业航天低轨组网加速")
        });
        assert!(hits.iter().any(|h| h.chain == "商业航天"));
    }

    #[test]
    fn test_solid_state_battery_separate_from_lithium() {
        // 固态电池应命中独立规则，而非笼统的 新能源-锂电池
        let hits =
            with_repository_rules(|| map_news_to_chains("丰田宣布硫化物固态电池2027年量产装车"));
        assert!(hits.iter().any(|h| h.chain == "新能源-固态电池"));
    }

    #[test]
    fn test_smart_driving_news() {
        let hits =
            with_repository_rules(|| map_news_to_chains("特斯拉FSD入华获批，端到端智驾加速落地"));
        assert!(hits.iter().any(|h| h.chain == "智能驾驶"));
    }

    #[test]
    fn test_board_keyword_stored_for_rule_hits() {
        // v2: rule 来源的 hit 应直接携带 board_keyword，不再依赖 resolve 阶段二次查表
        // BR-006 (2026-06-29): AI硬件-CPO 已关停, 改用 AI硬件-PCB (已加权到 priority 95)
        let hits =
            with_repository_rules(|| map_news_to_chains("PCB全线涨价20%，HDI高多层板持续紧缺"));
        let pcb_hit = hits.iter().find(|h| h.chain == "AI硬件-PCB").unwrap();
        assert_eq!(pcb_hit.board_keyword, "PCB");
        assert!(!pcb_hit.board_keyword.is_empty());
    }

    #[test]
    fn test_new_energy_hydrogen() {
        let hits =
            with_repository_rules(|| map_news_to_chains("绿氢项目批量获批，PEM电解槽需求爆发在即"));
        assert!(hits.iter().any(|h| h.chain == "新能源-氢能"));
    }

    #[test]
    fn test_rare_earth_magnets() {
        // 修复 C-3 (2026-06-29 codex review): 恢复强断言 — 稀土永磁 BR-006 关停,
        // 不应在 map_news_to_chains 中命中. 标题含"机器人"关键词, 应只命中
        // 机器人(priority=80), 不命中关停的稀土永磁.
        let hits = with_repository_rules(|| {
            map_news_to_chains("稀土配额收紧叠加人形机器人放量，钕铁硼磁材供需缺口扩大")
        });
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
        let hits =
            with_repository_rules(|| map_news_to_chains("中国量子计算原型机实现1000量子比特突破"));
        assert!(hits.iter().any(|h| h.chain == "量子计算"));
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
        let hits =
            with_repository_rules(|| map_news_to_chains("PCB全线涨价20%，HDI高多层板持续紧缺"));
        assert!(hits.iter().any(|h| h.chain == "AI硬件-PCB"));
    }
}
