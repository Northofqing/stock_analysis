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
