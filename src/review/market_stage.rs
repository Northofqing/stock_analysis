//! v12 MVP-4 §7.1: MarketStage 阶段判定.
//!
//! 设计: review 域自有的 HeatStage 七态 + confidence + effective_stage().
//! - effective_stage() < 0.6 取相邻保守档 (Unknown→Cold).
//! - 盘后 R-02 计算落 account_mode_log 快照, 盘中 AccountMode 只读昨日快照.
//! - 阶段→权限映射表 (§6.2) 接 AccountMode 建议.

/// 复盘阶段词汇。
///
/// BR-191: 这只是 review 域的显式输入/渲染类型，不从已退役的本地
/// sector-history JSONL 或默认市场上下文推断。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum HeatStage {
    Cold,
    Start,
    Ferment,
    Climax,
    Divergence,
    Fade,
    Unknown,
}

impl HeatStage {
    pub fn label(self) -> &'static str {
        match self {
            HeatStage::Cold => "冷",
            HeatStage::Start => "启动",
            HeatStage::Ferment => "发酵",
            HeatStage::Climax => "高潮",
            HeatStage::Divergence => "分歧",
            HeatStage::Fade => "退潮",
            HeatStage::Unknown => "未知",
        }
    }
}

/// 阶段 + 置信度
#[derive(Debug, Clone)]
pub struct MarketStage {
    pub stage: HeatStage,
    pub confidence: f64,
}

impl MarketStage {
    /// 有效阶段: 置信度 <0.6 取相邻保守档; Unknown→Cold.
    pub fn effective_stage(&self) -> HeatStage {
        if self.confidence >= 0.6 {
            return self.stage;
        }
        match self.stage {
            HeatStage::Climax | HeatStage::Divergence => HeatStage::Ferment, // 保守: 视为发酵
            HeatStage::Unknown => HeatStage::Cold,
            other => other, // Start/Ferment/Fade 保留
        }
    }
}

/// 阶段→AccountMode 建议 (v12 §6.2 表).
pub fn stage_to_account_mode(stage: HeatStage) -> &'static str {
    match stage {
        HeatStage::Start | HeatStage::Ferment => "Normal (板块起势, 可常规操作)",
        HeatStage::Cold => "Normal (盘面冷, 默认保守)",
        HeatStage::Climax => "ReduceOnly (高潮分歧, 减仓优先)",
        HeatStage::Divergence => "ReduceOnly (背离, 减仓优先)",
        HeatStage::Fade => "Frozen (退潮, 禁止新开仓)",
        HeatStage::Unknown => "ReduceOnly (阶段未知, 保守)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_stage_high_confidence_unchanged() {
        let m = MarketStage {
            stage: HeatStage::Climax,
            confidence: 0.8,
        };
        assert_eq!(m.effective_stage(), HeatStage::Climax);
    }

    #[test]
    fn effective_stage_low_confidence_climax_becomes_ferment() {
        let m = MarketStage {
            stage: HeatStage::Climax,
            confidence: 0.4,
        };
        assert_eq!(m.effective_stage(), HeatStage::Ferment);
    }

    #[test]
    fn effective_stage_low_confidence_unknown_becomes_cold() {
        let m = MarketStage {
            stage: HeatStage::Unknown,
            confidence: 0.3,
        };
        assert_eq!(m.effective_stage(), HeatStage::Cold);
    }

    #[test]
    fn stage_to_account_mode_fade_is_frozen() {
        assert!(stage_to_account_mode(HeatStage::Fade).contains("Frozen"));
    }

    #[test]
    fn stage_to_account_mode_start_is_normal() {
        assert!(stage_to_account_mode(HeatStage::Start).contains("Normal"));
    }
}
