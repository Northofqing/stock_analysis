//! v12 MVP-5 §8.3: 性能反馈聚合 (三维分组: VirtualReason × FailureReason × MarketStage).
//!
//! 设计: 胜率/盈亏比/最大回撤/MFE/MAE/执行率. 样本不足显式标注.

use crate::opportunity::news_ranker::HeatStage;
use chrono::Local;

/// 三维分组
#[derive(Debug, Clone, Default)]
pub struct PerformanceGroup {
    pub sample_count: usize,
    pub win_rate: Option<f64>, // None = 样本不足不出结论
    pub avg_pnl_pct: Option<f64>,
    pub max_drawdown_pct: Option<f64>,
    pub mfe: Option<f64>, // Max Favorable Excursion
    pub mae: Option<f64>, // Max Adverse Excursion
}

impl PerformanceGroup {
    /// 样本 < 10 不作决策依据 (BR-020)
    pub fn is_decidable(&self) -> bool {
        self.sample_count >= 10
    }
    /// 样本 < 20 标"样本不足"
    pub fn is_under_sampled(&self) -> bool {
        self.sample_count < 20
    }
}

/// 三维分组结果
#[derive(Debug, Clone, Default)]
pub struct PerformanceReport {
    pub by_reason: std::collections::HashMap<String, PerformanceGroup>,
    pub by_failure: std::collections::HashMap<String, PerformanceGroup>,
    pub by_stage: std::collections::HashMap<HeatStage, PerformanceGroup>,
}

/// 渲染报告
pub fn render_performance(report: &PerformanceReport) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "📊 性能反馈报告（{}）\n",
        Local::now().format("%Y-%m-%d")
    ));
    if report.by_reason.is_empty() && report.by_failure.is_empty() && report.by_stage.is_empty() {
        s.push_str("样本不足 (三维分组都为空), 不出结论\n");
        return s;
    }
    for (key, g) in &report.by_reason {
        render_group(&mut s, &format!("理由:{}", key), g);
    }
    for (key, g) in &report.by_failure {
        render_group(&mut s, &format!("失败:{}", key), g);
    }
    for (stage, g) in &report.by_stage {
        render_group(&mut s, &format!("阶段:{}", stage.label()), g);
    }
    s
}

fn render_group(s: &mut String, key: &str, g: &PerformanceGroup) {
    if g.is_under_sampled() {
        s.push_str(&format!("  {}: 样本不足 ({})\n", key, g.sample_count));
        return;
    }
    let win = g
        .win_rate
        .map(|w| format!("{:.1}%", w * 100.0))
        .unwrap_or_else(|| "N/A".into());
    let avg = g
        .avg_pnl_pct
        .map(|p| format!("{:+.2}%", p))
        .unwrap_or_else(|| "N/A".into());
    let mfe = g
        .mfe
        .map(|m| format!("{:+.2}%", m))
        .unwrap_or_else(|| "N/A".into());
    let mae = g
        .mae
        .map(|m| format!("{:+.2}%", m))
        .unwrap_or_else(|| "N/A".into());
    s.push_str(&format!(
        "  {}: 样本{} 胜率{} 均收益{} MFE{} MAE{}\n",
        key, g.sample_count, win, avg, mfe, mae,
    ));
}

/// 聚合: 计算胜率/均收益 (从 paper_trades 历史 + prediction_tracker)
pub fn compute_group(pnls: &[f64], // 每笔虚拟腿的盈亏 (%)
) -> PerformanceGroup {
    let sample_count = pnls.len();
    if sample_count == 0 {
        return PerformanceGroup {
            sample_count: 0,
            win_rate: None,
            avg_pnl_pct: None,
            max_drawdown_pct: None,
            mfe: None,
            mae: None,
        };
    }
    let wins = pnls.iter().filter(|p| **p > 0.5).count();
    let win_rate = Some(wins as f64 / sample_count as f64);
    let avg_pnl_pct = Some(pnls.iter().sum::<f64>() / sample_count as f64);
    let mfe = pnls.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let mae = pnls.iter().cloned().fold(f64::INFINITY, f64::min);
    // 最大回撤 (累计曲线最低点)
    let mut cumulative = 0.0;
    let mut peak = 0.0;
    let mut max_dd = 0.0;
    for p in pnls {
        cumulative += p;
        if cumulative > peak {
            peak = cumulative;
        }
        let dd = peak - cumulative;
        if dd > max_dd {
            max_dd = dd;
        }
    }
    PerformanceGroup {
        sample_count,
        win_rate,
        avg_pnl_pct,
        max_drawdown_pct: Some(max_dd),
        mfe: Some(mfe),
        mae: Some(mae),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_empty() {
        let r = PerformanceReport::default();
        let s = render_performance(&r);
        assert!(s.contains("样本不足"));
    }

    #[test]
    fn compute_basic() {
        let pnls = vec![1.0, 2.0, -1.0, 3.0, -2.0];
        let g = compute_group(&pnls);
        assert_eq!(g.sample_count, 5);
        assert_eq!(g.win_rate, Some(0.6)); // 3/5
        assert!(g.mfe.unwrap() >= 3.0);
        assert!(g.mae.unwrap() <= -2.0);
        assert!(g.is_under_sampled());
    }

    #[test]
    fn is_decidable_threshold() {
        let mut g = PerformanceGroup {
            sample_count: 5,
            ..PerformanceGroup::default()
        };
        assert!(!g.is_decidable());
        g.sample_count = 10;
        assert!(g.is_decidable());
    }

    #[test]
    fn compute_max_drawdown() {
        // 涨 1, 涨 1, 跌 3 → peak=2, drawdown=3
        let pnls = vec![1.0, 1.0, -3.0];
        let g = compute_group(&pnls);
        assert!(g.max_drawdown_pct.unwrap() >= 3.0);
    }
}
