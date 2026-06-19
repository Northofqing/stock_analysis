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

    for pos in holdings {
        let mut best_hit: Option<&ChainHit> = None;

        // 找到该持仓所属的产业链
        for hit in hits {
            if hit.stocks.iter().any(|s| s.code == pos.code) {
                match best_hit {
                    None => best_hit = Some(hit),
                    Some(prev) if hit.keywords.len() > prev.keywords.len() => {
                        best_hit = Some(hit); // 关键词越多匹配越强
                    }
                    _ => {}
                }
            }
        }

        match best_hit {
            Some(hit) => {
                let (direction, reason) = match hit.fund_flow_pct {
                    Some(flow) if flow >= FLOW_POSITIVE_PCT => (
                        ImpactDirection::Positive,
                        format!("{}：{}（主力净占比+{:.1}%）", hit.chain, hit.logic, flow),
                    ),
                    Some(flow) if flow <= FLOW_NEGATIVE_PCT => (
                        ImpactDirection::Negative,
                        format!("{}：消息利好但主力净流出{:.1}%，资金背离", hit.chain, flow),
                    ),
                    Some(flow) => (
                        ImpactDirection::Neutral,
                        format!("{}：资金平淡（主力净占比{:.1}%）", hit.chain, flow),
                    ),
                    None => (
                        ImpactDirection::Neutral,
                        format!("{}：{}（资金数据不足）", hit.chain, hit.logic),
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
}
