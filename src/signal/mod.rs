//! Signal Context — 统一的买卖信号数据结构。
//!
//! 不重写 trend_analyzer/detector/strategy 的内部逻辑，
//! 只定义它们输出信号的统一格式。保留每个指标的文字描述，不压缩成单分数。
//!
//! FIXME(architecture): Signal/SignalSet are defined but not imported by any module.
//! They should serve as the canonical signal vocabulary for monitor, pipeline, and decision
//! contexts. See ARCHITECTURE_REVIEW.md Finding F1.

use chrono::NaiveDateTime;

#[derive(Debug, Clone)]
pub struct Signal {
    pub source: SignalSource,
    pub direction: SignalDirection,
    pub strength: u8,        // 0-100
    pub description: String, // "MACD金叉 + 均线多头排列"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignalSource {
    Trend,     // 趋势指标 (MA/MACD)
    Momentum,  // 动量指标 (RSI/KDJ)
    Volume,    // 成交量
    Flow,      // 资金流
    News,      // 消息面
}

impl SignalSource {
    pub fn label(&self) -> &'static str {
        match self {
            SignalSource::Trend => "趋势",
            SignalSource::Momentum => "动量",
            SignalSource::Volume => "量能",
            SignalSource::Flow => "资金",
            SignalSource::News => "消息",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalDirection { Bullish, Bearish, Neutral }

impl SignalDirection {
    pub fn label(&self) -> &'static str {
        match self {
            SignalDirection::Bullish => "看多",
            SignalDirection::Bearish => "看空",
            SignalDirection::Neutral => "中性",
        }
    }
}

/// 一只股票的多维度信号集合
#[derive(Debug, Clone)]
pub struct SignalSet {
    pub code: String,
    pub signals: Vec<Signal>,
    pub timestamp: NaiveDateTime,
}

impl SignalSet {
    /// 输出人类可读的多指标研判（不复合成单分数）
    pub fn summary(&self) -> String {
        if self.signals.is_empty() { return "无信号".to_string(); }
        self.signals.iter()
            .map(|s| format!("  {}[{}]: {}", s.source.label(), s.direction.label(), s.description))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 按方向统计
    pub fn bullish_count(&self) -> usize {
        self.signals.iter().filter(|s| s.direction == SignalDirection::Bullish).count()
    }

    pub fn bearish_count(&self) -> usize {
        self.signals.iter().filter(|s| s.direction == SignalDirection::Bearish).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_set_summary() {
        let set = SignalSet {
            code: "000001".into(),
            signals: vec![
                Signal { source: SignalSource::Trend, direction: SignalDirection::Bullish, strength: 70, description: "MA多头排列".into() },
                Signal { source: SignalSource::Momentum, direction: SignalDirection::Bullish, strength: 60, description: "RSI金叉".into() },
            ],
            timestamp: chrono::Local::now().naive_utc(),
        };
        let text = set.summary();
        assert!(text.contains("趋势[看多]: MA多头排列"));
        assert!(text.contains("动量[看多]: RSI金叉"));
        assert_eq!(set.bullish_count(), 2);
    }

    #[test]
    fn test_empty_signal_set() {
        let set = SignalSet {
            code: "000001".into(),
            signals: vec![],
            timestamp: chrono::Local::now().naive_utc(),
        };
        assert_eq!(set.summary(), "无信号");
    }
}
