//! 持仓影响评估 — 每条新闻 × 每只持仓 → 利好/中性/利空。

use crate::portfolio::Position;
use super::chain_mapper::ChainHit;

#[derive(Debug, Clone)]
pub struct PositionImpact {
    pub code: String,
    pub name: String,
    pub direction: ImpactDirection,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImpactDirection { Positive, Neutral, Negative }

impl ImpactDirection {
    pub fn emoji(&self) -> &'static str {
        match self {
            ImpactDirection::Positive => "✅",
            ImpactDirection::Neutral => "→",
            ImpactDirection::Negative => "⚠️",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ImpactDirection::Positive => "利好",
            ImpactDirection::Neutral => "中性",
            ImpactDirection::Negative => "利空",
        }
    }
}

/// 资金流向阈值（今日主力净占比 %）：≥ 强流入视为利好，≤ 强流出视为利空。
const FLOW_POSITIVE_PCT: f64 = 2.0;
const FLOW_NEGATIVE_PCT: f64 = -2.0;

fn normalize_text(s: &str) -> String {
    s.to_lowercase().replace(' ', "")
}

fn concept_alignment_score(hit: &ChainHit, concepts: &[String]) -> usize {
    if concepts.is_empty() {
        return 0;
    }

    let chain_text = normalize_text(&hit.chain);
    let logic_text = normalize_text(&hit.logic);
    let keyword_texts: Vec<String> = hit.keywords.iter().map(|k| normalize_text(k)).collect();

    let mut score = 0usize;
    for c in concepts {
        let concept = normalize_text(c);
        if concept.len() < 2 {
            continue;
        }

        // 强匹配：概念名与 chain / 关键词直接包含关系。
        if chain_text.contains(&concept)
            || concept.contains(&chain_text)
            || keyword_texts.iter().any(|k| k.contains(&concept) || concept.contains(k))
        {
            score += 2;
            continue;
        }

        // 弱匹配：仅在逻辑文案中出现。
        if logic_text.contains(&concept) || concept.contains(&logic_text) {
            score += 1;
        }
    }

    score
}

fn concept_preferred_chain(concepts: &[String]) -> Option<&'static str> {
    let concept_texts: Vec<String> = concepts.iter().map(|c| normalize_text(c)).collect();

    let rules: &[(&[&str], &str)] = &[
        (&["pcb", "印制电路板", "hdi", "封装基板", "覆铜板", "电子布", "玻纤布", "玻纤纱"], "AI硬件-PCB"),
        (&["mlcc", "多层陶瓷电容", "被动元件", "陶瓷电容"], "AI硬件-MLCC"),
        (&["光模块", "光通信", "光纤", "cpo", "硅光"], "AI硬件-CPO"),
        (&["智能驾驶", "无人驾驶", "自动驾驶", "智驾", "l3自动驾驶", "l4自动驾驶"], "智能驾驶"),
        (&["固态电池", "半固态电池", "全固态电池"], "新能源-固态电池"),
        (&["钠离子电池", "钠电", "钠电池"], "新能源-钠离子电池"),
        (&["锂电池", "磷酸铁锂", "三元锂"], "新能源-锂电池"),
        (&["创新药", "cxo", "cro", "cdmo", "adc", "双抗", "glp-1"], "创新药-CXO"),
        (&["军工", "军工电子", "军用芯片", "相控阵雷达"], "军工电子"),
        (&["可控核聚变", "核聚变", "托卡马克"], "可控核聚变"),
        (&["消费电子", "智能手机", "折叠屏", "摄像头模组", "cis", "fpc"], "消费电子"),
    ];

    for (needles, chain) in rules {
        if needles.iter().any(|needle| concept_texts.iter().any(|c| c.contains(needle))) {
            return Some(*chain);
        }
    }
    None
}

fn select_best_hit<'a>(
    hits: &'a [ChainHit],
    code: &str,
    concepts: &[String],
) -> Option<&'a ChainHit> {
    let preferred_chain = concept_preferred_chain(concepts);
    let mut best: Option<(&ChainHit, usize, usize)> = None;

    for hit in hits {
        if !hit.stocks.iter().any(|s| s.code == code) {
            continue;
        }

        if let Some(chain) = preferred_chain {
            if hit.chain == chain {
                return Some(hit);
            }
        }

        let alignment = concept_alignment_score(hit, concepts);
        let kw_score = hit.keywords.len();

        match best {
            None => best = Some((hit, alignment, kw_score)),
            Some((_, best_align, best_kw))
                if alignment > best_align || (alignment == best_align && kw_score > best_kw) =>
            {
                best = Some((hit, alignment, kw_score));
            }
            _ => {}
        }
    }

    best.map(|(hit, _, _)| hit)
}

fn build_static_chain_hit(chain: &str, concepts: &[String]) -> Option<ChainHit> {
    let chain = chain.to_string();
    // 与 chain_rules.toml 保持同源：通过 chain_mapper 规则实时反查 board_keyword。
    let board_keyword = crate::opportunity::chain_mapper::map_news_to_chains(&chain)
        .into_iter()
        .find(|h| h.chain == chain)
        .map(|h| h.board_keyword)
        .unwrap_or_default();

    if board_keyword.is_empty() {
        return None;
    }

    let matched_keywords: Vec<String> = concepts
        .iter()
        .filter_map(|c| {
            let norm = normalize_text(c);
            if norm.contains(&normalize_text(&board_keyword)) || normalize_text(&board_keyword).contains(&norm) {
                Some(c.clone())
            } else {
                None
            }
        })
        .collect();

    if matched_keywords.is_empty() {
        return None;
    }

    Some(ChainHit {
        chain,
        keywords: matched_keywords,
        logic: "静态主业概念归因".to_string(),
        stocks: Vec::new(),
        source: crate::opportunity::chain_mapper::ChainSource::AiDegraded,
        board_keyword,
        fund_flow_pct: None,
    })
}

fn load_cached_concepts_safe(max_age_days: i64) -> std::collections::HashMap<String, Vec<String>> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::database::DatabaseManager::get().get_cached_concepts(max_age_days)
    }))
    .unwrap_or_default()
}

/// 评估新闻对持仓的影响。
///
/// 三态判定（数据红线 2.2：缺数据不臆测）：
/// - 持仓在产业链命中标的中 + 板块资金流入 → 利好
/// - 持仓在命中标的中 + 板块资金大幅流出 → 利空（消息与资金背离）
/// - 在命中标的中但资金平淡 → 中性
/// - 在命中标的中但**无资金数据** → 中性·数据不足（不臆测多空）
/// - 不在任何命中标的中 → 中性·无直接产业链关联
pub fn assess_impact(hits: &[ChainHit], holdings: &[Position]) -> Vec<PositionImpact> {
    let mut results = Vec::new();
    // 静态概念标签兜底：用于同股多链命中时优先选择更贴近主业的产业链。
    let concepts_map = load_cached_concepts_safe(14);

    enum SelectedHit<'a> {
        Borrowed(&'a ChainHit),
        Owned(ChainHit),
    }

    impl<'a> SelectedHit<'a> {
        fn chain(&self) -> &str {
            match self {
                SelectedHit::Borrowed(hit) => &hit.chain,
                SelectedHit::Owned(hit) => &hit.chain,
            }
        }

        fn logic(&self) -> &str {
            match self {
                SelectedHit::Borrowed(hit) => &hit.logic,
                SelectedHit::Owned(hit) => &hit.logic,
            }
        }

        fn fund_flow_pct(&self) -> Option<f64> {
            match self {
                SelectedHit::Borrowed(hit) => hit.fund_flow_pct,
                SelectedHit::Owned(hit) => hit.fund_flow_pct,
            }
        }
    }

    for pos in holdings {
        let concepts = concepts_map.get(&pos.code).cloned().unwrap_or_default();
        let preferred_chain = concept_preferred_chain(&concepts);
        let static_hit = concept_preferred_chain(&concepts)
            .and_then(|chain| build_static_chain_hit(chain, &concepts));

        // 防误判：当持仓有主业概念标签时，动态命中的产业链必须与概念有对齐。
        // 否则容易把“板块成分股中的边缘标的”误判为直接受益标的。
        let borrowed_hit = select_best_hit(hits, &pos.code, &concepts).filter(|hit| {
            if concepts.is_empty() {
                // 无概念标签时维持旧行为，避免信息完全丢失。
                return true;
            }

            let align = concept_alignment_score(hit, &concepts);
            if align > 0 {
                return true;
            }

            match preferred_chain {
                Some(chain) => hit.chain == chain,
                None => false,
            }
        });

        let best_hit = borrowed_hit
            .map(SelectedHit::Borrowed)
            .or_else(|| static_hit.map(SelectedHit::Owned));

        match best_hit {
            Some(hit) => {
                let (direction, reason) = match hit.fund_flow_pct() {
                    Some(flow) if flow >= FLOW_POSITIVE_PCT => (
                        ImpactDirection::Positive,
                        format!("{}：{}（主力净占比+{:.1}%）", hit.chain(), hit.logic(), flow),
                    ),
                    Some(flow) if flow <= FLOW_NEGATIVE_PCT => (
                        ImpactDirection::Negative,
                        format!("{}：消息利好但主力净流出{:.1}%，资金背离", hit.chain(), flow),
                    ),
                    Some(flow) => (
                        ImpactDirection::Neutral,
                        format!("{}：资金平淡（主力净占比{:.1}%）", hit.chain(), flow),
                    ),
                    None => (
                        ImpactDirection::Neutral,
                        format!("{}：{}（资金数据不足）", hit.chain(), hit.logic()),
                    ),
                };
                results.push(PositionImpact {
                    code: pos.code.clone(),
                    name: pos.name.clone(),
                    direction,
                    reason,
                });
            }
            None => {
                // 无匹配 → 中性
                results.push(PositionImpact {
                    code: pos.code.clone(),
                    name: pos.name.clone(),
                    direction: ImpactDirection::Neutral,
                    reason: "无直接产业链关联".to_string(),
                });
            }
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn si(code: &str, name: &str) -> crate::opportunity::chain_mapper::StockInfo {
        crate::opportunity::chain_mapper::StockInfo { code: code.into(), name: name.into(), change_pct: 0.0, vol_ratio: 1.0 }
    }

    fn pos(code: &str, name: &str) -> Position {
        Position {
            code: code.into(), name: name.into(),
            shares: 1000, cost_price: 10.0, hard_stop: 9.0,
            added_at: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            status: crate::portfolio::PositionStatus::Holding,
            sector: "其他".into(),
        }
    }

    fn hit_with_flow(stock: &str, name: &str, flow: Option<f64>) -> ChainHit {
        ChainHit {
            chain: "AI硬件-PCB".into(),
            keywords: vec!["PCB".into()],
            logic: "PCB涨价".into(),
            stocks: vec![si(stock, name)],
            source: super::super::chain_mapper::ChainSource::Rule,
            board_keyword: String::new(),
            fund_flow_pct: flow,
        }
    }

    #[test]
    fn test_inflow_positive() {
        let hits = vec![hit_with_flow("002579", "中京电子", Some(5.0))];
        let impacts = assess_impact(&hits, &vec![pos("002579", "中京电子")]);
        assert_eq!(impacts[0].direction, ImpactDirection::Positive);
    }

    #[test]
    fn test_outflow_negative() {
        // 消息利好但主力大幅净流出 → 利空（资金背离）
        let hits = vec![hit_with_flow("002579", "中京电子", Some(-6.0))];
        let impacts = assess_impact(&hits, &vec![pos("002579", "中京电子")]);
        assert_eq!(impacts[0].direction, ImpactDirection::Negative);
    }

    #[test]
    fn test_flat_flow_neutral() {
        let hits = vec![hit_with_flow("002579", "中京电子", Some(0.5))];
        let impacts = assess_impact(&hits, &vec![pos("002579", "中京电子")]);
        assert_eq!(impacts[0].direction, ImpactDirection::Neutral);
    }

    #[test]
    fn test_missing_flow_neutral_not_assumed() {
        // 缺资金数据 → 中性·数据不足，绝不臆测为利好/利空
        let hits = vec![hit_with_flow("002579", "中京电子", None)];
        let impacts = assess_impact(&hits, &vec![pos("002579", "中京电子")]);
        assert_eq!(impacts[0].direction, ImpactDirection::Neutral);
        assert!(impacts[0].reason.contains("数据不足"));
    }

    #[test]
    fn test_unrelated_holding_neutral() {
        let hits = vec![hit_with_flow("002579", "中京电子", Some(5.0))];
        let holdings = vec![pos("000813", "德展健康")];
        let impacts = assess_impact(&hits, &holdings);
        assert_eq!(impacts[0].direction, ImpactDirection::Neutral);
        assert!(impacts[0].reason.contains("无直接产业链关联"));
    }

    #[test]
    fn test_concept_priority_prefers_pcb_chain() {
        let hits = vec![
            ChainHit {
                chain: "AI硬件-CPO".into(),
                keywords: vec!["光模块".into(), "CPO".into(), "硅光".into()],
                logic: "光互联升级".into(),
                stocks: vec![si("002579", "中京电子")],
                source: super::super::chain_mapper::ChainSource::Rule,
                board_keyword: "CPO".into(),
                fund_flow_pct: Some(3.0),
            },
            ChainHit {
                chain: "AI硬件-PCB".into(),
                keywords: vec!["PCB".into()],
                logic: "PCB价值量提升".into(),
                stocks: vec![si("002579", "中京电子")],
                source: super::super::chain_mapper::ChainSource::Rule,
                board_keyword: "PCB".into(),
                fund_flow_pct: Some(3.0),
            },
        ];

        let concepts = vec!["PCB概念".to_string(), "HDI".to_string()];
        let best = select_best_hit(&hits, "002579", &concepts).unwrap();
        assert_eq!(best.chain, "AI硬件-PCB");
    }
}
