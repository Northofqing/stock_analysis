//! v15.3 Phase D4: Impact analyzer — 打分 + 方向 + 多源共振

use crate::signal::market_event::{Direction, MarketEvent};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelationType {
    SelfCode,
    SupplyChain,
    Industry,
    PolicyImpact,
    AnalystView,
    EarningsRef,
}

impl RelationType {
    pub fn base_score(&self) -> f64 {
        match self {
            Self::SelfCode => 100.0,
            Self::AnalystView => 85.0,
            Self::EarningsRef => 80.0,
            Self::SupplyChain => 70.0,
            Self::PolicyImpact => 60.0,
            Self::Industry => 30.0,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::SelfCode => "self",
            Self::AnalystView => "analyst_view",
            Self::EarningsRef => "earnings",
            Self::SupplyChain => "supply_chain",
            Self::PolicyImpact => "policy_impact",
            Self::Industry => "industry",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsImpact {
    pub code: String,
    pub name: String,
    pub score: f64,
    pub direction: Direction,
    pub reason: String,
    pub source_count: u8,
    pub age_hours: f64,
    pub relation: RelationType,
}

pub fn direction_bonus(dir: Direction) -> f64 {
    match dir {
        Direction::Bull => 10.0,
        Direction::Bear => -10.0,
        _ => 0.0,
    }
}

pub fn importance_bonus(strength: u8) -> f64 {
    (strength as f64 / 10.0).min(10.0) * 5.0
}

pub fn source_weight_bonus(source_count: u8) -> f64 {
    ((source_count as f64 - 1.0).max(0.0) * 10.0).min(30.0)
}

pub fn decay_factor(age_hours: f64) -> f64 {
    (-age_hours / 24.0).exp()
}

pub fn score_event(event: &MarketEvent, relation: RelationType, source_count: u8) -> NewsImpact {
    let now = Utc::now();
    let age_hours = (now - event.occurred_at.with_timezone(&Utc)).num_minutes() as f64 / 60.0;
    let base = relation.base_score();
    let dir_b = direction_bonus(event.direction);
    let imp_b = importance_bonus(event.strength);
    let src_w = source_weight_bonus(source_count);
    let decay = decay_factor(age_hours);
    let confidence = event.certainty as f64 / 100.0;
    let score = (base + dir_b + imp_b + src_w) * decay * confidence;

    NewsImpact {
        code: event.subject.clone(),
        name: event.subject.clone(),
        score,
        direction: event.direction,
        reason: format!("{}:{}", relation.label(), event.full_title),
        source_count,
        age_hours,
        relation,
    }
}

pub fn deduplicate_by_event_hash(impacts: Vec<NewsImpact>) -> Vec<NewsImpact> {
    let mut by_code: HashMap<String, NewsImpact> = HashMap::new();
    for imp in impacts {
        let entry = by_code.entry(imp.code.clone()).or_insert_with(|| imp.clone());
        entry.source_count = entry.source_count.max(imp.source_count).saturating_add(1);
        if imp.score > entry.score {
            *entry = imp;
        }
    }
    by_code.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::market_event::{Direction, EventType, MarketEvent, SourceRef};
    use chrono::Local;

    fn mk_event(dir: Direction, strength: u8, code: &str) -> MarketEvent {
        let now = Utc::now().with_timezone(&Local);
        MarketEvent {
            event_id: format!("test-{code}"),
            simhash: 42,
            full_title: format!("title {code}"),
            event_type: EventType::Other,
            subject: code.to_string(),
            object: Some(code.to_string()),
            direction: dir,
            strength,
            certainty: 80,
            chains: vec![],
            occurred_at: now,
            provenance: vec![SourceRef { provider: "test".into(), url: None, fetched_at: now }],
            ai_degraded: false,
            stale: false,
        }
    }

    #[test]
    fn test_base_score_self_highest() {
        assert!(RelationType::SelfCode.base_score() > RelationType::SupplyChain.base_score());
        assert!(RelationType::SupplyChain.base_score() > RelationType::Industry.base_score());
    }

    #[test]
    fn test_direction_bonus_signs() {
        assert!(direction_bonus(Direction::Bull) > 0.0);
        assert!(direction_bonus(Direction::Bear) < 0.0);
        assert_eq!(direction_bonus(Direction::Neutral), 0.0);
    }

    #[test]
    fn test_score_event_bull_self() {
        let e = mk_event(Direction::Bull, 100, "000001");
        let imp = score_event(&e, RelationType::SelfCode, 1);
        assert!(imp.score > 100.0, "self bull should exceed 100, got {}", imp.score);
    }

    #[test]
    fn test_source_weight_resonance() {
        let e = mk_event(Direction::Neutral, 50, "600519");
        let alone = score_event(&e, RelationType::SupplyChain, 1);
        let multi = score_event(&e, RelationType::SupplyChain, 3);
        assert!(multi.score > alone.score, "3 sources should score higher than 1");
    }

    #[test]
    fn test_dedup_merges_sources() {
        let e1 = mk_event(Direction::Bull, 80, "000001");
        let e2 = mk_event(Direction::Bull, 80, "000001");
        let i1 = score_event(&e1, RelationType::SelfCode, 1);
        let i2 = score_event(&e2, RelationType::SelfCode, 1);
        let merged = deduplicate_by_event_hash(vec![i1, i2]);
        assert_eq!(merged.len(), 1);
        assert!(merged[0].source_count >= 2, "merged source_count should grow, got {}", merged[0].source_count);
    }
}
