//! 修复 P0-3: launch_gate 阶段门槛
//!
//! 量化产品经理要求 (P0-3):
//!  - 沙盘 → 灰度: 12 周 + 200 样本 + 60% 胜率 + Calmar 1.0 (全部满足)
//!  - 灰度 → 实盘: 30 天 + 55% 胜率
//!  - 灰度 → 沙盘 (回退): 胜率 < 50%
//!  - 实盘阶段 LaunchGate::check_transition 不自动转, 只能人工/风控回退

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchStage {
    /// 沙盘: 只入 prediction_tracker 影子盘, 不推送用户 (P0-3 推荐)
    Shadow,
    /// 灰度: 单日推送 ≤ 5 候选, 限 30 天
    Gray,
    /// 实盘: 全量推送
    Live,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StageMetrics {
    /// 沙盘运行天数
    pub shadow_days: u32,
    /// winrate 真实样本数
    pub winrate_samples: u32,
    /// 真实胜率 (0-1)
    pub winrate_pct: f64,
    /// Calmar 比率 (年化收益/最大回撤)
    pub calmar_ratio: f64,
    /// 灰度运行天数
    pub gray_days: u32,
}

pub struct LaunchGate;

impl LaunchGate {
    /// 修复 P0-3: 阶段切换检查
    /// 沙盘 → 灰度: 12 周 + 200 样本 + 60% 胜率 + Calmar ≥ 1.0 (全部满足)
    /// 灰度 → 实盘: 30 天 + 55% 胜率
    /// 灰度 → 沙盘 (回退): 胜率 < 50%
    /// 实盘 → 任何: 人工/风控事件手动处理, 不自动
    pub fn check_transition(current: LaunchStage, m: &StageMetrics) -> Option<LaunchStage> {
        match current {
            LaunchStage::Shadow => {
                // 修复 P0-3: 4 个条件全部满足
                if m.shadow_days >= 60  // 12 周
                    && m.winrate_samples >= 200
                    && m.winrate_pct >= 0.60
                    && m.calmar_ratio >= 1.0
                {
                    Some(LaunchStage::Gray)
                } else {
                    None
                }
            }
            LaunchStage::Gray => {
                // 修复 P0-3: 灰度 → 实盘 (30 天 + 55% 胜率)
                if m.gray_days >= 30 && m.winrate_pct >= 0.55 {
                    Some(LaunchStage::Live)
                } else if m.winrate_pct < 0.50 {
                    // 修复 P0-3: 灰度 → 沙盘 (回退, 胜率 < 50%)
                    Some(LaunchStage::Shadow)
                } else {
                    None
                }
            }
            // 修复 P0-3: 实盘阶段不自动转, 只能人工/风控事件手动回退
            LaunchStage::Live => None,
        }
    }
}
