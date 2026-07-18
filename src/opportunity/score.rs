//! 修复 P0-1: dual_score 评分模型
//!
//! 设计哲学: 量化产品经理的硬约束是 NS3 — 0~100 分是"风险评估"而非"胜率预测"。
//! 之前的 ad-hoc 加分模型 (score_hit_confidence) 把两件事混在一起, 量化 PM 视角看是危险信号:
//!   - winrate_score 占位 50 = "假装普通", 系统偏差 7.5 分
//!   - 单 final 0~100 让下游以为 = 胜率信号
//!
//! 修复:
//!   - dual_score: event_risk_score (风险) + trade_signal_score (胜率, 可选 None)
//!   - data_sufficiency: 区分"真弱"和"数据不足"
//!   - data_sufficiency < 2 项 false -> event_risk_score 封顶为推送门减 1
//!   - winrate 样本 < 200 -> trade_signal_score = None (不假装 50)
//!   - weight_version 落审计, 上线后可回溯
//!
//! 修复 P1-2: winrate 二元化 (None / 0 / 真实 0-100)

use serde::{Deserialize, Serialize};

/// BR-096 机器合同中的完整数据评分值域上限。
pub const EVENT_RISK_SCORE_MAX: u8 = 100;
/// 两参兼容入口使用的安全默认推送门，与 strategy.toml 对齐。
pub const DEFAULT_OPPORTUNITY_PUSH_THRESHOLD: u8 = 75;

pub fn valid_push_threshold(threshold: u8) -> bool {
    (1..=EVENT_RISK_SCORE_MAX).contains(&threshold)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScorePart {
    pub name: String,
    pub value: f64, // 0-100
    pub weight: f64,
    /// 修复 P0-1: 区分"真弱"和"数据不足"
    /// false = 数据缺, 给的中性值 50, 不应被解读为中性
    /// true = 数据齐, value 反映真实
    pub data_sufficiency: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct DataSufficiency {
    /// 风险评估是否够数据 (≥4/5 项 data_sufficiency=true)
    pub event_risk_sufficient: bool,
    /// 胜率是否够 (≥200 样本 + 真实胜率 > 0)
    pub winrate_sufficient: bool,
    /// 实时资金热度是否有
    pub has_intraday_flow: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpportunityScore {
    /// 风险评估 (NS3 唯一维度, 0-100)
    /// 含义: 事件对受益方的预期冲击力度 + 不确定性
    pub event_risk_score: u8,
    /// 交易信号 (None = 样本不足, 0 = 明确负信号, 0-100 = 真实胜率)
    pub trade_signal_score: Option<u8>,
    /// 评分明细 (可追溯, 至少 5 项)
    pub parts: Vec<ScorePart>,
    pub data_sufficiency: DataSufficiency,
    /// 权重版本 (审计追溯)
    pub weight_version: String,
    /// 备注 (NS3 警示, 数据不足说明, 风险评估定位)
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ScoreInputs {
    pub event_strength: u8,
    pub event_certainty: u8,
    pub chain_match_score: u8,
    /// None = 数据缺
    pub flow_score: Option<f64>,
    /// 1 = 单源, 2+ = 跨源
    pub cross_source_count: u8,
    /// None = 数据缺
    pub quality_score: Option<f64>,
    /// 修复 P1-2: None = 样本不足, Some(0) = 无数据, Some(0.65) = 真实胜率 65%
    pub winrate_score: Option<f64>,
    /// 修复 P0-1: AI 降级标志 (true=规则降级抽取, event_score ×0.5)
    /// spec §0/§5: "AI 不可用 → ⑦ 降权 ×0.5"
    /// 默认 false (AI 正常抽取); event_extractor/build_degraded 路径置 true
    pub ai_degraded: bool,
}

impl Default for ScoreInputs {
    /// 提供默认值, 便于测试和增量构建
    /// 量化 PM 视角: 任何字段缺省都应是"保守"而非"乐观"
    fn default() -> Self {
        Self {
            event_strength: 0,
            event_certainty: 0,
            chain_match_score: 0,
            flow_score: None,
            cross_source_count: 1, // 默认单源 (保守, cross_score 低)
            quality_score: None,
            winrate_score: None,
            ai_degraded: false,
        }
    }
}

/// 修复 P0-1: dual_score 计算
/// event_risk_score: 5 项加权, 无 winrate 数据时封顶为默认推送门减 1
/// trade_signal_score: 单独 None/0/0-100
pub fn compute_dual_score(inputs: &ScoreInputs, weight_version: &str) -> OpportunityScore {
    compute_dual_score_with_threshold(inputs, weight_version, DEFAULT_OPPORTUNITY_PUSH_THRESHOLD)
}

/// BR-014 / BR-096: 生产路径必须把最终比较使用的同一阈值显式传入评分。
pub fn compute_dual_score_with_threshold(
    inputs: &ScoreInputs,
    weight_version: &str,
    push_threshold: u8,
) -> OpportunityScore {
    let mut parts = Vec::new();
    let mut notes = Vec::new();

    // event_risk_score 五项 (修复 P0-1: winrate 不参与, 这是 NS3 风险评估)
    let event_s_raw =
        (inputs.event_strength.min(100) as f64 + inputs.event_certainty.min(100) as f64) / 2.0;
    // 修复 P0-1 + spec §0/§5: AI 不可用 → event_score ×0.5
    // ai_degraded=true 表示规则降级, strength/certainty 是保守默认值, 应进一步降权
    let event_s = if inputs.ai_degraded {
        notes.push("[AI降级] event_score ×0.5".to_string());
        event_s_raw * 0.5
    } else {
        event_s_raw
    };
    let chain_s = inputs.chain_match_score.min(100) as f64;
    let flow_s = inputs.flow_score.unwrap_or(50.0);
    let cross_s = (inputs.cross_source_count.min(5) as f64 * 25.0).min(100.0);
    let quality_s = inputs.quality_score.unwrap_or(50.0);

    // 修复 P0-1: ai_degraded 时 event 项 data_sufficiency=false
    // 含义: 算法降级, value 不应被解读为真实信号
    parts.push(ScorePart {
        name: "event".into(),
        value: event_s,
        weight: 0.30,
        data_sufficiency: !inputs.ai_degraded,
    });
    parts.push(ScorePart {
        name: "chain".into(),
        value: chain_s,
        weight: 0.25,
        data_sufficiency: true,
    });
    parts.push(ScorePart {
        name: "flow".into(),
        value: flow_s,
        weight: 0.15,
        data_sufficiency: inputs.flow_score.is_some(),
    });
    parts.push(ScorePart {
        name: "cross".into(),
        value: cross_s,
        weight: 0.10,
        data_sufficiency: inputs.cross_source_count >= 2,
    });
    parts.push(ScorePart {
        name: "quality".into(),
        value: quality_s,
        weight: 0.20,
        data_sufficiency: inputs.quality_score.is_some(),
    });

    let event_risk_score: f64 = parts.iter().map(|p| p.value * p.weight).sum();

    // 修复 P0-1: data_sufficiency 计数
    let insufficient_count = parts.iter().filter(|p| !p.data_sufficiency).count();
    let data_sufficiency = DataSufficiency {
        event_risk_sufficient: insufficient_count < 2,
        // 修复 P1-2: winrate_sufficient 必真实数据, 不是 None
        winrate_sufficient: inputs.winrate_score.is_some() && inputs.winrate_score.unwrap() > 0.0,
        has_intraday_flow: inputs.flow_score.is_some(),
    };

    // BR-014 / BR-096 边界证明: 设推送门为 T，数据不足封顶 C=T-1，
    // 则 score ≤ C < T。非法阈值 fail closed 到 0 分封顶，生产门同时拒绝该配置。
    let clamp_max = push_threshold
        .checked_sub(1)
        .filter(|_| valid_push_threshold(push_threshold))
        .unwrap_or(0) as f64;
    let mut event_risk_score_clamped = event_risk_score;
    if insufficient_count >= 2 {
        event_risk_score_clamped = event_risk_score_clamped.min(clamp_max);
        notes.push(format!(
            "数据不足({} 项缺失), event_risk_score 封顶 {} (threshold-1)",
            insufficient_count, clamp_max
        ));
    }
    // 修复 R-2 (2026-06-30): 包含 Some(0.0) (零胜率也是负信号)
    let winrate_missing_or_zero = match inputs.winrate_score {
        None => true,
        Some(v) if v <= 0.0 => true,
        _ => false,
    };
    if winrate_missing_or_zero {
        event_risk_score_clamped = event_risk_score_clamped.min(clamp_max);
        if !notes
            .iter()
            .any(|n| n.contains("无回测") || n.contains("无样本") || n.contains("winrate=0"))
        {
            notes.push(format!(
                "无有效 winrate (None 或 0), event_risk_score 封顶 {} (P0-1 NS3)",
                clamp_max
            ));
        }
    }
    if !data_sufficiency.has_intraday_flow {
        notes.push("资金数据滞后/不足, flow=50 标中性".to_string());
    }

    // 修复 P0-1 + P1-2: trade_signal_score 二元化
    let trade_signal_score = match inputs.winrate_score {
        None => {
            notes.push("无历史样本回测, trade_signal=None".to_string());
            None
        }
        Some(v) if v <= 0.0 => {
            notes.push("winrate=0, 明确负信号".to_string());
            Some(0)
        }
        Some(v) => Some(((v * 100.0).clamp(0.0, 100.0)) as u8),
    };

    if inputs.winrate_score.is_none() {
        notes.push("[无回测数据]".to_string());
    }

    OpportunityScore {
        event_risk_score: event_risk_score_clamped.clamp(0.0, f64::from(EVENT_RISK_SCORE_MAX))
            as u8,
        trade_signal_score,
        parts,
        data_sufficiency,
        weight_version: weight_version.to_string(),
        notes,
    }
}

#[cfg(test)]
mod tests_r2 {
    //! 修复 R-2 (2026-06-30 codex review, AGENTS §2.9, BR-014):
    //! event_risk_score clamp 不能 > threshold (防数据不足票被 push).
    use super::*;

    fn high_inputs(winrate: Option<f64>) -> ScoreInputs {
        ScoreInputs {
            event_strength: 90,
            event_certainty: 90,
            chain_match_score: 90,
            flow_score: Some(80.0),
            cross_source_count: 2,
            quality_score: Some(80.0),
            winrate_score: winrate,
            ai_degraded: false,
        }
    }

    #[test]
    fn test_r2_clamp_uses_the_same_explicit_threshold() {
        // Case 1: 无 winrate 在 75 分推送门下必须封顶到 74。
        let s = compute_dual_score_with_threshold(&high_inputs(None), "v9.1", 75);
        assert!(
            s.event_risk_score < 75,
            "无 winrate 应严格低于同一推送门, 实际 {}",
            s.event_risk_score
        );

        // Case 2: 配置为 60 时关系仍然是 threshold-1，没有灰度旁路。
        let s = compute_dual_score_with_threshold(&high_inputs(None), "v9.1", 60);
        assert!(
            s.event_risk_score < 60,
            "无 winrate 应封顶 < 60, 实际 {}",
            s.event_risk_score
        );

        // Case 3: Some(0.0) 也触发同一封顶（零胜率 = 负信号）。
        let s = compute_dual_score_with_threshold(&high_inputs(Some(0.0)), "v9.1", 75);
        assert!(
            s.event_risk_score < 75,
            "winrate=0 应封顶 < 75, 实际 {}",
            s.event_risk_score
        );

        // Case 4: 有效 winrate (Some(0.5)) → 不 clamp
        let s = compute_dual_score_with_threshold(&high_inputs(Some(0.5)), "v9.1", 75);
        assert!(
            s.event_risk_score >= 75,
            "有效 winrate=0.5 时不应 clamp, 实际 {}",
            s.event_risk_score
        );

        // Case 5: 非法阈值 fail closed，不会因 saturating_sub 而误放行。
        let s = compute_dual_score_with_threshold(&high_inputs(None), "v9.1", 0);
        assert_eq!(s.event_risk_score, 0);
    }
}
