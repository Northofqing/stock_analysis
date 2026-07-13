//! v15.3 Phase D6: NewsDispatcher — impact → v14 push

use crate::news::impact::NewsImpact;
use crate::signal::market_event::Direction;

#[derive(Debug, Clone)]
pub struct NewsPush {
    pub text: String,
    pub headline: String,
    pub code: Option<String>,
    pub score: f64,
    pub direction: Direction,
}

pub fn decide(impact: &NewsImpact) -> Option<NewsPush> {
    if impact.score < 40.0 {
        return None;
    }
    Some(NewsPush {
        text: format!(
            "📊 {} | 分数 {:.0} | {} {} | 多源×{} | age {:.1}h",
            impact.name,
            impact.score,
            match impact.direction {
                Direction::Bull => "🟢",
                Direction::Bear => "🔴",
                _ => "⚪",
            },
            impact.reason,
            impact.source_count,
            impact.age_hours,
        ),
        headline: impact.reason.clone(),
        code: Some(impact.code.clone()),
        score: impact.score,
        direction: impact.direction,
    })
}

pub fn is_important(p: &NewsPush) -> bool {
    p.score >= 70.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::news::impact::{RelationType, score_event};
    use crate::signal::market_event::{Direction, EventType, MarketEvent, SourceRef};
    use chrono::{Local, Utc};

    fn mk_event(dir: Direction, strength: u8, code: &str, title: &str) -> MarketEvent {
        let now = Utc::now().with_timezone(&Local);
        MarketEvent {
            event_id: format!("test-{code}"),
            simhash: 42,
            full_title: title.into(),
            event_type: EventType::Other,
            subject: code.into(),
            object: Some(code.into()),
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
    fn test_decide_high_score_pushed() {
        let e = mk_event(Direction::Bull, 100, "000001", "长鑫 IPO");
        let imp = score_event(&e, RelationType::SelfCode, 2);
        assert!(imp.score > 70.0);
        assert!(decide(&imp).is_some());
        assert!(is_important(&decide(&imp).unwrap()));
    }

    #[test]
    fn test_decide_low_score_dropped() {
        let e = mk_event(Direction::Neutral, 10, "600519", "弱产业传闻");
        let imp = score_event(&e, RelationType::Industry, 1);
        assert!(imp.score < 40.0, "low score {}, should be < 40", imp.score);
        assert!(decide(&imp).is_none());
    }

    #[test]
    fn test_decide_mid_score_pushed_not_important() {
        let e = mk_event(Direction::Bear, 30, "300750", "新能源车政策微调");
        let imp = score_event(&e, RelationType::PolicyImpact, 1);
        let p = decide(&imp).expect("should be decided above 40 threshold");
        assert!(p.score >= 40.0, "score {} should be ≥ 40", p.score);
        assert!(p.score < 70.0, "score {} should be < 70 (info tier)", p.score);
        assert!(!is_important(&p));
    }
}
