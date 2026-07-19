//! v16.4 #3: Decision Engine 3 子层 (替代 intraday_monitor::evaluate_candidate 单函数).
//!
//! 设计 (v16.3 doc §4): 1 个 evaluate_candidate 拆 3 子层
//!   1. FeatureBuilder    — 解析 metric_json + 抽取 vol_ratio / push_subkind / ts 特征
//!   2. ScoreCalculator   — 调 8 strategy.score(input) 收集分数, 加权综合
//!   3. DecisionPolicy    — 业务规则: 早盘量能 / 时间窗 / 阈值, 出最终 Decision
//!
//! v16.4 #3 注: 3 子层独立 struct, 各单测, intraday_monitor 后续 commit 接入.

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

/// ============= Layer 1: FeatureBuilder =============
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Features {
    pub vol_ratio: f64,
    pub price_chg_pct: f64,
    pub sector: String,
    pub push_subkind: String,
    pub push_kind: String, // Fix review #15: 加 push_kind 让 Layer 3 用 features 而非 input.push_kind
    pub push_age_hours: f64,
}

pub struct FeatureBuilder;

impl FeatureBuilder {
    pub fn build(
        metric_json: &str,
        push_time: DateTime<Local>,
        now: DateTime<Local>,
    ) -> Result<Features, String> {
        if now < push_time {
            return Err("push_time 位于未来".to_string());
        }
        let m: serde_json::Value = serde_json::from_str(metric_json)
            .map_err(|error| format!("metric_json 解析失败: {error}"))?;
        if !m.is_object() {
            return Err("metric_json 必须是对象".to_string());
        }
        let finite_number = |field: &str| {
            m.get(field)
                .and_then(serde_json::Value::as_f64)
                .filter(|value| value.is_finite())
                .ok_or_else(|| format!("缺少有效指标 {field}"))
        };
        let optional_string = |field: &str| -> Result<String, String> {
            match m.get(field) {
                None | Some(serde_json::Value::Null) => Ok(String::new()),
                Some(value) => value
                    .as_str()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| format!("指标 {field} 必须是非空字符串")),
            }
        };
        let vol = finite_number("vol_ratio")?;
        let chg = m
            .get("price_chg_pct")
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite())
            .ok_or_else(|| "缺少有效指标 price_chg_pct".to_string())?;
        let sector = optional_string("sector")?;
        let sub = optional_string("push_subkind")?;
        let kind = optional_string("push_kind")?;
        let age = (now - push_time).num_seconds() as f64 / 3600.0;
        Ok(Features {
            vol_ratio: vol,
            price_chg_pct: chg,
            sector,
            push_subkind: sub,
            push_kind: kind,
            push_age_hours: age,
        })
    }
}

/// ============= Layer 2: ScoreCalculator =============
pub struct ScoreCalculator;

#[derive(Debug, Clone)]
pub struct ScoredStrategy {
    pub strategy_id: String,
    pub score: f64,
    pub reason: String,
}

impl ScoreCalculator {
    pub fn aggregate(scores: Vec<ScoredStrategy>) -> Option<ScoredStrategy> {
        scores.into_iter().max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}

/// ============= Layer 3: DecisionPolicy =============
pub struct DecisionPolicy;

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Approve {
        score: f64,
        strategy: String,
        reason: String,
    },
    Reject {
        reason: String,
    },
}

pub const DECISION_SCORE_THRESHOLD: f64 = 6.0;
pub const PUSH_AGE_MAX_HOURS: f64 = 1.0;
pub const VOLUME_SURGE_MIN: f64 = 5.0;

impl DecisionPolicy {
    pub fn decide(features: &Features, scored: Option<ScoredStrategy>) -> Decision {
        if features.push_subkind == "AuctionVolume" && features.vol_ratio < VOLUME_SURGE_MIN {
            return Decision::Reject {
                reason: format!("早盘量能不足 vol={}", features.vol_ratio),
            };
        }
        if features.push_age_hours > PUSH_AGE_MAX_HOURS {
            return Decision::Reject {
                reason: format!(
                    "推送超 {}h (实际 {:.1}h)",
                    PUSH_AGE_MAX_HOURS, features.push_age_hours
                ),
            };
        }
        match scored {
            Some(s) if s.score >= DECISION_SCORE_THRESHOLD => Decision::Approve {
                score: s.score,
                strategy: s.strategy_id,
                reason: s.reason,
            },
            Some(s) => Decision::Reject {
                reason: format!(
                    "综合分 {:.1} < 阈值 {:.1}",
                    s.score, DECISION_SCORE_THRESHOLD
                ),
            },
            None => Decision::Reject {
                reason: "无 strategy 命中".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    fn make_features(vol: f64, age: f64, sub: &str) -> Features {
        Features {
            vol_ratio: vol,
            price_chg_pct: 0.0,
            sector: "test".to_string(),
            push_subkind: sub.to_string(),
            push_kind: "Momentum".to_string(),
            push_age_hours: age,
        }
    }

    fn make_scored(score: f64) -> ScoredStrategy {
        ScoredStrategy {
            strategy_id: "Momentum".to_string(),
            score,
            reason: "test".to_string(),
        }
    }

    #[test]
    fn feature_builder_parses_metric_json() {
        let now = Local::now();
        let metric = r#"{"vol_ratio": 6.5, "price_chg_pct": 1.5, "sector": "AI", "push_subkind": "Momentum"}"#;
        let f = FeatureBuilder::build(metric, now, now).expect("valid metrics");
        assert_eq!(f.vol_ratio, 6.5);
        assert_eq!(f.push_subkind, "Momentum");
    }

    #[test]
    fn feature_builder_rejects_future_and_non_object_evidence() {
        let now = Local::now();
        assert!(FeatureBuilder::build("{}", now + chrono::Duration::seconds(1), now).is_err());
        assert!(FeatureBuilder::build("[]", now, now).is_err());
    }

    #[test]
    fn feature_builder_handles_bad_json() {
        let now = Local::now();
        let error = FeatureBuilder::build("not json", now, now).expect_err("invalid JSON");
        assert!(error.contains("metric_json 解析失败"));
    }

    #[test]
    fn score_calculator_picks_max() {
        let scores = vec![make_scored(5.0), make_scored(8.0), make_scored(7.0)];
        let best = ScoreCalculator::aggregate(scores).unwrap();
        assert_eq!(best.score, 8.0);
    }

    #[test]
    fn score_calculator_empty_returns_none() {
        let best = ScoreCalculator::aggregate(vec![]);
        assert!(best.is_none());
    }

    #[test]
    fn decision_approves_high_score_fresh_push() {
        let f = make_features(8.0, 0.5, "Momentum");
        let s = make_scored(8.0);
        let d = DecisionPolicy::decide(&f, Some(s));
        assert!(matches!(d, Decision::Approve { .. }));
    }

    #[test]
    fn decision_rejects_old_push() {
        let f = make_features(8.0, 2.0, "Momentum");
        let s = make_scored(8.0);
        let d = DecisionPolicy::decide(&f, Some(s));
        assert!(matches!(d, Decision::Reject { .. }));
    }

    #[test]
    fn decision_rejects_low_score() {
        let f = make_features(8.0, 0.5, "Momentum");
        let s = make_scored(5.0);
        let d = DecisionPolicy::decide(&f, Some(s));
        assert!(matches!(d, Decision::Reject { .. }));
    }

    #[test]
    fn decision_rejects_low_volume_auction() {
        let f = make_features(2.0, 0.5, "AuctionVolume");
        let s = make_scored(8.0);
        let d = DecisionPolicy::decide(&f, Some(s));
        assert!(matches!(d, Decision::Reject { .. }));
    }
}
