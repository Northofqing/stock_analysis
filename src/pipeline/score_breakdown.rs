//! 多维评分拆解：把"综合评分"拆为 5 个独立维度（0~100），
//! 让用户分别看见技术面 / 盈利质量 / 估值安全边际 / 资金面 / 增长可持续性。
//!
//! 设计原则：每个维度独立打分、独立解释，互不平均，避免短期技术信号
//! 与长期基本面被合并成一个含义模糊的综合分。

use serde::{Deserialize, Serialize};

use crate::data_provider::money_flow::MoneyFlowSummary;
use crate::data_provider::{assess_quality, KlineData};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FactorAction {
    Normal,
    Disable,
    Invert,
    DownWeight,
}

fn parse_factor_action(s: &str) -> FactorAction {
    match s.trim().to_ascii_lowercase().as_str() {
        "disable" => FactorAction::Disable,
        "invert" => FactorAction::Invert,
        "down_weight" => FactorAction::DownWeight,
        _ => FactorAction::Normal,
    }
}

fn apply_factor_action(score: i32, action: FactorAction, down_weight_scale: f64) -> f64 {
    let v = score.clamp(0, 100) as f64;
    match action {
        FactorAction::Normal => v,
        FactorAction::Disable => 0.0,
        FactorAction::Invert => 100.0 - v,
        FactorAction::DownWeight => v * down_weight_scale.clamp(0.0, 1.0),
    }
}

/// 计算评分所需的最小输入集合（不依赖完整 AnalysisResult，便于在 AI 调用前构造）。
pub struct ScoreInputs<'a> {
    pub sentiment_score: i32,
    pub money_flow: Option<&'a MoneyFlowSummary>,
    pub money_flow_section: Option<&'a str>,
    pub volume_ratio_5d: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    /// 技术面（趋势/均线/动量）
    pub technical: i32,
    /// 盈利质量（CFO/NI、毛利/净利、风险旗）
    pub fundamental_quality: i32,
    /// 估值安全边际（PE/PB 历史分位 + 行业分位 + 目标价空间）
    pub valuation_safety: i32,
    /// 资金面（主力净流入 + 量比）
    pub capital_flow: i32,
    /// 增长可持续性（多期营收/净利同比 + ROE 趋势）
    pub growth_sustainability: i32,
}

pub fn compute(inputs: &ScoreInputs<'_>, data: &KlineData) -> ScoreBreakdown {
    ScoreBreakdown {
        technical: inputs.sentiment_score.clamp(0, 100),
        fundamental_quality: quality_score(data),
        valuation_safety: valuation_score(data),
        capital_flow: capital_flow_score(inputs),
        growth_sustainability: growth_score(data),
    }
}

fn quality_score(d: &KlineData) -> i32 {
    let Some(hist) = d.financials_history.as_ref() else {
        return 50;
    };
    if hist.is_empty() {
        return 50;
    }
    let Some(q) = assess_quality(hist) else {
        return 50;
    };
    let base = 100i32.saturating_sub(q.risk_score as i32);
    // CFO/NI 加成：取近 6 期均值
    let ratios: Vec<f64> = hist
        .iter()
        .take(6)
        .filter_map(|p| p.cfo_to_ni_ratio())
        .collect();
    let bonus = if ratios.is_empty() {
        0
    } else {
        let avg = ratios.iter().sum::<f64>() / ratios.len() as f64;
        if avg >= 1.0 {
            10
        } else if avg >= 0.6 {
            5
        } else if avg < 0.3 {
            -10
        } else {
            0
        }
    };
    (base + bonus).clamp(0, 100)
}

fn valuation_score(d: &KlineData) -> i32 {
    let mut sum = 0i32;
    let mut count = 0i32;
    if let Some(vh) = &d.valuation_history {
        if let Some(pep) = vh.pe_percentile {
            sum += (100.0 - pep) as i32;
            count += 1;
        }
        if let Some(pbp) = vh.pb_percentile {
            sum += (100.0 - pbp) as i32;
            count += 1;
        }
    }
    if let Some(ind) = &d.industry {
        if let Some(p) = ind.pe_percentile {
            sum += (100.0 - p) as i32;
            count += 1;
        }
    }
    if let Some(cs) = &d.consensus {
        if let Some(upside) = cs.upside_pct(d.close) {
            let bonus = if upside > 30.0 {
                90
            } else if upside > 10.0 {
                75
            } else if upside > 0.0 {
                60
            } else if upside > -10.0 {
                40
            } else if upside > -20.0 {
                20
            } else {
                10
            };
            sum += bonus;
            count += 1;
        }
    }
    if count == 0 {
        50
    } else {
        (sum / count).clamp(0, 100)
    }
}

fn capital_flow_score(r: &ScoreInputs<'_>) -> i32 {
    let mut score = 50i32;
    // Phase 3: 优先使用原始资金流时序做 EWMA（指数加权，最近一天权重最大）
    if let Some(mf) = r.money_flow {
        if let Some(ewma_yi) = mf.ewma_main_net_yi() {
            score = if ewma_yi > 2.0 {
                90
            } else if ewma_yi > 0.5 {
                75
            } else if ewma_yi > 0.0 {
                60
            } else if ewma_yi > -0.5 {
                40
            } else if ewma_yi > -2.0 {
                25
            } else {
                10
            };
        }
        // 单日反弹但 5 日趋势仍流出 → 不能给资金面高分，强制压在 40 以下
        if mf.is_one_day_bounce() && score > 40 {
            score = 35;
        }
    } else if let Some(mf_section) = r.money_flow_section {
        // 兜底：旧路径，解析字符串
        if let Some(net) = parse_5d_net_yi(mf_section) {
            score = if net > 2.0 {
                90
            } else if net > 0.5 {
                75
            } else if net > 0.0 {
                60
            } else if net > -0.5 {
                40
            } else if net > -2.0 {
                25
            } else {
                10
            };
        }
    }
    if let Some(vr) = r.volume_ratio_5d {
        if vr > 1.5 {
            score = (score + 5).min(100);
        } else if vr < 0.7 {
            score = (score - 5).max(0);
        }
    }
    score.clamp(0, 100)
}

/// 从 money_flow_section 字符串里解析"近5日"主力累计净流入（单位：亿）。
/// BR-118: 只接受标签后、`亿` 前的完整有限浮点文本。
fn parse_5d_net_yi(s: &str) -> Option<f64> {
    let rest = s
        .split_once("近5日:")
        .map(|(_, rest)| rest)
        .or_else(|| s.split_once("近5日：").map(|(_, rest)| rest))?;
    let value_text = rest.split_once('亿')?.0.trim();
    let value = value_text.parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

fn growth_score(d: &KlineData) -> i32 {
    let Some(hist) = d.financials_history.as_ref() else {
        return 50;
    };
    let show: Vec<_> = hist.iter().take(4).collect();
    if show.is_empty() {
        return 50;
    }
    let rev: Vec<f64> = show.iter().filter_map(|p| p.revenue_yoy).collect();
    let np: Vec<f64> = show.iter().filter_map(|p| p.net_profit_yoy).collect();
    let mut score = 50i32;
    if !rev.is_empty() {
        let avg = rev.iter().sum::<f64>() / rev.len() as f64;
        score = if avg > 30.0 {
            85
        } else if avg > 10.0 {
            70
        } else if avg > 0.0 {
            55
        } else if avg > -10.0 {
            35
        } else {
            15
        };
    }
    if !np.is_empty() {
        let avg = np.iter().sum::<f64>() / np.len() as f64;
        // 净利与营收方向一致则微调
        if avg > 30.0 {
            score = (score + 5).min(100);
        } else if avg < -20.0 {
            score = (score - 10).max(0);
        }
    }
    // ROE 趋势加成
    let roes: Vec<f64> = show.iter().filter_map(|p| p.roe).collect();
    if roes.len() >= 3 {
        let strictly_up = roes.windows(2).all(|w| w[0] >= w[1]); // 越近越大
        let strictly_down = roes.windows(2).all(|w| w[0] <= w[1]); // 越近越小
        if strictly_up && !strictly_down {
            score = (score + 10).min(100);
        } else if strictly_down && !strictly_up {
            score = (score - 10).max(0);
        }
    }
    score.clamp(0, 100)
}

/// 渲染为 Markdown 区块（5 行表格 + 图标）
pub fn render_section(sb: &ScoreBreakdown) -> String {
    fn tag(v: i32) -> &'static str {
        if v >= 70 {
            "🟢"
        } else if v >= 40 {
            "🟡"
        } else {
            "🔴"
        }
    }
    let mut s = String::new();
    s.push_str("| 维度 | 分数 | 评估 |\n");
    s.push_str("|------|------|------|\n");
    s.push_str(&format!(
        "| 技术面 | {} | {} |\n",
        sb.technical,
        tag(sb.technical)
    ));
    s.push_str(&format!(
        "| 盈利质量 | {} | {} |\n",
        sb.fundamental_quality,
        tag(sb.fundamental_quality)
    ));
    s.push_str(&format!(
        "| 估值安全边际 | {} | {} |\n",
        sb.valuation_safety,
        tag(sb.valuation_safety)
    ));
    s.push_str(&format!(
        "| 资金面 | {} | {} |\n",
        sb.capital_flow,
        tag(sb.capital_flow)
    ));
    s.push_str(&format!(
        "| 增长可持续 | {} | {} |\n",
        sb.growth_sustainability,
        tag(sb.growth_sustainability)
    ));
    s
}

/// 基于五维评分计算排序分（0~100），用于展示/排序/回测选股。
///
/// 注意：此分数不用于买入触发，不修改 sentiment_score 主链路。
pub fn compute_ranking_score(sb: &ScoreBreakdown) -> i32 {
    let cfg = &crate::config::get_monitor_config().factor_feedback;

    let (tech_action, quality_action, valuation_action, flow_action, growth_action) = if cfg.enabled
    {
        (
            parse_factor_action(&cfg.technical_action),
            parse_factor_action(&cfg.quality_action),
            parse_factor_action(&cfg.valuation_action),
            parse_factor_action(&cfg.flow_action),
            parse_factor_action(&cfg.growth_action),
        )
    } else {
        (
            FactorAction::Normal,
            FactorAction::Normal,
            FactorAction::Normal,
            FactorAction::Normal,
            FactorAction::Normal,
        )
    };

    let scale = cfg.down_weight_scale;

    // 等权平均：先按 action 转换，再聚合为排序分。
    let dims = [
        apply_factor_action(sb.technical, tech_action, scale),
        apply_factor_action(sb.fundamental_quality, quality_action, scale),
        apply_factor_action(sb.valuation_safety, valuation_action, scale),
        apply_factor_action(sb.capital_flow, flow_action, scale),
        apply_factor_action(sb.growth_sustainability, growth_action, scale),
    ];

    (dims.iter().sum::<f64>() / dims.len() as f64)
        .round()
        .clamp(0.0, 100.0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_provider::consensus::ConsensusData;
    use crate::data_provider::financials::FinancialPeriod;
    use crate::data_provider::industry::IndustryBenchmark;
    use crate::data_provider::money_flow::{MoneyFlowDay, MoneyFlowSummary};
    use crate::data_provider::valuation_history::ValuationHistory;
    use crate::data_provider::{AdjustType, KlineData};
    use chrono::NaiveDate;

    fn kline(close: f64) -> KlineData {
        KlineData {
            date: NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid fixture date"),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1_000.0,
            amount: close * 1_000.0,
            pct_chg: 0.0,
            intraday_price: None,
            settled: true,
            pe_ratio: None,
            pb_ratio: None,
            turnover_rate: None,
            market_cap: None,
            circulating_cap: None,
            eps: None,
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            sharpe_ratio: None,
            financials_history: None,
            valuation_history: None,
            consensus: None,
            industry: None,
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
            adjust: AdjustType::None,
        }
    }

    fn valuation_history(pe_percentile: f64, pb_percentile: f64) -> ValuationHistory {
        ValuationHistory {
            current_pe: Some(20.0),
            current_pb: Some(2.0),
            pe_percentile: Some(pe_percentile),
            pb_percentile: Some(pb_percentile),
            pe_min: Some(10.0),
            pe_max: Some(30.0),
            pe_median: Some(20.0),
            pb_min: Some(1.0),
            pb_max: Some(3.0),
            pb_median: Some(2.0),
            sample_days: 3,
            oldest_date: Some("2026-07-16".into()),
            newest_date: Some("2026-07-18".into()),
        }
    }

    fn one_day_flow(main_net: f64) -> MoneyFlowSummary {
        MoneyFlowSummary {
            days: vec![MoneyFlowDay {
                date: "2026-07-18".into(),
                main_net,
                xl_net: 0.0,
                big_net: 0.0,
                main_pct: 0.0,
                pct_chg: 0.0,
            }],
        }
    }

    #[test]
    fn missing_evidence_is_neutral_and_sentiment_is_clamped() {
        let score = compute(
            &ScoreInputs {
                sentiment_score: 120,
                money_flow: None,
                money_flow_section: None,
                volume_ratio_5d: None,
            },
            &kline(10.0),
        );

        assert_eq!(score.technical, 100);
        assert_eq!(score.fundamental_quality, 50);
        assert_eq!(score.valuation_safety, 50);
        assert_eq!(score.capital_flow, 50);
        assert_eq!(score.growth_sustainability, 50);
    }

    #[test]
    fn factor_feedback_actions_transform_clamped_scores() {
        assert_eq!(parse_factor_action(" disable "), FactorAction::Disable);
        assert_eq!(parse_factor_action("INVERT"), FactorAction::Invert);
        assert_eq!(parse_factor_action("down_weight"), FactorAction::DownWeight);
        assert_eq!(parse_factor_action("unknown"), FactorAction::Normal);

        assert_eq!(apply_factor_action(120, FactorAction::Normal, 0.5), 100.0);
        assert_eq!(apply_factor_action(80, FactorAction::Disable, 0.5), 0.0);
        assert_eq!(apply_factor_action(-10, FactorAction::Invert, 0.5), 100.0);
        assert_eq!(apply_factor_action(80, FactorAction::DownWeight, 0.5), 40.0);
        assert_eq!(apply_factor_action(80, FactorAction::DownWeight, 2.0), 80.0);
        assert_eq!(apply_factor_action(80, FactorAction::DownWeight, -1.0), 0.0);
    }

    #[test]
    fn legacy_money_flow_text_scores_the_value_after_the_label() {
        let score = compute(
            &ScoreInputs {
                sentiment_score: 50,
                money_flow: None,
                money_flow_section: Some("主力资金近5日: +2.50亿"),
                volume_ratio_5d: Some(1.6),
            },
            &kline(10.0),
        );

        assert_eq!(score.capital_flow, 95);
    }

    #[test]
    fn legacy_money_flow_text_accepts_chinese_colon_and_rejects_bad_values() {
        let capital_score = |section: &str| {
            compute(
                &ScoreInputs {
                    sentiment_score: 50,
                    money_flow: None,
                    money_flow_section: Some(section),
                    volume_ratio_5d: None,
                },
                &kline(10.0),
            )
            .capital_flow
        };

        assert_eq!(capital_score("近5日：-1.25亿"), 25);
        assert_eq!(capital_score("近5日: NaN亿"), 50);
        assert_eq!(capital_score("近5日: +2.00说明亿"), 50);
        assert_eq!(capital_score("近5日: +2.00"), 50);
        assert_eq!(capital_score("近3日: +2.00亿"), 50);
    }

    #[test]
    fn valuation_score_combines_history_industry_and_consensus_evidence() {
        let mut data = kline(10.0);
        data.valuation_history = Some(valuation_history(20.0, 40.0));
        data.industry = Some(IndustryBenchmark {
            pe_percentile: Some(60.0),
            ..IndustryBenchmark::default()
        });
        data.consensus = Some(ConsensusData {
            target_price_high_avg: Some(13.1),
            ..ConsensusData::default()
        });

        let score = compute(
            &ScoreInputs {
                sentiment_score: 50,
                money_flow: None,
                money_flow_section: None,
                volume_ratio_5d: None,
            },
            &data,
        );

        assert_eq!(score.valuation_safety, 67);
    }

    #[test]
    fn valuation_target_price_bands_are_deterministic() {
        let cases = [
            (13.1, 90),
            (11.1, 75),
            (10.1, 60),
            (9.5, 40),
            (8.5, 20),
            (7.0, 10),
        ];

        for (target_price, expected) in cases {
            let mut data = kline(10.0);
            data.consensus = Some(ConsensusData {
                target_price_high_avg: Some(target_price),
                ..ConsensusData::default()
            });
            let actual = compute(
                &ScoreInputs {
                    sentiment_score: 50,
                    money_flow: None,
                    money_flow_section: None,
                    volume_ratio_5d: None,
                },
                &data,
            );

            assert_eq!(actual.valuation_safety, expected, "target={target_price}");
        }
    }

    #[test]
    fn financial_quality_rewards_cash_coverage_and_penalizes_accrual_risk() {
        let mut healthy = kline(10.0);
        healthy.financials_history = Some(vec![FinancialPeriod {
            eps: Some(1.0),
            op_cash_flow_ps: Some(1.2),
            revenue_yoy: Some(10.0),
            net_profit_yoy: Some(12.0),
            ..FinancialPeriod::default()
        }]);
        let mut risky = kline(10.0);
        risky.financials_history = Some(vec![FinancialPeriod {
            eps: Some(1.0),
            op_cash_flow_ps: Some(0.2),
            revenue_yoy: Some(10.0),
            net_profit_yoy: Some(50.0),
            ..FinancialPeriod::default()
        }]);
        let inputs = ScoreInputs {
            sentiment_score: 50,
            money_flow: None,
            money_flow_section: None,
            volume_ratio_5d: None,
        };

        assert_eq!(compute(&inputs, &healthy).fundamental_quality, 100);
        assert_eq!(compute(&inputs, &risky).fundamental_quality, 65);
    }

    #[test]
    fn growth_revenue_bands_use_independent_literal_scores() {
        let inputs = ScoreInputs {
            sentiment_score: 50,
            money_flow: None,
            money_flow_section: None,
            volume_ratio_5d: None,
        };
        for (revenue_yoy, expected) in [(31.0, 85), (11.0, 70), (1.0, 55), (-5.0, 35), (-11.0, 15)]
        {
            let mut data = kline(10.0);
            data.financials_history = Some(vec![FinancialPeriod {
                revenue_yoy: Some(revenue_yoy),
                ..FinancialPeriod::default()
            }]);

            assert_eq!(
                compute(&inputs, &data).growth_sustainability,
                expected,
                "revenue_yoy={revenue_yoy}"
            );
        }
    }

    #[test]
    fn profit_and_roe_trends_adjust_growth_score() {
        let inputs = ScoreInputs {
            sentiment_score: 50,
            money_flow: None,
            money_flow_section: None,
            volume_ratio_5d: None,
        };
        let mut improving = kline(10.0);
        improving.financials_history = Some(vec![
            FinancialPeriod {
                revenue_yoy: Some(31.0),
                net_profit_yoy: Some(31.0),
                roe: Some(15.0),
                ..FinancialPeriod::default()
            },
            FinancialPeriod {
                revenue_yoy: Some(31.0),
                net_profit_yoy: Some(31.0),
                roe: Some(12.0),
                ..FinancialPeriod::default()
            },
            FinancialPeriod {
                revenue_yoy: Some(31.0),
                net_profit_yoy: Some(31.0),
                roe: Some(10.0),
                ..FinancialPeriod::default()
            },
        ]);
        let mut deteriorating = kline(10.0);
        deteriorating.financials_history = Some(vec![
            FinancialPeriod {
                revenue_yoy: Some(-5.0),
                net_profit_yoy: Some(-30.0),
                roe: Some(8.0),
                ..FinancialPeriod::default()
            },
            FinancialPeriod {
                revenue_yoy: Some(-5.0),
                net_profit_yoy: Some(-30.0),
                roe: Some(10.0),
                ..FinancialPeriod::default()
            },
            FinancialPeriod {
                revenue_yoy: Some(-5.0),
                net_profit_yoy: Some(-30.0),
                roe: Some(12.0),
                ..FinancialPeriod::default()
            },
        ]);

        assert_eq!(compute(&inputs, &improving).growth_sustainability, 100);
        assert_eq!(compute(&inputs, &deteriorating).growth_sustainability, 15);
    }

    #[test]
    fn raw_money_flow_ewma_bands_are_deterministic() {
        for (main_net, expected) in [
            (2.1e8, 90),
            (1.0e8, 75),
            (0.1e8, 60),
            (-0.1e8, 40),
            (-1.0e8, 25),
            (-3.0e8, 10),
        ] {
            let flow = one_day_flow(main_net);
            let actual = compute(
                &ScoreInputs {
                    sentiment_score: 50,
                    money_flow: Some(&flow),
                    money_flow_section: Some("近5日: +99.00亿"),
                    volume_ratio_5d: None,
                },
                &kline(10.0),
            );

            assert_eq!(actual.capital_flow, expected, "main_net={main_net}");
        }
    }

    #[test]
    fn volume_ratio_adjusts_capital_score_without_fabricating_flow() {
        let capital_score = |volume_ratio_5d| {
            compute(
                &ScoreInputs {
                    sentiment_score: 50,
                    money_flow: None,
                    money_flow_section: None,
                    volume_ratio_5d,
                },
                &kline(10.0),
            )
            .capital_flow
        };

        assert_eq!(capital_score(Some(1.6)), 55);
        assert_eq!(capital_score(Some(1.0)), 50);
        assert_eq!(capital_score(Some(0.6)), 45);
        assert_eq!(capital_score(None), 50);
    }

    #[test]
    fn score_rendering_and_default_ranking_expose_all_dimensions() {
        let score = ScoreBreakdown {
            technical: 70,
            fundamental_quality: 40,
            valuation_safety: 39,
            capital_flow: 100,
            growth_sustainability: 0,
        };

        let rendered = render_section(&score);
        assert!(rendered.contains("| 技术面 | 70 | 🟢 |"));
        assert!(rendered.contains("| 盈利质量 | 40 | 🟡 |"));
        assert!(rendered.contains("| 估值安全边际 | 39 | 🔴 |"));
        assert!(rendered.contains("| 资金面 | 100 | 🟢 |"));
        assert!(rendered.contains("| 增长可持续 | 0 | 🔴 |"));
        assert_eq!(compute_ranking_score(&score), 50);
    }
}

#[cfg(test)]
#[path = "../gate_d_score_breakdown_regression.rs"]
mod gate_d_regression;
