//! v11-P0-4 commit A: 持仓决策台 — 决策模型层
//!
//! ## 背景
//!
//! 之前 `bin/monitor/main.rs` 的 `build_holding_summary` 靠 `contains("规避")` 字符串猜动作
//! (B9 推送), 鲁棒性差. P0-4 用强类型 `Action` + `Priority` 替代, 让下游 `decide()` 裁决层
//! (commit B) 有稳定输入, 渲染层 (commit C) 也有清晰结构.
//!
//! ## 三层类型
//!
//! - `Action` 枚举: 4 个动作 (强类型, 编译期枚举, 序列化稳定)
//! - `Priority` 枚举: 3 个优先级 (P0 = 硬止损/风控 critical, P1 = 软止损/放量弱化, P2 = 观察/加仓)
//! - `FinalDecision` 结构: 一只持仓的最终决策 (含依据 reasons + AI 一句话摘要)
//! - `DecisionReason` 结构: 单条依据 (kind 描述来源: 止损/风控/放量/消息面, text 描述细节)
//!
//! ## 设计原则
//!
//! - AI 不进 `Action` / `Priority` 决策 (v11 IC 证伪 sentiment, 数字误导), 只在 `ai_card_summary`
//!   字段以 1-2 句文字附注呈现
//! - 强类型 + `Copy` + `Serialize`/`Deserialize` 便于落盘 + 跨 commit 传值

use serde::{Deserialize, Serialize};

/// 持仓最终动作
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    /// 立即减仓 (P0 触发: 硬止损/风控一票否决)
    ReduceNow,
    /// 逢高减仓 (P1 触发: 软止损/放量弱化/消息面利空)
    Reduce,
    /// 持有观察 (默认, 无风险信号)
    Hold,
    /// 关注加仓点 (P2 触发: 布林买点 + 无风险信号)
    WatchAdd,
}

impl Action {
    /// 简短标签 (供卡片显示, 不超过 4 汉字)
    pub fn label(self) -> &'static str {
        match self {
            Action::ReduceNow => "立即减仓",
            Action::Reduce => "逢高减仓",
            Action::Hold => "持有观察",
            Action::WatchAdd => "关注加仓",
        }
    }

    /// emoji 标记 (供卡片显示)
    pub fn emoji(self) -> &'static str {
        match self {
            Action::ReduceNow => "🔴",
            Action::Reduce => "🟡",
            Action::Hold => "🟢",
            Action::WatchAdd => "🔵",
        }
    }
}

/// 优先级
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum Priority {
    /// 硬止损 / 风控一票否决, critical 推送
    P0,
    /// 软止损 / 放量弱化 / 消息面利空
    P1,
    /// 观察 / 加仓
    P2,
}

impl Priority {
    /// P0 标签
    pub fn label(self) -> &'static str {
        match self {
            Priority::P0 => "P0",
            Priority::P1 => "P1",
            Priority::P2 => "P2",
        }
    }
}

/// 依据来源 (用于 reasons[].kind 字段, 渲染时按 kind 排序/着色)
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DecisionReasonKind {
    /// 硬/技术/结构止损 (risk::stop_loss::check_stops)
    StopLoss,
    /// 单票/板块/止损线 (risk::limits::check_position_limits)
    PositionLimit,
    /// 放量形态 (放量分析)
    VolumePattern,
    /// 消息面持仓影响 (产业链持仓影响)
    NewsImpact,
    /// 持仓现价/涨跌 (健康度数据)
    Health,
}

impl DecisionReasonKind {
    pub fn label(self) -> &'static str {
        match self {
            DecisionReasonKind::StopLoss => "止损",
            DecisionReasonKind::PositionLimit => "风控",
            DecisionReasonKind::VolumePattern => "放量",
            DecisionReasonKind::NewsImpact => "消息面",
            DecisionReasonKind::Health => "现价",
        }
    }
}

/// 单条决策依据
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionReason {
    pub kind: DecisionReasonKind,
    pub text: String,
}

impl DecisionReason {
    pub fn new(kind: DecisionReasonKind, text: impl Into<String>) -> Self {
        Self { kind, text: text.into() }
    }
}

/// 一只持仓的最终决策
///
/// 落盘 JSON 用于审计 + 跨 commit 传值. 卡片渲染 (commit C) 直接消费.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FinalDecision {
    /// 股票代码 (6 位)
    pub code: String,
    /// 股票名称
    pub name: String,
    /// 当前价
    pub current_price: f64,
    /// 今日涨跌幅 (%)
    pub change_pct: f64,
    /// 最终动作
    pub action: Action,
    /// 优先级
    pub priority: Priority,
    /// 决策依据 (按重要性排序, 渲染时按此顺序展示)
    pub reasons: Vec<DecisionReason>,
    /// 硬止损价 (None 表示无止损信号)
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stop_loss: Option<f64>,
    /// AI 1-2 句摘要 (grill Q4 决策: 不含分数, 仅文字; v11 IC 证伪 sentiment)
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ai_card_summary: Option<String>,
}

impl FinalDecision {
    /// 构造简单决策 (无 AI 摘要)
    pub fn new(
        code: impl Into<String>,
        name: impl Into<String>,
        current_price: f64,
        change_pct: f64,
        action: Action,
        priority: Priority,
        reasons: Vec<DecisionReason>,
    ) -> Self {
        Self {
            code: code.into(),
            name: name.into(),
            current_price,
            change_pct,
            action,
            priority,
            reasons,
            stop_loss: None,
            ai_card_summary: None,
        }
    }

    /// 链式 builder: 加 AI 摘要
    pub fn with_ai_summary(mut self, summary: impl Into<String>) -> Self {
        self.ai_card_summary = Some(summary.into());
        self
    }

    /// 链式 builder: 设止损价
    pub fn with_stop_loss(mut self, stop_loss: f64) -> Self {
        self.stop_loss = Some(stop_loss);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Action 4 个变体都唯一 + Copy + Serialize
    #[test]
    fn action_variants_distinct() {
        let actions = vec![
            Action::ReduceNow,
            Action::Reduce,
            Action::Hold,
            Action::WatchAdd,
        ];
        let mut sorted = actions;
        sorted.sort_by_key(|a| format!("{:?}", a));
        sorted.dedup();
        assert_eq!(sorted.len(), 4, "Action 4 个变体必须唯一");
    }

    /// Priority 3 个变体, Ord 实现保证 P0 < P1 < P2
    #[test]
    fn priority_ord() {
        assert!(Priority::P0 < Priority::P1);
        assert!(Priority::P1 < Priority::P2);
        assert!(Priority::P0 < Priority::P2);
    }

    /// Action 简短标签 ≤ 4 汉字
    #[test]
    fn action_label_short() {
        for a in [Action::ReduceNow, Action::Reduce, Action::Hold, Action::WatchAdd] {
            let l = a.label();
            assert!(l.chars().count() <= 4, "Action 标签应 ≤ 4 汉字, 实际: '{}'", l);
        }
    }

    /// FinalDecision JSON 序列化 / 反序列化 round-trip (落盘审计需要)
    #[test]
    fn final_decision_serde_roundtrip() {
        let original = FinalDecision::new(
            "000001",
            "平安银行",
            12.30,
            -1.5,
            Action::Reduce,
            Priority::P1,
            vec![
                DecisionReason::new(DecisionReasonKind::StopLoss, "硬止损触发 ¥12.10"),
                DecisionReason::new(DecisionReasonKind::VolumePattern, "放量冲高回落"),
            ],
        )
        .with_ai_summary("技术破位, 主力净流出 3.5亿")
        .with_stop_loss(12.10);

        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: FinalDecision = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.code, "000001");
        assert_eq!(parsed.action, Action::Reduce);
        assert_eq!(parsed.priority, Priority::P1);
        assert_eq!(parsed.reasons.len(), 2);
        assert_eq!(parsed.reasons[0].kind, DecisionReasonKind::StopLoss);
        assert_eq!(parsed.stop_loss, Some(12.10));
        assert_eq!(parsed.ai_card_summary.as_deref(), Some("技术破位, 主力净流出 3.5亿"));
    }

    /// FinalDecision 链式 builder
    #[test]
    fn final_decision_builder() {
        let d = FinalDecision::new("600519", "贵州茅台", 1500.0, 0.5, Action::Hold, Priority::P2, vec![])
            .with_ai_summary("无明显信号, 持有观察");
        assert_eq!(d.ai_card_summary.as_deref(), Some("无明显信号, 持有观察"));
        assert_eq!(d.stop_loss, None, "未设止损价应 None");
    }

    /// AI card_summary None 序列化时跳过 (落盘 JSON 不含该字段)
    #[test]
    fn ai_summary_none_skipped_in_json() {
        let d = FinalDecision::new("000001", "test", 10.0, 0.0, Action::Hold, Priority::P2, vec![]);
        let json = serde_json::to_string(&d).expect("serialize");
        assert!(!json.contains("ai_card_summary"), "None 时应跳过该字段");
        assert!(!json.contains("stop_loss"), "None 时应跳过该字段");
    }
}
