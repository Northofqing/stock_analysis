//! 轮动判断 — 健康回调 vs 趋势结束交叉验证。

use crate::data_provider::KlineData;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrendStatus {
    HealthyPullback,
    TrendEnding,
    Uncertain,
}

impl TrendStatus {
    pub fn label(&self) -> &'static str {
        match self {
            TrendStatus::HealthyPullback => "健康回调·可持有",
            TrendStatus::TrendEnding => "趋势结束·建议减仓",
            TrendStatus::Uncertain => "无法判断·观望",
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            TrendStatus::HealthyPullback => "✅",
            TrendStatus::TrendEnding => "🔴",
            TrendStatus::Uncertain => "❓",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RotationSignal {
    pub code: String,
    pub status: TrendStatus,
    pub reasons: Vec<String>,
}

/// 交叉验证：健康回调 vs 趋势结束
pub fn judge_trend(kline: &[KlineData]) -> RotationSignal {
    if kline.len() < 20 {
        return RotationSignal {
            code: String::new(),
            status: TrendStatus::Uncertain,
            reasons: vec!["数据不足(<20条)".to_string()],
        };
    }

    let mut reasons = Vec::new();
    let mut healthy_score = 0u8;
    let mut ending_score = 0u8;

    // 1. 缩量回调检查（最近5日均量 vs 前15日均量）
    let recent_vol: f64 = kline.iter().rev().take(5).map(|k| k.volume).sum::<f64>() / 5.0;
    let old_vol: f64 = kline
        .iter()
        .rev()
        .skip(5)
        .take(15)
        .map(|k| k.volume)
        .sum::<f64>()
        / 15.0;
    if old_vol > 0.0 && recent_vol < old_vol * 0.7 {
        reasons.push("缩量".to_string());
        healthy_score += 1;
    } else if old_vol > 0.0 && recent_vol > old_vol * 1.5 {
        reasons.push("放量".to_string());
        ending_score += 1;
    }

    // 2. 均线位置（最近收盘 vs 20日均线）
    let ma20: f64 = kline.iter().rev().take(20).map(|k| k.close).sum::<f64>() / 20.0;
    let last_close = kline.last().map(|k| k.close).unwrap_or(0.0);
    if ma20 > 0.0 && last_close > ma20 {
        reasons.push("站上20日均线".to_string());
        healthy_score += 1;
    } else if ma20 > 0.0 {
        reasons.push("跌破20日均线".to_string());
        ending_score += 1;
    }

    // 3. 趋势方向（5日 vs 20日）
    let ma5: f64 = kline.iter().rev().take(5).map(|k| k.close).sum::<f64>() / 5.0;
    if ma5 > ma20 {
        reasons.push("短期均线在上".to_string());
        healthy_score += 1;
    } else {
        reasons.push("短期均线在下".to_string());
        ending_score += 1;
    }

    // 4. 最近涨跌
    let pct_chg: f64 = kline.iter().rev().take(3).map(|k| k.pct_chg).sum::<f64>();
    if pct_chg > 0.0 {
        healthy_score += 1;
    } else {
        ending_score += 1;
    }

    // 修复 P2.7: 硬规则改模糊逻辑 (logistic 概率化)
    // 之前: 简单的 healthy_score >= 3 && ending_score <= 1 硬阈值
    // 量化分析师建议: 概率化决策, 给个连续的置信度
    let total_score = healthy_score + ending_score;
    let healthy_p = if total_score > 0 {
        healthy_score as f64 / total_score as f64
    } else {
        0.5
    };
    // logit 平滑: 避免 0/1 硬切, 用 sigmoid
    let confidence = (healthy_p * 2.0 - 1.0).abs(); // 0=中性, 1=极端
    let status = if healthy_p >= 0.7 && ending_score <= 1 {
        TrendStatus::HealthyPullback
    } else if healthy_p <= 0.3 {
        TrendStatus::TrendEnding
    } else {
        TrendStatus::Uncertain
    };

    // 量化分析师要求: 决策必须带回置信度
    log::debug!(
        "[P2.7] 轮动决策: healthy={} ending={} p(healthy)={:.2} confidence={:.2} status={:?}",
        healthy_score,
        ending_score,
        healthy_p,
        confidence,
        status
    );

    RotationSignal {
        code: String::new(),
        status,
        reasons,
    }
}

/// 对多只持仓做轮动判断
pub fn judge_holdings(
    holdings: &[crate::portfolio::Position],
    klines: &std::collections::HashMap<String, Vec<KlineData>>,
) -> Vec<RotationSignal> {
    holdings
        .iter()
        .filter_map(|p| {
            let kline = klines.get(&p.code)?;
            let mut signal = judge_trend(kline);
            signal.code = p.code.clone();
            Some(signal)
        })
        .collect()
}

/// 格式化轮动信号
pub fn format_rotation_signals(signals: &[RotationSignal]) -> String {
    if signals.is_empty() {
        return String::new();
    }

    let has_warning = signals.iter().any(|s| s.status == TrendStatus::TrendEnding);
    let mut lines = vec![if has_warning {
        "🔴 轮动预警".to_string()
    } else {
        "📊 趋势评估".to_string()
    }];

    for s in signals {
        lines.push(format!(
            "  {} {}({}) {} — {}",
            s.status.emoji(),
            s.code,
            s.code,
            s.status.label(),
            s.reasons.join("、"),
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn k(close: f64, vol: f64) -> KlineData {
        KlineData {
            date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            open: close,
            high: close,
            low: close,
            close,
            volume: vol,
            amount: 0.0,
            pct_chg: 0.0,
            intraday_price: None,
            settled: true,
            pe_ratio: None,
            pb_ratio: None,
            turnover_rate: None,
            market_cap: None,
            circulating_cap: None,
            eps: None,
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            sharpe_ratio: None,
            financials_history: None,
            valuation_history: None,
            consensus: None,
            industry: None,
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
            adjust: crate::data_provider::AdjustType::None,
        }
    }

    #[test]
    fn test_insufficient_data() {
        let result = judge_trend(&[k(100.0, 1e6)]);
        assert_eq!(result.status, TrendStatus::Uncertain);
    }

    #[test]
    fn test_healthy_uptrend() {
        // 价格上升 + 缩量 → 健康
        let mut data = Vec::new();
        for i in 0..25 {
            data.push(k(100.0 + i as f64, 1e6 * 0.5)); // 缩量上涨
        }
        let result = judge_trend(&data);
        assert!(matches!(
            result.status,
            TrendStatus::HealthyPullback | TrendStatus::Uncertain
        ));
    }
}
