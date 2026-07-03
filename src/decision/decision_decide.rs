//! v11-P0-4 commit B: 持仓决策台 — 裁决层
//!
//! ## 背景
//!
//! Commit A 落了 `Action` / `Priority` / `FinalDecision` 类型. Commit B 落 **裁决规则**:
//! 输入 `DecisionInputs` (止损信号 / 风控违规 / 放量弱化 / 消息面利空 + 现价/涨跌幅),
//! 按 5 层规则 + 冲突裁决输出 `FinalDecision`.
//!
//! ## 5 层规则 (P0-4 §3.4, 风控一票否决)
//!
//! - 层1: 硬止损 (StopLevel::Hard) **或** 风控超标 (LimitViolation) → ReduceNow (P0)
//! - 层2: 技术/结构止损 (StopLevel::Technical/Structural) → Reduce (P1)
//! - 层3: 放量冲高回落/尾盘跳水 (volume_weak) → Reduce (P1)
//! - 层4: 消息面重大利空 (news_negative) → Reduce (P1)
//! - 层5: 无风险信号 + 布林买点 (boll_buy_point) → WatchAdd (P2)
//! - 默认: 无信号 → Hold (P2) — 诚实标注, 不假装中性
//!
//! ## 冲突裁决
//!
//! 层1 一票否决: 即使层5 布林买点信号, 硬止损/风控超标仍优先 ReduceNow.
//! 多层同时触发: 取最严重层 (层1 > 层2 > 层3/4 > 层5 > 默认).
//!
//! ## AI 隔离 (grill Q4 决策)
//!
//! `ai_card_summary` 是输入参数, **不进** `decide()` 裁决逻辑. AI 文字只在卡片渲染时附注,
//! 不影响 Action/Priority. v11 IC 证伪 sentiment_score, 数字误导, 排除.

use crate::decision::decision_panel::{
    Action, DecisionReason, DecisionReasonKind, FinalDecision, Priority,
};
use crate::risk::limits::LimitViolation;
use crate::risk::stop_loss::{StopLevel, StopSignal};

/// 持仓决策输入
///
/// 喂入 `decide()` 的一只持仓所有信号. AI 文字 (ai_card_summary) 不参与裁决, 只在卡片附注.
#[derive(Clone, Debug)]
pub struct DecisionInputs {
    /// 股票代码 (6 位)
    pub code: String,
    /// 股票名称
    pub name: String,
    /// 当前价
    pub current_price: f64,
    /// 今日涨跌幅 (%)
    pub change_pct: f64,
    /// 止损信号 (risk::stop_loss::check_stops 输出, 可能空)
    pub stop_signals: Vec<StopSignal>,
    /// 风控违规 (risk::limits::check_position_limits 输出, 可能空)
    pub limit_violations: Vec<LimitViolation>,
    /// 放量冲高回落/尾盘跳水 (B5 推送分析出的 bool)
    pub volume_weak: bool,
    /// 消息面重大利空 (C5 推送的 bool)
    pub news_negative: bool,
    /// 布林买点 (放量分析/技术面)
    pub boll_buy_point: bool,
    /// 硬止损价 (None 表示未设)
    pub hard_stop: Option<f64>,
    /// AI 1-2 句摘要 (grill Q4: 不进裁决, 仅卡片附注)
    pub ai_card_summary: Option<String>,
}

/// 5 层规则 + 冲突裁决
///
/// 风控一票否决: 硬止损或风控超标 → ReduceNow (P0), 即使有布林买点也压过.
/// 优先级: 层1 > 层2 > 层3/层4 > 层5 > 默认.
///
/// 各层 reasons 独立收集 (信息完整性), 只在最后取最严重层做 action/priority.
pub fn decide(inputs: DecisionInputs) -> FinalDecision {
    let mut reasons: Vec<DecisionReason> = Vec::new();
    let mut triggered_layer: Option<u8> = None; // 1-5, None = 默认 Hold, 数字越小越严重

    // 层1a: 硬止损 → ReduceNow (P0)
    if let Some(s) = inputs
        .stop_signals
        .iter()
        .find(|s| s.level == StopLevel::Hard)
    {
        reasons.push(DecisionReason::new(
            DecisionReasonKind::StopLoss,
            format!(
                "硬止损触发 ¥{:.2} (现价 ¥{:.2})",
                s.trigger_price, s.current_price
            ),
        ));
        triggered_layer = Some(1);
    }

    // 层1b: 风控超标 → ReduceNow (P0)
    if let Some(v) = inputs.limit_violations.first() {
        reasons.push(DecisionReason::new(
            DecisionReasonKind::PositionLimit,
            format!("风控超标: {} ({} > {})", v.rule, v.current, v.limit),
        ));
        // 风控一票否决, 即使层1a 硬止损已触发也保持层1
        triggered_layer = Some(1);
    }

    // 层2: 技术/结构止损 → Reduce (P1) (独立检查, 不管层1 是否触发)
    if let Some(s) = inputs
        .stop_signals
        .iter()
        .find(|s| s.level == StopLevel::Technical || s.level == StopLevel::Structural)
    {
        reasons.push(DecisionReason::new(
            DecisionReasonKind::StopLoss,
            format!("{}触发 ¥{:.2}", s.level.label(), s.trigger_price),
        ));
        // 仅当层1 未触发时才设为层2 (层1 仍压过)
        if triggered_layer.is_none() || triggered_layer > Some(2) {
            triggered_layer = Some(2);
        }
    }

    // 层3: 放量冲高回落/尾盘跳水 → Reduce (P1)
    if inputs.volume_weak {
        reasons.push(DecisionReason::new(
            DecisionReasonKind::VolumePattern,
            "放量冲高回落/尾盘跳水 (已验证形态)",
        ));
        if triggered_layer.is_none() || triggered_layer > Some(3) {
            triggered_layer = Some(3);
        }
    }

    // 层4: 消息面重大利空 → Reduce (P1)
    if inputs.news_negative {
        reasons.push(DecisionReason::new(
            DecisionReasonKind::NewsImpact,
            "消息面重大利空 (C5)",
        ));
        if triggered_layer.is_none() || triggered_layer > Some(4) {
            triggered_layer = Some(4);
        }
    }

    // 层5: 布林买点 + 无风险信号 → WatchAdd (P2)
    if inputs.boll_buy_point
        && !inputs.volume_weak
        && !inputs.news_negative
        && inputs.stop_signals.is_empty()
        && inputs.limit_violations.is_empty()
    {
        reasons.push(DecisionReason::new(
            DecisionReasonKind::VolumePattern,
            "布林买点 + 无风险信号",
        ));
        // 层5 仅在层1-4 都未触发时
        if triggered_layer.is_none() {
            triggered_layer = Some(5);
        }
    }

    // 默认: 无信号 → Hold (P2) 诚实标注
    if reasons.is_empty() {
        reasons.push(DecisionReason::new(
            DecisionReasonKind::Health,
            "无风险信号, 持有观察 (默认)",
        ));
    }

    // 映射层 → (Action, Priority)
    let (action, priority) = match triggered_layer {
        Some(1) => (Action::ReduceNow, Priority::P0),
        Some(2) | Some(3) | Some(4) => (Action::Reduce, Priority::P1),
        Some(5) => (Action::WatchAdd, Priority::P2),
        None => (Action::Hold, Priority::P2),
        _ => unreachable!(),
    };

    // 构造 FinalDecision (grill Q4: AI 摘要通过 with_ai_summary 链式)
    let mut d = FinalDecision::new(
        inputs.code,
        inputs.name,
        inputs.current_price,
        inputs.change_pct,
        action,
        priority,
        reasons,
    );
    if let Some(stop) = inputs.hard_stop {
        d = d.with_stop_loss(stop);
    }
    if let Some(ai) = inputs.ai_card_summary {
        d = d.with_ai_summary(ai);
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::decision_panel::Action;

    fn default_inputs() -> DecisionInputs {
        DecisionInputs {
            code: "000001".to_string(),
            name: "test".to_string(),
            current_price: 10.0,
            change_pct: 0.0,
            stop_signals: vec![],
            limit_violations: vec![],
            volume_weak: false,
            news_negative: false,
            boll_buy_point: false,
            hard_stop: None,
            ai_card_summary: None,
        }
    }

    /// 默认: 无信号 → Hold (P2)
    #[test]
    fn layer_default_hold() {
        let d = decide(default_inputs());
        assert_eq!(d.action, Action::Hold);
        assert_eq!(d.priority, Priority::P2);
        assert_eq!(d.reasons.len(), 1);
        assert_eq!(d.reasons[0].kind, DecisionReasonKind::Health);
    }

    /// 层1 硬止损 → ReduceNow (P0), 风控一票否决
    #[test]
    fn layer1_hard_stop_reduce_now() {
        let mut inputs = default_inputs();
        inputs.stop_signals.push(StopSignal {
            code: "000001".to_string(),
            name: "test".to_string(),
            level: StopLevel::Hard,
            current_price: 10.0,
            trigger_price: 12.0,
            reason: "硬止损".to_string(),
        });
        let d = decide(inputs);
        assert_eq!(d.action, Action::ReduceNow);
        assert_eq!(d.priority, Priority::P0);
    }

    /// 层1 风控超标 → ReduceNow (P0)
    #[test]
    fn layer1_limit_violation_reduce_now() {
        let mut inputs = default_inputs();
        inputs.limit_violations.push(LimitViolation {
            code: "000001".to_string(),
            name: "test".to_string(),
            rule: "单票仓位上限".to_string(),
            current: "15.0%".to_string(),
            limit: "≤10%".to_string(),
        });
        let d = decide(inputs);
        assert_eq!(d.action, Action::ReduceNow);
        assert_eq!(d.priority, Priority::P0);
    }

    /// 层2 技术/结构止损 → Reduce (P1)
    #[test]
    fn layer2_tech_stop_reduce() {
        let mut inputs = default_inputs();
        inputs.stop_signals.push(StopSignal {
            code: "000001".to_string(),
            name: "test".to_string(),
            level: StopLevel::Technical,
            current_price: 10.0,
            trigger_price: 10.5,
            reason: "破 MA20".to_string(),
        });
        let d = decide(inputs);
        assert_eq!(d.action, Action::Reduce);
        assert_eq!(d.priority, Priority::P1);
    }

    /// 层3 放量弱化 → Reduce (P1)
    #[test]
    fn layer3_volume_weak_reduce() {
        let mut inputs = default_inputs();
        inputs.volume_weak = true;
        let d = decide(inputs);
        assert_eq!(d.action, Action::Reduce);
        assert_eq!(d.priority, Priority::P1);
    }

    /// 层4 消息面利空 → Reduce (P1)
    #[test]
    fn layer4_news_negative_reduce() {
        let mut inputs = default_inputs();
        inputs.news_negative = true;
        let d = decide(inputs);
        assert_eq!(d.action, Action::Reduce);
        assert_eq!(d.priority, Priority::P1);
    }

    /// 层5 布林买点 + 无风险 → WatchAdd (P2)
    #[test]
    fn layer5_boll_buy_watch_add() {
        let mut inputs = default_inputs();
        inputs.boll_buy_point = true;
        let d = decide(inputs);
        assert_eq!(d.action, Action::WatchAdd);
        assert_eq!(d.priority, Priority::P2);
    }

    /// 冲突: 层1 硬止损 + 层5 布林买点 → 层1 (P0 一票否决)
    #[test]
    fn layer1_overrides_layer5() {
        let mut inputs = default_inputs();
        inputs.stop_signals.push(StopSignal {
            code: "000001".to_string(),
            name: "test".to_string(),
            level: StopLevel::Hard,
            current_price: 10.0,
            trigger_price: 12.0,
            reason: "硬止损".to_string(),
        });
        inputs.boll_buy_point = true; // 试图 WatchAdd, 但应被层1 压过
        let d = decide(inputs);
        assert_eq!(d.action, Action::ReduceNow, "层1 一票否决, 即使层5 布林买点也应是 ReduceNow");
        assert_eq!(d.priority, Priority::P0);
    }

    /// AI 摘要: 进 final_decision.ai_card_summary, 不影响 action/priority
    #[test]
    fn ai_summary_does_not_change_decision() {
        let mut inputs = default_inputs();
        inputs.ai_card_summary = Some("强烈卖出 (composite=28)".to_string());
        let d = decide(inputs);
        assert_eq!(d.action, Action::Hold); // 默认 Hold, AI 文字不影响
        assert_eq!(d.priority, Priority::P2);
        assert_eq!(
            d.ai_card_summary.as_deref(),
            Some("强烈卖出 (composite=28)")
        );
    }

    /// 优先级顺序: 层1 > 层2 (即使同时触发, 取层1)
    #[test]
    fn priority_layer1_beats_layer2() {
        let mut inputs = default_inputs();
        inputs.stop_signals.push(StopSignal {
            code: "000001".to_string(),
            name: "test".to_string(),
            level: StopLevel::Technical, // 层2
            current_price: 10.0,
            trigger_price: 10.5,
            reason: "技术".to_string(),
        });
        inputs.limit_violations.push(LimitViolation {
            code: "000001".to_string(),
            name: "test".to_string(),
            rule: "单票仓位上限".to_string(),
            current: "15%".to_string(),
            limit: "≤10%".to_string(),
        });
        let d = decide(inputs);
        // 两个 reason (技术止损 + 风控超标), 但 action/priority 是层1 (P0)
        assert_eq!(d.action, Action::ReduceNow);
        assert_eq!(d.priority, Priority::P0);
        assert_eq!(d.reasons.len(), 2, "两个 reason 都要保留: 技术止损 + 风控超标");
    }
}
