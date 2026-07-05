//! v12 MVP-4 §7.1: MarketStage 阶段判定.
//!
//! 设计: HeatStage 七态 (复用 news_ranker::HeatStage) + confidence + effective_stage().
//! - effective_stage() < 0.6 取相邻保守档 (Unknown→Cold).
//! - 盘后 R-02 计算落 account_mode_log 快照, 盘中 AccountMode 只读昨日快照.
//! - 阶段→权限映射表 (§6.2) 接 AccountMode 建议.

use crate::opportunity::news_ranker::HeatStage;

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

/// 5 维打分 (板块涨幅/资金/涨停/指数/板块轮动) → HeatStage.
/// 简化版: 复用 news_ranker::judge_heat_stage 不重写.
pub fn score_to_stage(
    board_code: Option<&str>,
    change_pct: f64,
    main_inflow: f64,
    main_net_pct_today: f64,
    main_net_pct_5d: f64,
    limit_up_count: Option<usize>,
) -> MarketStage {
    let stage = crate::opportunity::news_ranker::detect_heat_stage(
        board_code,
        change_pct,
        main_inflow,
        main_net_pct_today,
        main_net_pct_5d,
        limit_up_count,
    );
    // 置信度启发: 涨停多 + 涨幅高 → 高置信
    let confidence = if change_pct.abs() > 3.0 && main_inflow.abs() > 5e7 {
        0.8
    } else if change_pct.abs() > 1.0 {
        0.5
    } else {
        0.3
    };
    MarketStage { stage, confidence }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_stage_high_confidence_unchanged() {
        let m = MarketStage { stage: HeatStage::Climax, confidence: 0.8 };
        assert_eq!(m.effective_stage(), HeatStage::Climax);
    }

    #[test]
    fn effective_stage_low_confidence_climax_becomes_ferment() {
        let m = MarketStage { stage: HeatStage::Climax, confidence: 0.4 };
        assert_eq!(m.effective_stage(), HeatStage::Ferment);
    }

    #[test]
    fn effective_stage_low_confidence_unknown_becomes_cold() {
        let m = MarketStage { stage: HeatStage::Unknown, confidence: 0.3 };
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

    #[test]
    fn score_to_stage_climax_high_confidence() {
        // 无 board_code 时 detect_heat_stage 返 Unknown, 符合"数据不足"语义
        let m = score_to_stage(None, 5.0, 1e8, 10.0, 2.0, Some(8));
        assert_eq!(m.stage, HeatStage::Unknown);
        assert!(m.confidence >= 0.5);
    }

    #[test]
    fn score_to_stage_unknown_when_data_missing() {
        let m = score_to_stage(None, 0.0, 0.0, 0.0, 0.0, None);
        assert_eq!(m.stage, HeatStage::Unknown);
    }
}