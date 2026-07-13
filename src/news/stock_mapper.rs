//! v15.3 D3: Stock Mapper — MarketEvent → 候选股票代码

use crate::news::ipo::supply_chain::RelationType as IpoRel;
use crate::signal::market_event::{EventType, MarketEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Relation {
    SelfCode,
    SupplyChain,
    Industry,
    PolicyImpact,
    AnalystView,
    EarningsRef,
}

#[derive(Debug, Clone)]
pub struct StockHit {
    pub code: String,
    pub name: String,
    pub relation: Relation,
    pub confidence: f64,
    pub source: String,
}

pub fn relation_for_event_type(et: EventType) -> Relation {
    match et {
        EventType::Earnings => Relation::EarningsRef,
        EventType::MarketAction => Relation::SelfCode,
        EventType::AnalystView => Relation::AnalystView,
        EventType::Policy | EventType::Announcement => Relation::PolicyImpact,
        _ => Relation::Industry,
    }
}

pub fn map_unlisted_to_listed(title: &str) -> Vec<StockHit> {
    let mut hits = Vec::new();
    for company in crate::news::ipo::supply_chain::ipo_companies() {
        if title.contains(company.pre_ipo_name) {
            for (code, name, rel) in company.related_stocks {
                let conf = match rel {
                    IpoRel::Shareholder => 0.85,
                    IpoRel::Supplier => 0.75,
                    IpoRel::Customer => 0.65,
                    IpoRel::Partner => 0.55,
                };
                hits.push(StockHit {
                    code: code.to_string(),
                    name: name.to_string(),
                    relation: Relation::SupplyChain,
                    confidence: conf,
                    source: format!("ipo_supply_chain:{}", company.pre_ipo_name),
                });
            }
        }
    }
    hits
}

pub fn map_industry_by_keyword(title: &str) -> Vec<StockHit> {
    let mut hits = Vec::new();
    let kw_pairs: Vec<(String, String)> = {
        // 复用 config::MonitorConfig (v15.1 B2.1)
        // 注意: industry_keywords 字段是 v15.1 plan, 若 config 还没加, 退化为 hardcoded
        let _ = crate::config::get_monitor_config();
        vec![
            ("存储".into(), "存储/DRAM/NAND/内存/长鑫/长存/兆易".into()),
            ("机器人".into(), "机器人/人形机器人/宇树/优必选/智元".into()),
            ("半导体".into(), "半导体/芯片/晶圆/封测/HBM/先进封装".into()),
            ("光伏".into(), "光伏/硅料/硅片/电池片/组件".into()),
            ("新能源车".into(), "新能源车/锂电池/正极材料/隔膜/电解液".into()),
        ]
    };
    for (name, kws) in &kw_pairs {
        if kws.split('/').any(|k| title.contains(k)) {
            hits.push(StockHit {
                code: format!("CHAIN:{name}"),
                name: format!("{name}板块"),
                relation: Relation::Industry,
                confidence: 0.4,
                source: format!("bom_kb:{name}"),
            });
        }
    }
    hits
}

pub fn map_subject(event: &MarketEvent) -> Vec<StockHit> {
    if event.subject.is_empty() || event.subject.len() != 6 || !event.subject.chars().all(|c| c.is_ascii_digit()) {
        return vec![];
    }
    vec![StockHit {
        code: event.subject.clone(),
        name: event.subject.clone(),
        relation: Relation::SelfCode,
        confidence: 0.95,
        source: format!("self:{:?}", event.event_type),
    }]
}

pub fn map_all(event: &MarketEvent) -> Vec<StockHit> {
    let mut hits = map_subject(event);
    hits.extend(map_unlisted_to_listed(&event.full_title));
    hits.extend(map_industry_by_keyword(&event.full_title));
    hits.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
    hits.dedup_by(|a, b| a.code == b.code && a.relation == b.relation);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::market_event::{Direction, EventType, MarketEvent, SourceRef};
    use chrono::{Local, Utc};

    fn mk_event(title: &str, subject: &str, et: EventType) -> MarketEvent {
        let now = Utc::now().with_timezone(&Local);
        MarketEvent {
            event_id: format!("test-{subject}"),
            simhash: 42,
            full_title: title.into(),
            event_type: et,
            subject: subject.into(),
            object: Some(subject.into()),
            direction: Direction::Neutral,
            strength: 70,
            certainty: 80,
            chains: vec![],
            occurred_at: now,
            provenance: vec![SourceRef { provider: "test".into(), url: None, fetched_at: now }],
            ai_degraded: false,
            stale: false,
        }
    }

    #[test]
    fn test_relation_for_event_type() {
        assert_eq!(relation_for_event_type(EventType::Earnings), Relation::EarningsRef);
        assert_eq!(relation_for_event_type(EventType::AnalystView), Relation::AnalystView);
        assert_eq!(relation_for_event_type(EventType::Policy), Relation::PolicyImpact);
    }

    #[test]
    fn test_map_unlisted_changxin_to_zhaoyi() {
        let hits = map_unlisted_to_listed("长鑫存储递交招股说明书");
        assert!(!hits.is_empty());
        let zhao = hits.iter().find(|h| h.name == "兆易创新").expect("兆易创新必须命中");
        assert_eq!(zhao.relation, Relation::SupplyChain);
        assert!(zhao.confidence >= 0.7);
    }

    #[test]
    fn test_map_subject_self() {
        let e = mk_event("茅台发布年报", "600519", EventType::Earnings);
        let hits = map_subject(&e);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].code, "600519");
        assert_eq!(hits[0].relation, Relation::SelfCode);
    }

    #[test]
    fn test_map_subject_rejects_non6digit() {
        let e = mk_event("test", "abc", EventType::Earnings);
        assert!(map_subject(&e).is_empty());
    }

    #[test]
    fn test_industry_keyword_hits() {
        let hits = map_industry_by_keyword("半导体行业突破");
        assert!(!hits.is_empty(), "半导体 keyword 必须命中");
    }
}
