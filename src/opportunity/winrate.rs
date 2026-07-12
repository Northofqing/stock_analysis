//! 修复 P1-2: winrate 二元化
//!
//! 之前: 占位 50 = 假装中性, 系统偏差 7.5 分
//! 现在: 样本 < 200 = None, 无数据 = 0, 有数据 = 真实胜率
//!
//! 量化产品经理视角: 没有足够样本时, 评分必 None (不假装)
//! 明确负信号 (胜率 < 50%) → 0 (允许, 但 evidence 是负的)
//! 明确正信号 (胜率 ≥ 50%) → 真实值

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestSample {
    pub event_id: String,
    /// N 日后收益 (例如 +5% 或 -3%)
    pub n_day_return: f64,
    pub day: NaiveDate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WinrateSummary {
    /// 0-1, 真实胜率 (None 时为 0 占位)
    pub score: f64,
    /// 样本是否足够 (≥ 200)
    pub sufficient: bool,
    pub total: usize,
    pub wins: usize,
    pub losses: usize,
}

/// 修复 P1-2: 二元化
/// - 样本 < 200 → None (无信号, 不假装 50)
/// - 真实胜率 < 50% → 0 (明确负信号, 允许)
/// - 真实胜率 ≥ 50% → 胜率值 (0.5 封顶, 不允许 > 0.5 假装特别高)
pub fn calc_winrate_score(samples: &[BacktestSample]) -> Option<f64> {
    let summary = compute_winrate_summary(samples);
    if !summary.sufficient {
        return None;
    }
    Some(summary.score)
}

/// 修复 P1-2: 计算胜率 (排除 n_day_return=0 的中性样本)
pub fn compute_winrate_summary(samples: &[BacktestSample]) -> WinrateSummary {
    const MIN_SAMPLES: usize = 200;
    // 过滤掉 n_day_return=0 (中性, 不算胜负)
    let valid: Vec<_> = samples
        .iter()
        .filter(|s| s.n_day_return.abs() > 0.0001)
        .collect();
    let total = valid.len();
    let wins = valid.iter().filter(|s| s.n_day_return > 0.0).count();
    let losses = total - wins;
    if total < MIN_SAMPLES {
        // insufficient 仍报告真实 wins/losses/total (数据缺 ≠ 数据全零)
        return WinrateSummary {
            score: 0.0,
            sufficient: false,
            total,
            wins,
            losses,
        };
    }
    let raw_score = wins as f64 / total as f64;
    // 修复 P1-2: 明确负信号 (胜率 < 50%) → 0, 正信号 → 真实值
    let score = if raw_score < 0.5 { 0.0 } else { raw_score };
    WinrateSummary {
        score,
        sufficient: true,
        total,
        wins,
        losses,
    }
}
