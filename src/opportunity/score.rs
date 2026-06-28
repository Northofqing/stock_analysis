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
//!   - data_sufficiency < 2 项 false -> event_risk_score 封顶 70
//!   - winrate 样本 < 200 -> trade_signal_score = None (不假装 50)
//!   - weight_version 落审计, 上线后可回溯
//!
//! 修复 P1-2: winrate 二元化 (None / 0 / 真实 0-100)

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScorePart {
    pub name: String,
    pub value: f64,        // 0-100
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
            cross_source_count: 1,  // 默认单源 (保守, cross_score 低)
            quality_score: None,
            winrate_score: None,
            ai_degraded: false,
        }
    }
}

/// 修复 P0-1: dual_score 计算
/// event_risk_score: 5 项加权, 无 winrate 数据时封顶 70
/// trade_signal_score: 单独 None/0/0-100
pub fn compute_dual_score(inputs: &ScoreInputs, weight_version: &str) -> OpportunityScore {
    let mut parts = Vec::new();
    let mut notes = Vec::new();

    // event_risk_score 五项 (修复 P0-1: winrate 不参与, 这是 NS3 风险评估)
    let event_s_raw = (inputs.event_strength.min(100) as f64 + inputs.event_certainty.min(100) as f64) / 2.0;
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
    parts.push(ScorePart { name: "chain".into(), value: chain_s, weight: 0.25, data_sufficiency: true });
    parts.push(ScorePart { name: "flow".into(), value: flow_s, weight: 0.15, data_sufficiency: inputs.flow_score.is_some() });
    parts.push(ScorePart { name: "cross".into(), value: cross_s, weight: 0.10, data_sufficiency: inputs.cross_source_count >= 2 });
    parts.push(ScorePart { name: "quality".into(), value: quality_s, weight: 0.20, data_sufficiency: inputs.quality_score.is_some() });

    let event_risk_score: f64 = parts.iter().map(|p| p.value * p.weight).sum();

    // 修复 P0-1: data_sufficiency 计数
    let insufficient_count = parts.iter().filter(|p| !p.data_sufficiency).count();
    let data_sufficiency = DataSufficiency {
        event_risk_sufficient: insufficient_count < 2,
        // 修复 P1-2: winrate_sufficient 必真实数据, 不是 None
        winrate_sufficient: inputs.winrate_score.is_some() && inputs.winrate_score.unwrap() > 0.0,
        has_intraday_flow: inputs.flow_score.is_some(),
    };

    // 修复 P0-1: 数据不足时封顶 70
    let mut event_risk_score_clamped = event_risk_score;
    if insufficient_count >= 2 {
        event_risk_score_clamped = event_risk_score_clamped.min(70.0);
        notes.push(format!("数据不足({} 项缺失), event_risk_score 封顶 70", insufficient_count));
    }
    // 修复 P0-1 NS3 强约束: 无 winrate 时, event_risk_score 必 ≤ 70
    // (winrate 是胜率信号, 与风险评估分轨, 但缺它意味着评分不完整, 封顶)
    if inputs.winrate_score.is_none() {
        event_risk_score_clamped = event_risk_score_clamped.min(70.0);
        if !notes.iter().any(|n| n.contains("无回测") || n.contains("无样本")) {
            notes.push("无 winrate 胜率数据, event_risk_score 封顶 70 (P0-1 NS3)".to_string());
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
        event_risk_score: event_risk_score_clamped.clamp(0.0, 100.0) as u8,
        trade_signal_score,
        parts,
        data_sufficiency,
        weight_version: weight_version.to_string(),
        notes,
    }
}
