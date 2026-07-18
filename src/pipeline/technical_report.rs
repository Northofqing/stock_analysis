//! 纯渲染：基于 `TrendAnalysisResult` 生成技术分析 Markdown。

use crate::trend_analyzer::TrendAnalysisResult;

/// 生成 `# 技术分析` 全部内容（不含 AI / 新闻段落）。
pub(super) fn build_technical_markdown(trend_result: &TrendAnalysisResult) -> String {
    let mut s = String::from("# 技术分析\n\n");

    // 核心技术指标表格
    s.push_str("## 📊 核心技术指标\n\n");
    s.push_str("| 指标 | 数值 | 状态 |\n");
    s.push_str("|------|------|------|\n");
    s.push_str(&format!(
        "| 趋势状态 | {} | {} |\n",
        trend_result.trend_status,
        match trend_result.signal_score {
            70..=100 => "✅ 良好",
            50..=69 => "⚠️ 中性",
            _ => "🔴 偏弱",
        }
    ));
    s.push_str(&format!(
        "| 买入信号 | {} | 评分: {}/100 |\n",
        trend_result.buy_signal, trend_result.signal_score
    ));
    s.push_str(&format!("| MA排列 | {} | - |\n", trend_result.ma_alignment));
    s.push_str(&format!(
        "| 量能状态 | {} | 量比: {:.2} |\n",
        trend_result.volume_status, trend_result.volume_ratio_5d
    ));
    s.push_str(&format!(
        "| 趋势强度 | {:.1}% | {} |\n",
        trend_result.trend_strength * 100.0,
        if trend_result.trend_strength > 0.7 {
            "强势"
        } else if trend_result.trend_strength > 0.4 {
            "中等"
        } else {
            "较弱"
        }
    ));

    if let Some(sharpe) = trend_result.sharpe_ratio {
        s.push_str(&format!(
            "| 夏普比率 | {:.3} | {} |\n",
            sharpe,
            if sharpe >= 2.0 {
                "🌟 优秀"
            } else if sharpe >= 1.0 {
                "✅ 良好"
            } else if sharpe >= 0.5 {
                "⚡ 一般"
            } else if sharpe >= 0.0 {
                "⚠️ 偏低"
            } else {
                "🔴 风险大于收益"
            }
        ));
    }

    // 关键价位
    if !trend_result.support_levels.is_empty() || !trend_result.resistance_levels.is_empty() {
        s.push_str("\n## 🎯 关键价位\n\n");
        s.push_str("| 类型 | 价位(元) | 说明 |\n");
        s.push_str("|------|---------|------|\n");
        for (i, level) in trend_result.resistance_levels.iter().enumerate() {
            s.push_str(&format!(
                "| 🔴 压力位{} | {:.2} | 突破后看涨 |\n",
                i + 1,
                level
            ));
        }
        s.push_str(&format!(
            "| 📍 当前价 | {:.2} | - |\n",
            trend_result.current_price
        ));
        for (i, level) in trend_result.support_levels.iter().enumerate() {
            s.push_str(&format!(
                "| 🟢 支撑位{} | {:.2} | 跌破需警惕 |\n",
                i + 1,
                level
            ));
        }
    }

    if !trend_result.signal_reasons.is_empty() {
        s.push_str("\n## 信号原因\n");
        for reason in &trend_result.signal_reasons {
            s.push_str(&format!("- {}\n", reason));
        }
    }

    if !trend_result.risk_factors.is_empty() {
        s.push_str("\n## 风险因素\n");
        for risk in &trend_result.risk_factors {
            s.push_str(&format!("- {}\n", risk));
        }
    }

    if trend_result.signal_score >= 60 {
        append_battle_plan(&mut s, trend_result);
    }

    s
}

/// 评分 ≥ 60 时追加「作战计划」段落。
fn append_battle_plan(s: &mut String, t: &TrendAnalysisResult) {
    s.push_str("\n## 🎯 作战计划\n\n");

    let current_price = t.current_price;

    // 建议买入价
    let buy_price = if t.bias_ma5 > 0.0 && t.bias_ma5 < 3.0 {
        current_price
    } else if !t.support_levels.is_empty() {
        t.support_levels[0]
    } else {
        current_price
    };

    // 止损位
    let stop_loss = if t.support_ma10 {
        t.ma10 * 0.98
    } else if !t.support_levels.is_empty() {
        t.support_levels[0] * 0.98
    } else {
        current_price * 0.95
    };

    // 目标价
    let target_price = if !t.resistance_levels.is_empty() {
        t.resistance_levels[0]
    } else {
        current_price * 1.12
    };

    s.push_str(&format!("- **建议买入价**: {:.2}元 ", buy_price));
    if t.bias_ma5 > 0.0 && t.bias_ma5 < 3.0 {
        s.push_str("(当前价位，接近MA5支撑)\n");
    } else if !t.support_levels.is_empty() {
        s.push_str("(等待回踩支撑位)\n");
    } else {
        s.push_str("(当前价位)\n");
    }

    s.push_str(&format!(
        "- **止损价位**: {:.2}元 (跌破-{:.1}%)\n",
        stop_loss,
        (1.0 - stop_loss / current_price) * 100.0
    ));
    s.push_str(&format!(
        "- **目标价位**: {:.2}元 (预期+{:.1}%)\n",
        target_price,
        (target_price / current_price - 1.0) * 100.0
    ));

    let position_suggestion = if t.signal_score >= 80 {
        "建议仓位: 50-70% (强势信号)"
    } else if t.signal_score >= 70 {
        "建议仓位: 30-50% (中等信号)"
    } else {
        "建议仓位: 20-30% (试探性建仓)"
    };
    s.push_str(&format!("- **{}**\n", position_suggestion));

    s.push_str("\n**操作策略**:\n");
    if t.support_ma5 || t.support_ma10 {
        s.push_str("- 当前在均线支撑位附近，可分批建仓\n");
    } else {
        s.push_str("- 等待回踩均线支撑再介入，不追高\n");
    }
    if !t.support_levels.is_empty() {
        s.push_str(&format!(
            "- 重要支撑位: {:.2}元，跌破需重新评估\n",
            t.support_levels[0]
        ));
    }
    if !t.resistance_levels.is_empty() {
        s.push_str(&format!(
            "- 上方压力位: {:.2}元，突破后可加仓\n",
            t.resistance_levels[0]
        ));
    }
    s.push_str("- 严格执行止损，避免深套\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trend_analyzer::{BuySignal, TrendStatus, VolumeStatus};

    fn trend(
        signal_score: i32,
        trend_strength: f64,
        sharpe_ratio: Option<f64>,
    ) -> TrendAnalysisResult {
        TrendAnalysisResult {
            code: "TEST_CODE_000001".into(),
            trend_status: TrendStatus::Bull,
            ma_alignment: "MA5>MA10>MA20".into(),
            trend_strength,
            ma5: 9.9,
            ma10: 9.5,
            ma20: 9.0,
            ma60: 8.0,
            current_price: 10.0,
            bias_ma5: 1.0,
            bias_ma10: 5.0,
            bias_ma20: 10.0,
            volume_status: VolumeStatus::HeavyVolumeUp,
            volume_ratio_5d: 1.5,
            volume_trend: "放量".into(),
            support_ma5: false,
            support_ma10: false,
            resistance_levels: Vec::new(),
            support_levels: Vec::new(),
            buy_signal: BuySignal::Buy,
            signal_score,
            signal_reasons: Vec::new(),
            risk_factors: Vec::new(),
            sharpe_ratio,
            indicator_analysis: None,
        }
    }

    #[test]
    fn weak_report_omits_optional_sections_and_battle_plan() {
        let report = build_technical_markdown(&trend(49, 0.4, None));

        assert!(report.contains("| 趋势状态 | 多头排列 | 🔴 偏弱 |"));
        assert!(report.contains("| 趋势强度 | 40.0% | 较弱 |"));
        assert!(!report.contains("夏普比率"));
        assert!(!report.contains("关键价位"));
        assert!(!report.contains("信号原因"));
        assert!(!report.contains("风险因素"));
        assert!(!report.contains("作战计划"));
    }

    #[test]
    fn complete_strong_report_renders_levels_reasons_risks_and_ma_plan() {
        let mut result = trend(70, 0.8, Some(2.0));
        result.support_ma5 = true;
        result.support_ma10 = true;
        result.support_levels = vec![9.2];
        result.resistance_levels = vec![11.5];
        result.signal_reasons = vec!["均线多头".into()];
        result.risk_factors = vec!["波动扩大".into()];

        let report = build_technical_markdown(&result);

        assert!(report.contains("| 趋势状态 | 多头排列 | ✅ 良好 |"));
        assert!(report.contains("| 趋势强度 | 80.0% | 强势 |"));
        assert!(report.contains("| 夏普比率 | 2.000 | 🌟 优秀 |"));
        assert!(report.contains("| 🔴 压力位1 | 11.50 |"));
        assert!(report.contains("| 🟢 支撑位1 | 9.20 |"));
        assert!(report.contains("- 均线多头"));
        assert!(report.contains("- 波动扩大"));
        assert!(report.contains("**建议买入价**: 10.00元 (当前价位，接近MA5支撑)"));
        assert!(report.contains("**止损价位**: 9.31元"));
        assert!(report.contains("**目标价位**: 11.50元"));
        assert!(report.contains("建议仓位: 30-50%"));
        assert!(report.contains("当前在均线支撑位附近"));
    }

    #[test]
    fn sharpe_bands_and_mid_strength_are_rendered_at_boundaries() {
        let cases = [
            (1.0, "✅ 良好"),
            (0.5, "⚡ 一般"),
            (0.0, "⚠️ 偏低"),
            (-0.1, "🔴 风险大于收益"),
        ];

        for (sharpe, expected) in cases {
            let report = build_technical_markdown(&trend(50, 0.5, Some(sharpe)));
            assert!(report.contains("| 趋势状态 | 多头排列 | ⚠️ 中性 |"));
            assert!(report.contains("| 趋势强度 | 50.0% | 中等 |"));
            assert!(report.contains(expected), "sharpe={sharpe}: {report}");
        }
    }

    #[test]
    fn battle_plan_uses_support_or_market_fallbacks_and_score_position_bands() {
        let mut supported = trend(80, 0.8, None);
        supported.bias_ma5 = 4.0;
        supported.support_levels = vec![9.0];
        let supported_report = build_technical_markdown(&supported);
        assert!(supported_report.contains("**建议买入价**: 9.00元 (等待回踩支撑位)"));
        assert!(supported_report.contains("**止损价位**: 8.82元"));
        assert!(supported_report.contains("**目标价位**: 11.20元"));
        assert!(supported_report.contains("建议仓位: 50-70%"));
        assert!(supported_report.contains("等待回踩均线支撑再介入"));
        assert!(supported_report.contains("重要支撑位: 9.00元"));

        let mut market = trend(60, 0.5, None);
        market.bias_ma5 = 4.0;
        let market_report = build_technical_markdown(&market);
        assert!(market_report.contains("**建议买入价**: 10.00元 (当前价位)"));
        assert!(market_report.contains("**止损价位**: 9.50元"));
        assert!(market_report.contains("**目标价位**: 11.20元"));
        assert!(market_report.contains("建议仓位: 20-30%"));
    }
}
