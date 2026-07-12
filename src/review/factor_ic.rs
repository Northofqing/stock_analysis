//! 因子 IC (Information Coefficient) 分析。
//!
//! 用于诊断 AI 综合评分 (sentiment_score) 及各维度因子与未来收益的相关性。
//! 使用 Spearman Rank IC (秩相关系数)，更稳健，不要求正态分布。
//!
//! ## 数据来源
//!
//! - `stock_position` 表: 已平仓交易的买入价/卖出价/收益率
//! - `analysis_result` 表: 买入日的 score_breakdown_json (5 维度评分)
//!
//! ## 输出
//!
//! - 各因子的 IC 序列、累计 IC、IR (Information Ratio)
//! - 因子方向性诊断 (IC 为负 → 因子与收益反向)
//! - IC 衰减曲线 (lag 1/2/3/4)
//! - 因子间相关性矩阵

use log::{info, warn};
use serde::{Deserialize, Serialize};

// ============================================================================
// 数据结构
// ============================================================================

/// 单因子 IC 分析结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactorIC {
    /// 因子名称
    pub factor_name: String,
    /// Rank IC 均值
    pub mean_ic: f64,
    /// Information Ratio = mean(IC) / std(IC)
    pub information_ratio: f64,
    /// IC 标准差
    pub ic_std: f64,
    /// IC > 0 的比例 (胜率)
    pub ic_win_rate: f64,
    /// t 统计量 = mean(IC) / (std(IC) / √n)
    pub t_stat: f64,
    /// IC 衰减: lag=1,2,3,4 的 IC 均值
    pub ic_decay: [f64; 4],
    /// 有效样本期数
    pub sample_periods: usize,
    /// 诊断结论
    pub verdict: FactorVerdict,
}

/// 因子方向性诊断
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FactorVerdict {
    /// IC 显著为正 → 因子有效 (正向预测)
    Positive { significant: bool },
    /// IC 接近 0 → 因子无效
    Neutral,
    /// IC 显著为负 → 因子反向 (越高越差)
    Negative { significant: bool },
}

impl FactorVerdict {
    pub fn label(&self) -> &'static str {
        match self {
            FactorVerdict::Positive { significant: true } => "🟢 显著正向",
            FactorVerdict::Positive { significant: false } => "🟡 弱正向",
            FactorVerdict::Neutral => "⚪ 无效",
            FactorVerdict::Negative { significant: false } => "🟠 弱反向",
            FactorVerdict::Negative { significant: true } => "🔴 显著反向",
        }
    }
}

/// IC 分析输入: (因子值, 前向收益) 配对
#[derive(Debug, Clone)]
pub struct FactorSample {
    /// 因子名称
    pub factor_name: String,
    /// 因子值序列
    pub factor_values: Vec<f64>,
    /// 对应的前向收益序列 (T+N 收益率, %)
    pub forward_returns: Vec<f64>,
}

// ============================================================================
// Spearman Rank IC 计算
// ============================================================================

/// 计算 Spearman Rank IC。
///
/// 步骤:
/// 1. 分别对 factor_values 和 forward_returns 做秩转换
/// 2. 计算秩的 Pearson 相关系数
///
/// 返回 None 表示样本不足 (< 30)。
pub fn compute_spearman_ic(factor_values: &[f64], forward_returns: &[f64]) -> Option<f64> {
    let n = factor_values.len().min(forward_returns.len());
    // Spearman Rank IC 在 15+ 样本时即可靠
    if n < 15 {
        warn!("样本量不足 {} (需要 ≥15)，无法可靠计算 IC", n);
        return None;
    }

    // 去除 NaN/Inf
    let pairs: Vec<(f64, f64)> = factor_values
        .iter()
        .zip(forward_returns.iter())
        .take(n)
        .filter(|(f, r)| f.is_finite() && r.is_finite())
        .map(|(f, r)| (*f, *r))
        .collect();

    if pairs.len() < 15 {
        return None;
    }

    let m = pairs.len();

    // Rank the factor values
    let mut factor_ranked: Vec<(usize, f64)> = pairs
        .iter()
        .enumerate()
        .map(|(i, (f, _))| (i, *f))
        .collect();
    factor_ranked.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut factor_ranks = vec![0.0; m];
    for (rank, (orig_idx, _)) in factor_ranked.iter().enumerate() {
        factor_ranks[*orig_idx] = (rank + 1) as f64;
    }
    // 处理并列: 取平均秩
    let mut i = 0;
    while i < m {
        let mut j = i + 1;
        while j < m && (factor_ranked[j].1 - factor_ranked[i].1).abs() < 1e-10 {
            j += 1;
        }
        if j > i + 1 {
            let avg_rank: f64 = factor_ranked[i..j]
                .iter()
                .enumerate()
                .map(|(_k, (orig_idx, _))| factor_ranks[*orig_idx])
                .sum::<f64>()
                / (j - i) as f64;
            for (_, (orig_idx, _)) in factor_ranked[i..j].iter().enumerate() {
                factor_ranks[*orig_idx] = avg_rank;
            }
        }
        i = j;
    }

    // Rank the returns
    let mut return_ranked: Vec<(usize, f64)> = pairs
        .iter()
        .enumerate()
        .map(|(i, (_, r))| (i, *r))
        .collect();
    return_ranked.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut return_ranks = vec![0.0; m];
    for (rank, (orig_idx, _)) in return_ranked.iter().enumerate() {
        return_ranks[*orig_idx] = (rank + 1) as f64;
    }
    // 处理并列
    let mut i = 0;
    while i < m {
        let mut j = i + 1;
        while j < m && (return_ranked[j].1 - return_ranked[i].1).abs() < 1e-10 {
            j += 1;
        }
        if j > i + 1 {
            let avg_rank: f64 = return_ranked[i..j]
                .iter()
                .enumerate()
                .map(|(_k, (orig_idx, _))| return_ranks[*orig_idx])
                .sum::<f64>()
                / (j - i) as f64;
            for (_, (orig_idx, _)) in return_ranked[i..j].iter().enumerate() {
                return_ranks[*orig_idx] = avg_rank;
            }
        }
        i = j;
    }

    // Pearson correlation of ranks
    let mean_fr = factor_ranks.iter().sum::<f64>() / m as f64;
    let mean_rr = return_ranks.iter().sum::<f64>() / m as f64;

    let cov: f64 = factor_ranks
        .iter()
        .zip(return_ranks.iter())
        .map(|(f, r)| (f - mean_fr) * (r - mean_rr))
        .sum();

    let std_f: f64 = factor_ranks
        .iter()
        .map(|f| (f - mean_fr).powi(2))
        .sum::<f64>()
        .sqrt();
    let std_r: f64 = return_ranks
        .iter()
        .map(|r| (r - mean_rr).powi(2))
        .sum::<f64>()
        .sqrt();

    if std_f < 1e-10 || std_r < 1e-10 {
        return Some(0.0);
    }

    let ic = cov / (std_f * std_r);
    // Clamp to [-1, 1] for numerical safety
    Some(ic.clamp(-1.0, 1.0))
}

// ============================================================================
// 滚动 IC 计算
// ============================================================================

/// 对时序数据计算滚动 IC 序列 (滚动窗口 forward-looking)。
///
/// factor_series[t] 对应 forward_returns[t] (T+N 收益)。
///
/// 返回 (ic_series, ic_decay[4])。
pub fn compute_rolling_ic(
    factor_values: &[f64],
    forward_returns: &[f64],
) -> Option<(Vec<f64>, [f64; 4])> {
    let n = factor_values.len().min(forward_returns.len());
    if n < 15 {
        return None;
    }

    // 过滤 NaN/Inf, 保留有效的 (factor, return) 配对
    let clean: Vec<(f64, f64)> = factor_values
        .iter()
        .zip(forward_returns.iter())
        .take(n)
        .filter(|(f, r)| f.is_finite() && r.is_finite())
        .map(|(f, r)| (*f, *r))
        .collect();

    if clean.len() < 15 {
        return None;
    }

    // 提取纯值序列
    let clean_factors: Vec<f64> = clean.iter().map(|(f, _)| *f).collect();
    let clean_returns: Vec<f64> = clean.iter().map(|(_, r)| *r).collect();

    // 使用滚动窗口计算 IC 序列 (至少 15 样本, 最多 20)
    let window = 15.max(clean.len() / 4).min(20);
    let mut ic_series = Vec::new();

    for start in 0..=clean.len().saturating_sub(window) {
        let end = start + window;
        let window_factors: Vec<f64> = clean_factors[start..end].to_vec();
        let window_returns: Vec<f64> = clean_returns[start..end].to_vec();
        if let Some(ic) = compute_spearman_ic(&window_factors, &window_returns) {
            ic_series.push(ic);
        }
    }

    if ic_series.is_empty() {
        return None;
    }

    // IC 衰减: lag 1-4 (因子值超前于收益的滞后期)
    let mut decay = [0.0; 4];
    for lag in 0..4 {
        let offset = lag + 1;
        if clean.len() > offset {
            let lag_factors: Vec<f64> = clean[..clean.len() - offset]
                .iter()
                .map(|(f, _)| *f)
                .collect();
            let lag_returns: Vec<f64> = clean[offset..].iter().map(|(_, r)| *r).collect();
            let m = lag_factors.len().min(lag_returns.len());
            if m >= 15 {
                if let Some(ic) = compute_spearman_ic(&lag_factors[..m], &lag_returns[..m]) {
                    decay[lag] = ic;
                }
            }
        }
    }

    Some((ic_series, decay))
}

/// 从数据库读取已平仓交易和分析结果，运行因子 IC 分析。
///
/// 通过 DatabaseManager::get_factor_ic_data() JOIN stock_position + analysis_result。
/// 用于 `--review` 复盘路径。
pub fn run_diagnostic() -> Option<String> {
    use crate::database::DatabaseManager;
    use crate::review::factor_report::generate_report;

    let db = DatabaseManager::get();
    let rows = db.get_factor_ic_data().ok()?;

    if rows.len() < 30 {
        info!(
            "[FactorIC] 有效已平仓交易 {} 笔 < 30，样本不足，跳过分析",
            rows.len()
        );
        return None;
    }

    info!("[FactorIC] 读取 {} 笔已平仓交易用于因子分析", rows.len());

    let mut technical_vals = Vec::new();
    let mut quality_vals = Vec::new();
    let mut valuation_vals = Vec::new();
    let mut flow_vals = Vec::new();
    let mut growth_vals = Vec::new();
    let mut sentiment_vals = Vec::new();
    let mut returns = Vec::new();

    for row in &rows {
        let ret = (row.sell_price / row.buy_price - 1.0) * 100.0;
        returns.push(ret);
        sentiment_vals.push(row.sentiment_score.unwrap_or(50) as f64);

        if let Some(ref json) = row.score_breakdown_json {
            if let Ok(sb) = serde_json::from_str::<crate::pipeline::ScoreBreakdown>(json) {
                technical_vals.push(sb.technical as f64);
                quality_vals.push(sb.fundamental_quality as f64);
                valuation_vals.push(sb.valuation_safety as f64);
                flow_vals.push(sb.capital_flow as f64);
                growth_vals.push(sb.growth_sustainability as f64);
            } else {
                push_neutral(
                    &mut technical_vals,
                    &mut quality_vals,
                    &mut valuation_vals,
                    &mut flow_vals,
                    &mut growth_vals,
                );
            }
        } else {
            push_neutral(
                &mut technical_vals,
                &mut quality_vals,
                &mut valuation_vals,
                &mut flow_vals,
                &mut growth_vals,
            );
        }
    }

    let n = returns.len();
    if n < 30 {
        return None;
    }

    let analyses: Vec<FactorIC> = [
        ("技术面(technical)", &technical_vals[..n]),
        ("盈利质量(fundamental_quality)", &quality_vals[..n]),
        ("估值安全边际(valuation_safety)", &valuation_vals[..n]),
        ("资金面(capital_flow)", &flow_vals[..n]),
        ("增长可持续(growth_sustainability)", &growth_vals[..n]),
    ]
    .iter()
    .filter_map(|(name, vals)| analyze_factor(name, vals, &returns[..n]))
    .collect();

    let sentiment_ic = analyze_factor(
        "AI综合评分(sentiment_score)",
        &sentiment_vals[..n],
        &returns[..n],
    );

    let samples = build_factor_samples(
        &technical_vals,
        &quality_vals,
        &valuation_vals,
        &flow_vals,
        &growth_vals,
        &returns,
        n,
    );
    let corr_matrix = factor_correlation_matrix(&samples);

    Some(generate_report(
        &analyses,
        &corr_matrix,
        sentiment_ic.as_ref(),
    ))
}

fn push_neutral(
    t: &mut Vec<f64>,
    q: &mut Vec<f64>,
    v: &mut Vec<f64>,
    f: &mut Vec<f64>,
    g: &mut Vec<f64>,
) {
    t.push(50.0);
    q.push(50.0);
    v.push(50.0);
    f.push(50.0);
    g.push(50.0);
}

fn build_factor_samples(
    technical: &[f64],
    quality: &[f64],
    valuation: &[f64],
    flow: &[f64],
    growth: &[f64],
    returns: &[f64],
    n: usize,
) -> Vec<FactorSample> {
    vec![
        FactorSample {
            factor_name: "技术面".to_string(),
            factor_values: technical[..n].to_vec(),
            forward_returns: returns[..n].to_vec(),
        },
        FactorSample {
            factor_name: "盈利质量".to_string(),
            factor_values: quality[..n].to_vec(),
            forward_returns: returns[..n].to_vec(),
        },
        FactorSample {
            factor_name: "估值安全边际".to_string(),
            factor_values: valuation[..n].to_vec(),
            forward_returns: returns[..n].to_vec(),
        },
        FactorSample {
            factor_name: "资金面".to_string(),
            factor_values: flow[..n].to_vec(),
            forward_returns: returns[..n].to_vec(),
        },
        FactorSample {
            factor_name: "增长可持续".to_string(),
            factor_values: growth[..n].to_vec(),
            forward_returns: returns[..n].to_vec(),
        },
    ]
}

// ============================================================================
// 因子分析
// ============================================================================

/// 对单个因子做完整 IC 分析。
pub fn analyze_factor(
    factor_name: &str,
    factor_values: &[f64],
    forward_returns: &[f64],
) -> Option<FactorIC> {
    let n = factor_values.len().min(forward_returns.len());
    if n < 15 {
        info!("因子 '{}' 样本量 {} < 15，跳过分析", factor_name, n);
        return None;
    }

    let (ic_series, ic_decay) = compute_rolling_ic(factor_values, forward_returns)?;

    let mean_ic = ic_series.iter().sum::<f64>() / ic_series.len() as f64;
    let ic_std = (ic_series
        .iter()
        .map(|ic| (ic - mean_ic).powi(2))
        .sum::<f64>()
        / ic_series.len() as f64)
        .sqrt();
    let information_ratio = if ic_std > 1e-10 {
        mean_ic / ic_std
    } else {
        0.0
    };
    let ic_win_rate =
        ic_series.iter().filter(|&&ic| ic > 0.0).count() as f64 / ic_series.len() as f64;
    let t_stat = if ic_std > 1e-10 {
        mean_ic / (ic_std / (ic_series.len() as f64).sqrt())
    } else {
        0.0
    };

    // 显著性: |t| > 2.0 近似 95% 置信
    let significant = t_stat.abs() > 2.0;
    let verdict = if mean_ic > 0.02 {
        FactorVerdict::Positive { significant }
    } else if mean_ic < -0.02 {
        FactorVerdict::Negative { significant }
    } else {
        FactorVerdict::Neutral
    };

    Some(FactorIC {
        factor_name: factor_name.to_string(),
        mean_ic,
        information_ratio,
        ic_std,
        ic_win_rate,
        t_stat,
        ic_decay,
        sample_periods: ic_series.len(),
        verdict,
    })
}

// ============================================================================
// 因子间相关性矩阵
// ============================================================================

/// 计算因子间 Spearman 秩相关矩阵。
pub fn factor_correlation_matrix(factors: &[FactorSample]) -> Vec<Vec<f64>> {
    let m = factors.len();
    let mut matrix = vec![vec![1.0; m]; m];

    for i in 0..m {
        for j in (i + 1)..m {
            let corr = compute_spearman_ic(&factors[i].factor_values, &factors[j].factor_values)
                .unwrap_or(0.0);
            matrix[i][j] = corr;
            matrix[j][i] = corr;
        }
    }

    matrix
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spearman_ic_perfect_positive() {
        // 完全正相关: x=y
        let x: Vec<f64> = (0..50).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..50).map(|i| i as f64).collect();
        let ic = compute_spearman_ic(&x, &y).unwrap();
        assert!(ic > 0.99, "Expected ~1.0, got {}", ic);
    }

    #[test]
    fn test_spearman_ic_perfect_negative() {
        // 完全负相关: y = -x
        let x: Vec<f64> = (0..50).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..50).map(|i| -(i as f64)).collect();
        let ic = compute_spearman_ic(&x, &y).unwrap();
        assert!(ic < -0.99, "Expected ~-1.0, got {}", ic);
    }

    #[test]
    fn test_spearman_ic_random() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        // 伪随机: IC 应接近 0
        let mut x = Vec::new();
        let mut y = Vec::new();
        for i in 0..100 {
            let mut h1 = DefaultHasher::new();
            (i, 0).hash(&mut h1);
            x.push((h1.finish() % 10000) as f64 / 10000.0);
            let mut h2 = DefaultHasher::new();
            (i, 1).hash(&mut h2);
            y.push((h2.finish() % 10000) as f64 / 10000.0);
        }
        let ic = compute_spearman_ic(&x, &y).unwrap();
        assert!(ic.abs() < 0.3, "Expected near 0, got {}", ic);
    }

    #[test]
    fn test_small_sample_returns_none() {
        let x = vec![1.0; 10];
        let y = vec![2.0; 10];
        assert!(compute_spearman_ic(&x, &y).is_none());
    }

    #[test]
    fn test_nan_filtered() {
        let mut x: Vec<f64> = (0..50).map(|i| i as f64).collect();
        let mut y: Vec<f64> = (0..50).map(|i| i as f64).collect();
        x[5] = f64::NAN;
        y[10] = f64::INFINITY;
        let ic = compute_spearman_ic(&x, &y).unwrap();
        // NaN/Inf 被过滤后仍应接近 1.0
        assert!(ic > 0.95, "Expected near 1.0 after NaN filter, got {}", ic);
    }

    #[test]
    fn test_factor_verdict_labels() {
        assert!(FactorVerdict::Positive { significant: true }
            .label()
            .contains("显著正向"));
        assert!(FactorVerdict::Negative { significant: true }
            .label()
            .contains("显著反向"));
        assert!(FactorVerdict::Neutral.label().contains("无效"));
    }
}
