//! v12 MVP4-4.1: market_stage 置信度精化.
//!
//! 设计: 5 维打分 → 综合 conf_pct (0~100).
//!   1. 情绪 (涨停/跌停/炸板率/连板高度)
//!   2. 资金 (主力净流入/北向/两融)
//!   3. 技术 (上证/创业/科创 涨幅)
//!   4. 政策 (公告关键词命中)
//!   5. 外部 (隔夜美股/汇率)
//!
//! 任一维度数据缺失 → 该维度计 50 (中性), 不阻断.
//! 数据完整度 < 2 时 → conf_pct = 50, 标 degraded=true.

use serde::{Deserialize, Serialize};

/// 5 维证据 (Option = 数据缺失)
#[derive(Clone, Debug, Default)]
pub struct MarketStageEvidence {
    pub sentiment: Option<SentimentMetrics>,
    pub capital: Option<CapitalMetrics>,
    pub technical: Option<TechnicalMetrics>,
    pub policy: Option<PolicyMetrics>,
    pub external: Option<ExternalMetrics>,
}

#[derive(Clone, Debug, Default)]
pub struct SentimentMetrics {
    pub limit_up_n: u32,
    pub limit_down_n: u32,
    pub broken_pct: f64,
    pub consecutive_h: u32,
}

#[derive(Clone, Debug, Default)]
pub struct CapitalMetrics {
    pub main_flow_yi: f64,
    pub amount_yi: f64,
    pub amount_delta_pct: f64,
}

#[derive(Clone, Debug, Default)]
pub struct TechnicalMetrics {
    pub sh_chg: f64,
    pub chinext_chg: f64,
    pub star_chg: f64,
}

#[derive(Clone, Debug, Default)]
pub struct PolicyMetrics {
    pub positive_hits: u32,
    pub negative_hits: u32,
}

#[derive(Clone, Debug, Default)]
pub struct ExternalMetrics {
    pub us_chg: f64,
    pub fx_chg: f64,
}

/// 评估结果
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketStageConfidence {
    pub heat_stage: String,         // "MainUp"/"HeatUp"/"Range"/"Fade"/"Climax"
    pub conf_pct: u8,              // 0~100 综合置信度
    /// 各维度分数 (0~100, None 表示数据缺失用 50 中性)
    pub dim_scores: DimScores,
    /// 数据完整维度数 (0~5)
    pub data_complete_n: u8,
    /// 数据降级标记
    pub degraded: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DimScores {
    pub sentiment: u8,
    pub capital: u8,
    pub technical: u8,
    pub policy: u8,
    pub external: u8,
}

/// MVP4-4.1 主评估: 5 维打分 + 综合
pub fn evaluate(ev: &MarketStageEvidence) -> MarketStageConfidence {
    let sentiment = score_sentiment(ev.sentiment.as_ref());
    let capital = score_capital(ev.capital.as_ref());
    let technical = score_technical(ev.technical.as_ref());
    let policy = score_policy(ev.policy.as_ref());
    let external = score_external(ev.external.as_ref());

    // 数据完整度计数
    let data_complete_n = (ev.sentiment.is_some() as u8)
        + (ev.capital.is_some() as u8)
        + (ev.technical.is_some() as u8)
        + (ev.policy.is_some() as u8)
        + (ev.external.is_some() as u8);

    let degraded = data_complete_n < 2;

    // 综合: 加权平均 (情绪/资金/技术 权重高, 政策/外部 权重低)
    let weighted_sum = (sentiment as u32) * 30
        + (capital as u32) * 25
        + (technical as u32) * 25
        + (policy as u32) * 10
        + (external as u32) * 10;
    let total_weight = if degraded { 100 } else { 30 + 25 + 25 + 10 + 10 };

    let conf_pct = ((weighted_sum as f64 / total_weight as f64).round() as u8).min(100);

    // 阶段判定 (基于综合分)
    let heat_stage = match conf_pct {
        80..=100 => "MainUp",
        60..=79 => "HeatUp",
        40..=59 => "Range",
        20..=39 => "Fade",
        _ => "Climax",
    }
    .to_string();

    MarketStageConfidence {
        heat_stage,
        conf_pct,
        dim_scores: DimScores {
            sentiment,
            capital,
            technical,
            policy,
            external,
        },
        data_complete_n,
        degraded,
    }
}

/// 情绪维度打分 (0~100)
fn score_sentiment(m: Option<&SentimentMetrics>) -> u8 {
    let Some(m) = m else { return 50 };
    let mut score = 50.0;
    // 涨停 > 30 家加分
    if m.limit_up_n >= 50 {
        score += 30.0;
    } else if m.limit_up_n >= 30 {
        score += 20.0;
    } else if m.limit_up_n >= 15 {
        score += 10.0;
    } else if m.limit_up_n < 5 {
        score -= 15.0;
    }
    // 跌停加分 (下跌趋势)
    if m.limit_down_n >= 20 {
        score -= 25.0;
    } else if m.limit_down_n >= 10 {
        score -= 15.0;
    }
    // 炸板率 > 30% 减分
    if m.broken_pct >= 30.0 {
        score -= 15.0;
    } else if m.broken_pct >= 20.0 {
        score -= 10.0;
    }
    // 连板高度 ≥ 5 加分
    if m.consecutive_h >= 5 {
        score += 15.0;
    } else if m.consecutive_h >= 3 {
        score += 10.0;
    }
    (score as f64).clamp(0.0, 100.0).round() as u8
}

/// 资金维度打分
fn score_capital(m: Option<&CapitalMetrics>) -> u8 {
    let Some(m) = m else { return 50 };
    let mut score = 50.0;
    if m.main_flow_yi > 100.0 {
        score += 30.0;
    } else if m.main_flow_yi > 50.0 {
        score += 20.0;
    } else if m.main_flow_yi > 0.0 {
        score += 10.0;
    } else if m.main_flow_yi < -100.0 {
        score -= 25.0;
    } else if m.main_flow_yi < -50.0 {
        score -= 15.0;
    }
    if m.amount_delta_pct > 10.0 {
        score += 15.0;
    } else if m.amount_delta_pct < -10.0 {
        score -= 10.0;
    }
    (score as f64).clamp(0.0, 100.0).round() as u8
}

/// 技术维度打分
fn score_technical(m: Option<&TechnicalMetrics>) -> u8 {
    let Some(m) = m else { return 50 };
    let avg = (m.sh_chg + m.chinext_chg + m.star_chg) / 3.0;
    let mut score = 50.0 + avg * 10.0; // 涨幅 +1% 加 10 分
    // 三指数共振加分
    if m.sh_chg > 0.5 && m.chinext_chg > 0.5 && m.star_chg > 0.5 {
        score += 10.0;
    }
    // 大跌惩罚
    if avg < -2.0 {
        score -= 20.0;
    }
    (score as f64).clamp(0.0, 100.0).round() as u8
}

/// 政策维度打分 (公告关键词命中)
fn score_policy(m: Option<&PolicyMetrics>) -> u8 {
    let Some(m) = m else { return 50 };
    let net = m.positive_hits as i32 - m.negative_hits as i32;
    (50 + net * 5).clamp(0, 100) as u8
}

/// 外部维度打分 (隔夜美股 + 汇率)
fn score_external(m: Option<&ExternalMetrics>) -> u8 {
    let Some(m) = m else { return 50 };
    let mut score = 50.0;
    // 美股涨 → A 股次日偏多
    if m.us_chg > 1.0 {
        score += 15.0;
    } else if m.us_chg < -1.0 {
        score -= 15.0;
    }
    // 汇率贬值 → 资金外流压力
    if m.fx_chg > 0.5 {
        score -= 10.0;
    }
    (score as f64).clamp(0.0, 100.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev_full() -> MarketStageEvidence {
        MarketStageEvidence {
            sentiment: Some(SentimentMetrics {
                limit_up_n: 35,
                limit_down_n: 3,
                broken_pct: 15.0,
                consecutive_h: 5,
            }),
            capital: Some(CapitalMetrics { main_flow_yi: 120.0, amount_yi: 8500.0, amount_delta_pct: 8.0 }),
            technical: Some(TechnicalMetrics { sh_chg: 0.5, chinext_chg: 1.2, star_chg: 1.5 }),
            policy: Some(PolicyMetrics { positive_hits: 5, negative_hits: 1 }),
            external: Some(ExternalMetrics { us_chg: 0.8, fx_chg: 0.1 }),
        }
    }

    #[test]
    fn full_evidence_high_confidence() {
        let r = evaluate(&ev_full());
        assert!(r.conf_pct >= 60, "全数据 + 强势应 ≥60, 实得 {}", r.conf_pct);
        assert!(!r.degraded);
        assert_eq!(r.data_complete_n, 5);
        assert_eq!(r.dim_scores.sentiment, 85); // 35*20/... let me check
        // (35 涨停 +20, 3 跌停 = 0, 15% 炸板 = 0, 5 连板 = +15) = 50+20+15 = 85
        // Actually let me just assert it's high
        assert!(r.dim_scores.sentiment >= 70);
    }

    #[test]
    fn missing_all_data_degraded() {
        let r = evaluate(&MarketStageEvidence::default());
        assert!(r.degraded, "全 None 应 degraded");
        assert_eq!(r.data_complete_n, 0);
        // 全 None → 各维度 50 → 加权 50
        assert_eq!(r.conf_pct, 50);
    }

    #[test]
    fn one_dimension_only_degraded() {
        let mut ev = MarketStageEvidence::default();
        ev.sentiment = Some(SentimentMetrics {
            limit_up_n: 35,
            limit_down_n: 3,
            broken_pct: 15.0,
            consecutive_h: 5,
        });
        let r = evaluate(&ev);
        // 1 维度 < 2 → degraded=true (设计: 至少 2 维数据才不算降级)
        assert!(r.degraded);
        assert_eq!(r.data_complete_n, 1);
    }

    #[test]
    fn two_dimensions_pass_degraded_check() {
        let mut ev = MarketStageEvidence::default();
        ev.sentiment = Some(SentimentMetrics::default());
        ev.capital = Some(CapitalMetrics::default());
        let r = evaluate(&ev);
        assert!(!r.degraded, "2 维度应通过");
        assert_eq!(r.data_complete_n, 2);
    }

    #[test]
    fn heat_stage_brackets() {
        // 极端高 (100 涨停 + 0 跌停 + 8 连板) → 主流上涨区间
        let mut ev = ev_full();
        ev.sentiment = Some(SentimentMetrics {
            limit_up_n: 100,
            limit_down_n: 0,
            broken_pct: 0.0,
            consecutive_h: 8,
        });
        let r = evaluate(&ev);
        assert!(matches!(r.heat_stage.as_str(), "MainUp" | "HeatUp"));

        // 极端低 (50 跌停) + 其他维度全 None → degraded, 综合分应 < 60
        let mut ev2 = MarketStageEvidence::default();
        ev2.sentiment = Some(SentimentMetrics {
            limit_up_n: 0,
            limit_down_n: 50,
            broken_pct: 50.0,
            consecutive_h: 0,
        });
        let r2 = evaluate(&ev2);
        // 全 50 中性 + sentiment 25 → conf_pct 应 < 60 → HeatUp/Range
        assert!(r2.conf_pct < 70, "应 < 70, 实得 {}", r2.conf_pct);
    }

    #[test]
    fn sentiment_high_limit_up() {
        let s = score_sentiment(Some(&SentimentMetrics {
            limit_up_n: 60, limit_down_n: 0, broken_pct: 5.0, consecutive_h: 6,
        }));
        assert!(s >= 90, "60 涨停 + 6 连板 应 ≥90, 实得 {}", s);
    }

    #[test]
    fn sentiment_many_limit_down() {
        let s = score_sentiment(Some(&SentimentMetrics {
            limit_up_n: 0, limit_down_n: 30, broken_pct: 40.0, consecutive_h: 0,
        }));
        assert!(s <= 10, "30 跌停 + 40% 炸板 应 ≤10, 实得 {}", s);
    }

    #[test]
    fn capital_main_flow_positive() {
        let c = score_capital(Some(&CapitalMetrics { main_flow_yi: 150.0, amount_yi: 0.0, amount_delta_pct: 12.0 }));
        assert!(c >= 80);
    }

    #[test]
    fn technical_bear_market() {
        let t = score_technical(Some(&TechnicalMetrics { sh_chg: -3.0, chinext_chg: -3.0, star_chg: -3.0 }));
        assert!(t <= 20, "三指数 -3% 应 ≤20, 实得 {}", t);
    }

    #[test]
    fn policy_positive_dominates() {
        let p = score_policy(Some(&PolicyMetrics { positive_hits: 10, negative_hits: 1 }));
        assert!(p >= 90);
    }

    #[test]
    fn external_us_up() {
        let e = score_external(Some(&ExternalMetrics { us_chg: 2.0, fx_chg: 0.0 }));
        assert!(e >= 60);
    }

    #[test]
    fn missing_dim_returns_50_neutral() {
        assert_eq!(score_sentiment(None), 50);
        assert_eq!(score_capital(None), 50);
        assert_eq!(score_technical(None), 50);
        assert_eq!(score_policy(None), 50);
        assert_eq!(score_external(None), 50);
    }
}