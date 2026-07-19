//! 风险否决规则：在既有评分 / 操作建议外，引入"硬约束"层。
//!
//! 当触发否决信号时：
//! 1. 在报告中显式输出"🚫 风险否决信号"区块
//! 2. 视严重程度强制下调 operation_advice
//! 3. 标注仓位上限（仅展示性，供用户参考）
//!
//! 4 条 Phase 1 规则：
//! - 营收连续 3 期负增长 → 不得输出『买入』
//! - CFO/NI<0.3 且 净利同比>营收同比×2 → 利润含金量警告 + 仓位 ≤30%
//! - 现价超出卖方目标价均值 >15% → 估值透支 + 仓位 ≤30%
//! - 双高估值分层否决：
//!   - P99/P99 极端档：任何『买入』→『观望』，仓位 ≤20%
//!   - P95/P95 严重档：任何『买入』→『观望』（仅回调介入），仓位 ≤30%
//!   - P80/P90 基础档：禁止『强烈建议买入』

use serde::{Deserialize, Serialize};

use crate::data_provider::money_flow::MoneyFlowSummary;
use crate::data_provider::KlineData;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VetoOutcome {
    pub flags: Vec<String>,
    /// 若需下调建议，给出新值；None 表示无需下调
    pub downgraded_advice: Option<String>,
    /// 仓位上限（百分比，仅展示）
    pub position_cap_pct: Option<u32>,
}

impl VetoOutcome {
    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }
}

pub fn evaluate(
    original_advice: &str,
    money_flow: Option<&MoneyFlowSummary>,
    data: &KlineData,
) -> VetoOutcome {
    let mut out = VetoOutcome::default();
    let original = original_advice.to_string();

    // Rule 1: 营收连续 3 期负增长 → 禁止买入
    if let Some(hist) = data.financials_history.as_ref() {
        let recent: Vec<f64> = hist.iter().take(3).filter_map(|p| p.revenue_yoy).collect();
        if recent.len() >= 3 && recent.iter().all(|v| *v < 0.0) {
            out.flags.push(format!(
                "🚫 营收连续 3 期负增长（{:.1}% / {:.1}% / {:.1}%）→ 禁止输出『买入』",
                recent[0], recent[1], recent[2]
            ));
            if original.contains("买入") {
                out.downgraded_advice = Some("观望".to_string());
            }
        }
    }

    // Rule 2: CFO/NI<0.3 且 净利增速>营收增速×2 → 利润含金量警告
    if let Some(hist) = data.financials_history.as_ref() {
        if let Some(latest) = hist.first() {
            if let (Some(c), Some(np), Some(rev)) = (
                latest.cfo_to_ni_ratio(),
                latest.net_profit_yoy,
                latest.revenue_yoy,
            ) {
                if c < 0.3 && rev.abs() > 1e-3 && np > rev.abs() * 2.0 {
                    out.flags.push(format!(
                        "⚠️ 利润含金量警告：CFO/NI={:.2} 偏低，且净利增速 {:.1}% ≈ 营收增速 {:.1}% 的 {:.1} 倍 → 应计利润可疑，建议仓位 ≤30%",
                        c, np, rev, np / rev.abs()
                    ));
                    cap_position(&mut out, 30);
                }
            }
        }
    }

    // Rule 3: 现价 > 目标价均值 ×1.15 → 估值透支
    if let Some(cs) = data.consensus.as_ref() {
        if let Some(upside) = cs.upside_pct(data.close) {
            if upside < -15.0 {
                out.flags.push(format!(
                    "⚠️ 估值透支：现价已高于卖方目标价均值 {:.1}% → 建议仓位 ≤30%",
                    upside.abs()
                ));
                cap_position(&mut out, 30);
            }
        }
    }

    // Rule 4: 双高估值分层否决（按极端程度三档处理，越极端力度越强）
    if let Some(vh) = data.valuation_history.as_ref() {
        if let (Some(pep), Some(pbp)) = (vh.pe_percentile, vh.pb_percentile) {
            if pep >= 99.0 && pbp >= 99.0 {
                // 极端档：估值处于历史最高分位，禁止任何买入
                out.flags.push(format!(
                    "🚫 极端双高估值：PE 历史分位 P{:.0} + PB 历史分位 P{:.0} 均处历史极值 → 禁止任何买入，仅回调后重新评估，仓位 ≤20%",
                    pep, pbp
                ));
                if original.contains("买入") && out.downgraded_advice.is_none() {
                    out.downgraded_advice = Some("观望".to_string());
                }
                cap_position(&mut out, 20);
            } else if pep > 95.0 && pbp > 95.0 {
                // 严重档：禁止追高买入，仅回调介入
                out.flags.push(format!(
                    "🚫 严重双高估值：PE 历史分位 P{:.0} + PB 历史分位 P{:.0} → 禁止追高买入（仅回调介入），仓位 ≤30%",
                    pep, pbp
                ));
                if original.contains("买入") && out.downgraded_advice.is_none() {
                    out.downgraded_advice = Some("观望".to_string());
                }
                cap_position(&mut out, 30);
            } else if pep > 80.0 && pbp > 90.0 {
                // 基础档：禁止『强烈建议买入』
                out.flags.push(format!(
                    "🚫 双高估值：PE 历史分位 P{:.0} + PB 历史分位 P{:.0} → 禁止『强烈建议买入』",
                    pep, pbp
                ));
                if original == "强烈建议买入" && out.downgraded_advice.is_none() {
                    out.downgraded_advice = Some("建议买入".to_string());
                }
            }
        }
    }

    // Rule 5 (Phase 3): 5 日累计流出 >30 亿 且 最新日反弹流入 <累计流出 20%
    //   → 单日反弹，趋势未逆转；不得输出『强烈建议买入』，仓位 ≤50%
    if let Some(mf) = money_flow {
        if mf.is_one_day_bounce() {
            let sum5_yi = mf.recent_main_sum(5) / 1e8;
            let latest_yi = mf.latest().map(|d| d.main_net / 1e8).unwrap_or(0.0);
            let ratio_pct = if sum5_yi.abs() > 1e-9 {
                latest_yi / sum5_yi.abs() * 100.0
            } else {
                0.0
            };
            out.flags.push(format!(
                "⚠️ 单日反弹，趋势未逆转：近 5 日主力累计流出 {:.1} 亿，但最新日仅流入 {:.2} 亿（占累计流出 {:.0}%）",
                sum5_yi, latest_yi, ratio_pct
            ));
            cap_position(&mut out, 50);
            if original == "强烈建议买入" && out.downgraded_advice.is_none() {
                out.downgraded_advice = Some("建议买入".to_string());
            }
        }
    }

    out
}

fn cap_position(out: &mut VetoOutcome, cap: u32) {
    out.position_cap_pct = Some(match out.position_cap_pct {
        Some(existing) => existing.min(cap),
        None => cap,
    });
}

/// 渲染否决信号区块（None 时无任何信号触发）。
pub fn render_section(outcome: &VetoOutcome, original_advice: &str) -> Option<String> {
    if outcome.is_empty() {
        return None;
    }
    let mut s = String::new();
    if let Some(new) = &outcome.downgraded_advice {
        s.push_str(&format!(
            "**操作建议调整**：『{}』 → 『{}』\n\n",
            original_advice, new
        ));
    }
    if let Some(cap) = outcome.position_cap_pct {
        s.push_str(&format!("**仓位上限**：≤ {}%\n\n", cap));
    }
    s.push_str("**触发的否决信号**：\n\n");
    for f in &outcome.flags {
        s.push_str(&format!("- {}\n", f));
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_provider::consensus::ConsensusData;
    use crate::data_provider::financials::FinancialPeriod;
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

    #[test]
    fn missing_optional_evidence_produces_no_veto() {
        let outcome = evaluate("观望", None, &kline(10.0));

        assert!(outcome.is_empty());
        assert_eq!(outcome.downgraded_advice, None);
        assert_eq!(outcome.position_cap_pct, None);
        assert_eq!(render_section(&outcome, "观望"), None);
    }

    #[test]
    fn three_negative_revenue_periods_block_buy_advice() {
        let mut data = kline(10.0);
        data.financials_history = Some(
            [-5.0, -10.0, -15.0]
                .into_iter()
                .map(|revenue_yoy| FinancialPeriod {
                    revenue_yoy: Some(revenue_yoy),
                    ..FinancialPeriod::default()
                })
                .collect(),
        );

        let outcome = evaluate("建议买入", None, &data);

        assert_eq!(outcome.downgraded_advice.as_deref(), Some("观望"));
        assert_eq!(outcome.position_cap_pct, None);
        assert_eq!(outcome.flags.len(), 1);
        assert!(outcome.flags[0].contains("营收连续 3 期负增长"));
    }

    #[test]
    fn low_cash_coverage_with_profit_divergence_caps_position() {
        let mut data = kline(10.0);
        data.financials_history = Some(vec![FinancialPeriod {
            eps: Some(1.0),
            op_cash_flow_ps: Some(0.2),
            revenue_yoy: Some(10.0),
            net_profit_yoy: Some(30.0),
            ..FinancialPeriod::default()
        }]);

        let outcome = evaluate("观望", None, &data);

        assert_eq!(outcome.downgraded_advice, None);
        assert_eq!(outcome.position_cap_pct, Some(30));
        assert_eq!(outcome.flags.len(), 1);
        assert!(outcome.flags[0].contains("CFO/NI=0.20"));
        assert!(outcome.flags[0].contains("建议仓位 ≤30%"));
    }

    #[test]
    fn price_above_consensus_target_caps_position() {
        let mut data = kline(10.0);
        data.consensus = Some(ConsensusData {
            target_price_high_avg: Some(8.0),
            ..ConsensusData::default()
        });

        let outcome = evaluate("观望", None, &data);

        assert_eq!(outcome.position_cap_pct, Some(30));
        assert_eq!(outcome.flags.len(), 1);
        assert!(outcome.flags[0].contains("现价已高于卖方目标价均值 20.0%"));
    }

    #[test]
    fn valuation_tiers_apply_documented_downgrades_and_caps() {
        let cases = [
            (
                99.0,
                99.0,
                "建议买入",
                Some("观望"),
                Some(20),
                "极端双高估值",
            ),
            (
                98.0,
                98.0,
                "建议买入",
                Some("观望"),
                Some(30),
                "严重双高估值",
            ),
            (
                85.0,
                95.0,
                "强烈建议买入",
                Some("建议买入"),
                None,
                "双高估值",
            ),
            (85.0, 95.0, "建议买入", None, None, "双高估值"),
        ];

        for (pe, pb, advice, downgraded, cap, flag_text) in cases {
            let mut data = kline(10.0);
            data.valuation_history = Some(valuation_history(pe, pb));
            let outcome = evaluate(advice, None, &data);

            assert_eq!(outcome.downgraded_advice.as_deref(), downgraded);
            assert_eq!(outcome.position_cap_pct, cap);
            assert_eq!(outcome.flags.len(), 1);
            assert!(outcome.flags[0].contains(flag_text));
        }
    }

    #[test]
    fn one_day_money_flow_bounce_downgrades_strong_buy() {
        let days = [-10.0, -10.0, -10.0, -12.0, 2.0]
            .into_iter()
            .enumerate()
            .map(|(index, main_net_yi)| MoneyFlowDay {
                date: format!("2026-07-{}", 14 + index),
                main_net: main_net_yi * 1e8,
                xl_net: 0.0,
                big_net: 0.0,
                main_pct: 0.0,
                pct_chg: 0.0,
            })
            .collect();
        let flow = MoneyFlowSummary { days };

        let outcome = evaluate("强烈建议买入", Some(&flow), &kline(10.0));

        assert_eq!(outcome.downgraded_advice.as_deref(), Some("建议买入"));
        assert_eq!(outcome.position_cap_pct, Some(50));
        assert_eq!(outcome.flags.len(), 1);
        assert!(outcome.flags[0].contains("累计流出 -40.0 亿"));
        assert!(outcome.flags[0].contains("最新日仅流入 2.00 亿"));
    }

    #[test]
    fn strictest_cap_and_first_downgrade_win_across_multiple_vetoes() {
        let mut data = kline(10.0);
        data.financials_history = Some(vec![
            FinancialPeriod {
                eps: Some(1.0),
                op_cash_flow_ps: Some(0.2),
                revenue_yoy: Some(-5.0),
                net_profit_yoy: Some(20.0),
                ..FinancialPeriod::default()
            },
            FinancialPeriod {
                revenue_yoy: Some(-10.0),
                ..FinancialPeriod::default()
            },
            FinancialPeriod {
                revenue_yoy: Some(-15.0),
                ..FinancialPeriod::default()
            },
        ]);
        data.valuation_history = Some(valuation_history(99.0, 99.0));
        let flow = MoneyFlowSummary {
            days: [-10.0, -10.0, -10.0, -12.0, 2.0]
                .into_iter()
                .enumerate()
                .map(|(index, main_net_yi)| MoneyFlowDay {
                    date: format!("2026-07-{}", 14 + index),
                    main_net: main_net_yi * 1e8,
                    xl_net: 0.0,
                    big_net: 0.0,
                    main_pct: 0.0,
                    pct_chg: 0.0,
                })
                .collect(),
        };

        let outcome = evaluate("强烈建议买入", Some(&flow), &data);

        assert_eq!(outcome.downgraded_advice.as_deref(), Some("观望"));
        assert_eq!(outcome.position_cap_pct, Some(20));
        assert_eq!(outcome.flags.len(), 4);
        let rendered = render_section(&outcome, "强烈建议买入").expect("veto section");
        assert!(rendered.contains("『强烈建议买入』 → 『观望』"));
        assert!(rendered.contains("**仓位上限**：≤ 20%"));
        assert_eq!(rendered.matches("\n- ").count(), 4);
    }
}
