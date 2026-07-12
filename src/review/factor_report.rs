//! 因子 IC 分析 — Markdown 报告生成。
//!
//! 生成面向用户的诊断报告，包含 IC 表格、衰减图、相关性矩阵和方向性诊断。

use super::factor_ic::{FactorIC, FactorVerdict};

/// 生成因子 IC 分析 Markdown 报告。
pub fn generate_report(
    analyses: &[FactorIC],
    corr_matrix: &[Vec<f64>],
    sentiment_score_ic: Option<&FactorIC>,
) -> String {
    let mut s = String::new();

    s.push_str("## 🔬 AI 评分因子 IC 诊断报告\n\n");

    // ── 执行摘要 ──
    s.push_str("### 执行摘要\n\n");

    if let Some(sent_ic) = sentiment_score_ic {
        match sent_ic.verdict {
            FactorVerdict::Negative { .. } => {
                s.push_str(&format!(
                    "> ⚠️ **AI 综合评分方向性错误**: IC = {:.4}, IR = {:.3}, t = {:.2}\n",
                    sent_ic.mean_ic, sent_ic.information_ratio, sent_ic.t_stat
                ));
                s.push_str("> AI 评分与未来收益 **系统性反向** — 高评分对应低收益。\n");
                s.push_str(
                    "> **建议**: 继续使用 B 方案（布林+MACD 共振 + 反向信号）作为买入触发。\n\n",
                );
            }
            FactorVerdict::Neutral => {
                s.push_str("> AI 综合评分方向性不显著，无法有效预测未来收益。\n\n");
            }
            FactorVerdict::Positive { .. } => {
                s.push_str("> ✅ AI 综合评分方向性正确，高评分对应高收益。\n\n");
            }
        }
    } else {
        s.push_str("> 样本量不足，无法对 AI 综合评分做方向性诊断。\n\n");
    }

    // ── IC 明细表 ──
    s.push_str("### 各因子 IC 明细\n\n");
    s.push_str("| 因子 | IC 均值 | IR | IC 标准差 | 胜率 | t-stat | 样本期 | 评估 |\n");
    s.push_str("|------|--------|----|---------|------|--------|--------|------|\n");

    for fa in analyses {
        s.push_str(&format!(
            "| {} | {:.4} | {:.3} | {:.4} | {:.0}% | {:.2} | {} | {} |\n",
            fa.factor_name,
            fa.mean_ic,
            fa.information_ratio,
            fa.ic_std,
            fa.ic_win_rate * 100.0,
            fa.t_stat,
            fa.sample_periods,
            fa.verdict.label(),
        ));
    }

    // ── IC 衰减 ──
    s.push_str("\n### IC 衰减 (Lag 1/2/3/4)\n\n");
    s.push_str("| 因子 | Lag 1 | Lag 2 | Lag 3 | Lag 4 |\n");
    s.push_str("|------|-------|-------|-------|-------|\n");
    for fa in analyses {
        s.push_str(&format!(
            "| {} | {:.4} | {:.4} | {:.4} | {:.4} |\n",
            fa.factor_name, fa.ic_decay[0], fa.ic_decay[1], fa.ic_decay[2], fa.ic_decay[3],
        ));
    }

    // ── 因子相关性矩阵 ──
    if !corr_matrix.is_empty() && analyses.len() >= 2 {
        s.push_str("\n### 因子间相关性矩阵\n\n");
        // Header
        s.push_str("|  | ");
        for fa in analyses {
            s.push_str(&format!("{} | ", truncate_name(&fa.factor_name, 8)));
        }
        s.push('\n');
        s.push_str("|--|");
        for _ in analyses {
            s.push_str("------|");
        }
        s.push('\n');

        for i in 0..analyses.len() {
            s.push_str(&format!(
                "| {} | ",
                truncate_name(&analyses[i].factor_name, 8)
            ));
            for j in 0..analyses.len() {
                if i < corr_matrix.len() && j < corr_matrix[i].len() {
                    let v = corr_matrix[i][j];
                    let icon = if v.abs() > 0.7 {
                        "🔴"
                    } else if v.abs() > 0.4 {
                        "🟡"
                    } else {
                        "🟢"
                    };
                    s.push_str(&format!("{:.3} {} | ", v, icon));
                } else {
                    s.push_str("- | ");
                }
            }
            s.push('\n');
        }
    }

    // ── 诊断建议 ──
    s.push_str("\n### 诊断建议\n\n");

    let negative_factors: Vec<&FactorIC> = analyses
        .iter()
        .filter(|fa| matches!(fa.verdict, FactorVerdict::Negative { .. }))
        .collect();
    let positive_factors: Vec<&FactorIC> = analyses
        .iter()
        .filter(|fa| matches!(fa.verdict, FactorVerdict::Positive { significant: true }))
        .collect();

    if !negative_factors.is_empty() {
        s.push_str("**方向性错误的因子** (需降权或反转):\n\n");
        for fa in &negative_factors {
            s.push_str(&format!(
                "- **{}**: IC={:.3}, 该因子值越高收益越差\n",
                fa.factor_name, fa.mean_ic
            ));
        }
        s.push('\n');
    }

    if !positive_factors.is_empty() {
        s.push_str("**有效因子** (可加权的):\n\n");
        for fa in &positive_factors {
            s.push_str(&format!(
                "- **{}**: IC={:.3}, IR={:.2}\n",
                fa.factor_name, fa.mean_ic, fa.information_ratio
            ));
        }
        s.push('\n');
    }

    if negative_factors.is_empty() && positive_factors.is_empty() {
        s.push_str("所有因子均未显示统计显著性 — 建议扩大样本量或重新设计因子。\n\n");
    }

    s.push_str(&format!(
        "> 📅 报告生成时间: {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    ));

    s
}

fn truncate_name(name: &str, max_len: usize) -> String {
    if name.len() > max_len {
        format!("{}..", &name[..max_len - 2])
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::factor_ic::{FactorIC, FactorVerdict};

    fn make_factor(name: &str, ic: f64, ir: f64, verdict: FactorVerdict) -> FactorIC {
        FactorIC {
            factor_name: name.to_string(),
            mean_ic: ic,
            information_ratio: ir,
            ic_std: 0.1,
            ic_win_rate: 0.6,
            t_stat: ic / 0.1 * 5.0,
            ic_decay: [ic, ic * 0.8, ic * 0.6, ic * 0.4],
            sample_periods: 50,
            verdict,
        }
    }

    #[test]
    fn test_report_generation_no_panic() {
        let factors = vec![
            make_factor(
                "技术面",
                0.05,
                0.5,
                FactorVerdict::Positive { significant: false },
            ),
            make_factor(
                "盈利质量",
                -0.03,
                -0.3,
                FactorVerdict::Negative { significant: false },
            ),
            make_factor("估值安全边际", 0.02, 0.2, FactorVerdict::Neutral),
        ];
        let corr = vec![
            vec![1.0, 0.3, -0.1],
            vec![0.3, 1.0, 0.2],
            vec![-0.1, 0.2, 1.0],
        ];
        let sent_ic = make_factor(
            "AI综合评分",
            -0.08,
            -0.8,
            FactorVerdict::Negative { significant: true },
        );
        let report = generate_report(&factors, &corr, Some(&sent_ic));
        assert!(report.contains("方向性错误"));
        assert!(report.contains("IC 均值"));
        assert!(report.contains("相关性矩阵"));
    }

    #[test]
    fn test_empty_report() {
        let report = generate_report(&[], &[], None);
        assert!(report.contains("AI 评分因子 IC 诊断报告"));
    }

    #[test]
    fn test_truncate_name() {
        assert_eq!(truncate_name("hello", 8), "hello");
        assert_eq!(truncate_name("hello_world_long", 8), "hello_..");
    }
}
