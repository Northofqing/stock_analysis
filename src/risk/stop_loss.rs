//! 三级止损体系 — 技术止损 / 结构止损 / 硬止损。
//!
//! 复用 monitor/risk.rs 已有逻辑（StopLoss/MarketRegime），提供检查接口。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopLevel {
    Technical,
    Structural,
    Hard,
}

impl StopLevel {
    pub fn label(&self) -> &'static str {
        match self {
            StopLevel::Technical => "技术止损（破短期均线）",
            StopLevel::Structural => "结构止损（破中期趋势）",
            StopLevel::Hard => "硬止损（绝对亏损线）",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StopSignal {
    pub code: String,
    pub name: String,
    pub level: StopLevel,
    pub current_price: f64,
    pub trigger_price: f64,
    pub reason: String,
}

/// 检查持仓是否触发三级止损
pub fn check_stops(
    code: &str,
    name: &str,
    current_price: f64,
    cost_price: f64,
    hard_stop: Option<f64>,
    ma20: Option<f64>,
    ma60: Option<f64>,
) -> Vec<StopSignal> {
    let mut signals = Vec::new();

    // 硬止损：跌破用户设定的硬止损价
    if let Some(hard_stop) = hard_stop {
        if hard_stop.is_finite() && hard_stop > 0.0 && current_price <= hard_stop {
            signals.push(StopSignal {
                code: code.to_string(),
                name: name.to_string(),
                level: StopLevel::Hard,
                current_price,
                trigger_price: hard_stop,
                reason: format!("跌破硬止损价 ¥{:.2}", hard_stop),
            });
        }
    }

    // 技术止损：亏损超 -10%
    if cost_price > 0.0 {
        let loss_pct = (current_price - cost_price) / cost_price * 100.0;
        if loss_pct <= -10.0 {
            signals.push(StopSignal {
                code: code.to_string(),
                name: name.to_string(),
                level: StopLevel::Technical,
                current_price,
                trigger_price: cost_price * 0.9,
                reason: format!("亏损 {:.1}% 触发技术止损", loss_pct),
            });
        }
    }

    // 结构止损：跌破 60 日均线
    if let (Some(ma60_v), Some(ma20_v)) = (ma60, ma20) {
        if current_price < ma60_v && ma20_v < ma60_v {
            signals.push(StopSignal {
                code: code.to_string(),
                name: name.to_string(),
                level: StopLevel::Structural,
                current_price,
                trigger_price: ma60_v,
                reason: format!("跌破60日均线 ¥{:.2} 且短期均线在下", ma60_v),
            });
        }
    }

    signals
}

/// 格式化止损告警
pub fn format_stop_alerts(signals: &[StopSignal]) -> String {
    if signals.is_empty() {
        return String::new();
    }
    let mut lines = vec!["🛑 止损触发".to_string()];
    for s in signals {
        lines.push(format!(
            "  {} {}({}) ¥{:.2} — {}",
            if s.level == StopLevel::Hard {
                "🔴"
            } else {
                "⚠️"
            },
            s.name,
            s.code,
            s.current_price,
            s.reason,
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hard_stop_triggered() {
        let signals = check_stops("TEST_CODE_000001", "测试", 8.5, 10.0, Some(9.0), None, None);
        assert_eq!(signals.len(), 2); // hard + technical
        assert!(signals.iter().any(|s| s.level == StopLevel::Hard));
    }

    #[test]
    fn test_technical_stop() {
        let signals = check_stops("TEST_CODE_000001", "测试", 8.9, 10.0, Some(7.0), None, None);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].level, StopLevel::Technical);
    }

    #[test]
    fn test_no_stop() {
        let signals = check_stops(
            "TEST_CODE_000001",
            "测试",
            10.5,
            10.0,
            Some(9.0),
            Some(10.0),
            Some(9.5),
        );
        assert!(signals.is_empty());
    }

    #[test]
    fn test_structural_stop_and_labels() {
        let signals = check_stops(
            "TEST_CODE_000001",
            "测试",
            8.0,
            0.0,
            None,
            Some(8.5),
            Some(9.0),
        );
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].level, StopLevel::Structural);
        assert_eq!(signals[0].trigger_price, 9.0);
        assert_eq!(StopLevel::Technical.label(), "技术止损（破短期均线）");
        assert_eq!(StopLevel::Structural.label(), "结构止损（破中期趋势）");
        assert_eq!(StopLevel::Hard.label(), "硬止损（绝对亏损线）");
    }

    #[test]
    fn test_invalid_hard_stop_is_ignored_and_alerts_are_formatted() {
        let signals = check_stops(
            "TEST_CODE_000001",
            "测试",
            8.0,
            10.0,
            Some(f64::NAN),
            None,
            None,
        );
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].level, StopLevel::Technical);
        assert_eq!(format_stop_alerts(&[]), "");
        let alert = format_stop_alerts(&signals);
        assert!(alert.starts_with("🛑 止损触发"));
        assert!(alert.contains("⚠️ 测试(TEST_CODE_000001)"));

        let hard = check_stops(
            "TEST_CODE_000002",
            "硬止损测试",
            8.0,
            10.0,
            Some(9.0),
            None,
            None,
        );
        assert!(format_stop_alerts(&hard).contains("🔴 硬止损测试"));
    }
}
