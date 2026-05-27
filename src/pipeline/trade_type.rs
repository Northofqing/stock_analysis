//! Phase 2: 交易类型标注。
//!
//! 同样的"建议买入"对不同股票意味着完全不同的持有逻辑：
//!   - 动量交易型：估值偏高、技术强势 → 顺势短线，止损要快，不适合价值长持
//!   - 逆向价值型：估值便宜、技术尚弱 → 左侧布局，持有周期 3-6 个月，需等技术确认
//!   - 趋势跟随型：估值中性、技术健康多头 → 中期持有，跟随均线节奏
//!   - 综合配置型：各维度均衡，无明显倾向
//!
//! 由 score_breakdown 计算完之后调用，不修改 sentiment_score。

use super::score_breakdown::ScoreBreakdown;

pub fn infer_from_breakdown(sb: &ScoreBreakdown) -> Option<String> {
    let tech = sb.technical;
    let val = sb.valuation_safety;
    let flow = sb.capital_flow;

    // 估值高 + 技术强 + 资金跟随 → 动量交易型
    if val < 40 && tech >= 65 && flow >= 55 {
        return Some(format!(
            "🚀 动量交易型 — 估值偏贵({})、技术强势({})、资金跟随({})；顺势短线为主，止损要快，不适合价值长持",
            val, tech, flow
        ));
    }
    // 估值便宜 + 技术尚未走强 → 逆向价值型
    if val >= 65 && tech < 55 {
        return Some(format!(
            "🔄 逆向价值型 — 估值便宜({})、技术尚弱({})；左侧布局逻辑，持有周期 3-6 个月，需等技术面确认（突破 MA20 / MACD 金叉）后再加仓",
            val, tech
        ));
    }
    // 估值中性 + 技术健康多头 → 趋势跟随型
    if (40..70).contains(&val) && tech >= 60 {
        return Some(format!(
            "📈 趋势跟随型 — 估值中性({})、技术健康({})；跟随均线节奏，跌破 MA20 减仓",
            val, tech
        ));
    }
    // 估值便宜 + 技术也强 → 价值+趋势共振，最佳标的
    if val >= 60 && tech >= 60 {
        return Some(format!(
            "💎 价值-趋势共振型 — 估值便宜({})、技术健康({})；中长期配置型机会",
            val, tech
        ));
    }
    Some(format!(
        "⚖️ 综合配置型 — 估值{}/技术{}/资金{}，各维度无明显倾向，按整体评分档位执行",
        val, tech, flow
    ))
}
