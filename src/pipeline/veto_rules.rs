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
//! - PE 历史分位>80 且 PB 历史分位>90 → 禁止『强烈建议买入』

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

    // Rule 4: PB 分位>90 且 PE 分位>80 → 禁止"强烈建议买入"
    if let Some(vh) = data.valuation_history.as_ref() {
        if let (Some(pep), Some(pbp)) = (vh.pe_percentile, vh.pb_percentile) {
            if pep > 80.0 && pbp > 90.0 {
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
