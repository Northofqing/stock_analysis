//! 统一放量启动信号结构。

#[derive(Debug, Clone)]
pub struct BreakoutSignal {
    pub code: String,
    pub name: String,
    pub volume_ratio: Option<f64>,
    pub vol_vs_20d_avg: Option<f64>,
    pub volume_pattern: VolumePattern,
    pub change_pct: f64,
    pub candle_strength: CandleStrength,
    pub ma_break: Option<MaBreakInfo>,
    pub price_position: PricePosition,
    pub breakout_type: BreakoutType,
    pub confidence: u8,
    pub description: String,
    pub data_degraded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumePattern {
    GentleIncrease,
    SuddenSpike,
    PostShrinkBurst,
    Flat,
}

impl VolumePattern {
    pub fn label(&self) -> &'static str {
        match self {
            VolumePattern::GentleIncrease => "温和放量",
            VolumePattern::SuddenSpike => "天量",
            VolumePattern::PostShrinkBurst => "地量后放量",
            VolumePattern::Flat => "无量",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandleStrength {
    Strong,
    Medium,
    Weak,
    Bearish,
}

impl CandleStrength {
    pub fn label(&self) -> &'static str {
        match self {
            CandleStrength::Strong => "强阳线",
            CandleStrength::Medium => "中阳",
            CandleStrength::Weak => "小阳",
            CandleStrength::Bearish => "阴线",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MaBreakInfo {
    pub ma5_above_ma10: bool,
    pub ma5_above_ma20: bool,
    pub ma10_above_ma20: bool,
    pub is_bullish_alignment: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PricePosition { Low, Mid, High, Unknown }

impl PricePosition {
    pub fn label(&self) -> &'static str {
        match self {
            PricePosition::Low => "低位",
            PricePosition::Mid => "中位",
            PricePosition::High => "高位",
            PricePosition::Unknown => "未知",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakoutType { Launch, Uncertain, Distribution }

impl BreakoutType {
    pub fn label(&self) -> &'static str {
        match self {
            BreakoutType::Launch => "启动",
            BreakoutType::Uncertain => "不确定",
            BreakoutType::Distribution => "出货",
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            BreakoutType::Launch => "🚀",
            BreakoutType::Uncertain => "❓",
            BreakoutType::Distribution => "⚠️",
        }
    }
}
