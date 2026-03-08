// -*- coding: utf-8 -*-
//! ===================================
//! 趋势交易分析器 - 基于用户交易理念
//! ===================================
//!
//! 交易理念核心原则：
//! 1. 严进策略 - 不追高，追求每笔交易成功率
//! 2. 趋势交易 - MA5>MA10>MA20 多头排列，顺势而为
//! 3. 效率优先 - 关注筹码结构好的股票
//! 4. 买点偏好 - 在 MA5/MA10 附近回踩买入
//!
//! 技术标准：
//! - 多头排列：MA5 > MA10 > MA20
//! - 乖离率：(Close - MA5) / MA5 < 5%（不追高）
//! - 量能形态：缩量回调优先

use log::warn;
use serde::{Deserialize, Serialize};
use std::fmt;

// ============================================================================
// 枚举类型
// ============================================================================

/// 趋势状态枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrendStatus {
    /// 强势多头 - MA5 > MA10 > MA20，且间距扩大
    StrongBull,
    /// 多头排列 - MA5 > MA10 > MA20
    Bull,
    /// 弱势多头 - MA5 > MA10，但 MA10 < MA20
    WeakBull,
    /// 盘整 - 均线缠绕
    Consolidation,
    /// 弱势空头 - MA5 < MA10，但 MA10 > MA20
    WeakBear,
    /// 空头排列 - MA5 < MA10 < MA20
    Bear,
    /// 强势空头 - MA5 < MA10 < MA20，且间距扩大
    StrongBear,
}

impl fmt::Display for TrendStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            Self::StrongBull => "强势多头",
            Self::Bull => "多头排列",
            Self::WeakBull => "弱势多头",
            Self::Consolidation => "盘整",
            Self::WeakBear => "弱势空头",
            Self::Bear => "空头排列",
            Self::StrongBear => "强势空头",
        };
        write!(f, "{}", s)
    }
}

/// 量能状态枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VolumeStatus {
    /// 放量上涨 - 量价齐升
    HeavyVolumeUp,
    /// 放量下跌 - 放量杀跌
    HeavyVolumeDown,
    /// 缩量上涨 - 无量上涨
    ShrinkVolumeUp,
    /// 缩量回调 - 缩量回调（好）
    ShrinkVolumeDown,
    /// 量能正常
    Normal,
}

impl fmt::Display for VolumeStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            Self::HeavyVolumeUp => "放量上涨",
            Self::HeavyVolumeDown => "放量下跌",
            Self::ShrinkVolumeUp => "缩量上涨",
            Self::ShrinkVolumeDown => "缩量回调",
            Self::Normal => "量能正常",
        };
        write!(f, "{}", s)
    }
}

/// 买入信号枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuySignal {
    /// 强烈买入 - 多条件满足
    StrongBuy,
    /// 买入 - 基本条件满足
    Buy,
    /// 持有 - 已持有可继续
    Hold,
    /// 观望 - 等待更好时机
    Wait,
    /// 卖出 - 趋势转弱
    Sell,
    /// 强烈卖出 - 趋势破坏
    StrongSell,
}

impl fmt::Display for BuySignal {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            Self::StrongBuy => "强烈买入",
            Self::Buy => "买入",
            Self::Hold => "持有",
            Self::Wait => "观望",
            Self::Sell => "卖出",
            Self::StrongSell => "强烈卖出",
        };
        write!(f, "{}", s)
    }
}

// ============================================================================
// 数据结构
// ============================================================================

/// 趋势分析结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendAnalysisResult {
    pub code: String,

    // 趋势判断
    pub trend_status: TrendStatus,
    pub ma_alignment: String,
    pub trend_strength: f64,

    // 均线数据
    pub ma5: f64,
    pub ma10: f64,
    pub ma20: f64,
    pub ma60: f64,
    pub current_price: f64,

    // 乖离率（与 MA5 的偏离度）
    pub bias_ma5: f64,
    pub bias_ma10: f64,
    pub bias_ma20: f64,

    // 量能分析
    pub volume_status: VolumeStatus,
    pub volume_ratio_5d: f64,
    pub volume_trend: String,

    // 支撑压力
    pub support_ma5: bool,
    pub support_ma10: bool,
    pub resistance_levels: Vec<f64>,
    pub support_levels: Vec<f64>,

    // 买入信号
    pub buy_signal: BuySignal,
    pub signal_score: i32,
    pub signal_reasons: Vec<String>,
    pub risk_factors: Vec<String>,
    
    // 风险调整后收益指标
    pub sharpe_ratio: Option<f64>,
}

impl Default for TrendAnalysisResult {
    fn default() -> Self {
        Self {
            code: String::new(),
            trend_status: TrendStatus::Consolidation,
            ma_alignment: String::new(),
            trend_strength: 0.0,
            ma5: 0.0,
            ma10: 0.0,
            ma20: 0.0,
            ma60: 0.0,
            current_price: 0.0,
            bias_ma5: 0.0,
            bias_ma10: 0.0,
            bias_ma20: 0.0,
            volume_status: VolumeStatus::Normal,
            volume_ratio_5d: 0.0,
            volume_trend: String::new(),
            support_ma5: false,
            support_ma10: false,
            resistance_levels: Vec::new(),
            support_levels: Vec::new(),
            buy_signal: BuySignal::Wait,
            signal_score: 0,
            signal_reasons: Vec::new(),
            risk_factors: Vec::new(),
            sharpe_ratio: None,
        }
    }
}

impl TrendAnalysisResult {
    pub fn new(code: String) -> Self {
        Self {
            code,
            ..Default::default()
        }
    }
}

// ============================================================================
// 股票数据结构
// ============================================================================

/// 股票数据行
#[derive(Debug, Clone)]
pub struct StockData {
    pub date: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub ma5: Option<f64>,
    pub ma10: Option<f64>,
    pub ma20: Option<f64>,
    pub ma60: Option<f64>,
}

// ============================================================================
// 趋势分析器
// ============================================================================

/// 股票趋势分析器
///
/// 基于用户交易理念实现：
/// 1. 趋势判断 - MA5>MA10>MA20 多头排列
/// 2. 乖离率检测 - 不追高，偏离 MA5 超过 5% 不买
/// 3. 量能分析 - 偏好缩量回调
/// 4. 买点识别 - 回踩 MA5/MA10 支撑
pub struct StockTrendAnalyzer {
    // 交易参数配置
    bias_threshold: f64,        // 乖离率阈值（%），超过此值不买入
    volume_shrink_ratio: f64,   // 缩量判断阈值（当日量/5日均量）
    volume_heavy_ratio: f64,    // 放量判断阈值
    ma_support_tolerance: f64,  // MA 支撑判断容忍度（2%）
}

impl Default for StockTrendAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl StockTrendAnalyzer {
    /// 创建新的分析器
    pub fn new() -> Self {
        Self {
            bias_threshold: 5.0,
            volume_shrink_ratio: 0.7,
            volume_heavy_ratio: 1.5,
            ma_support_tolerance: 0.02,
        }
    }

    /// 分析股票趋势（接受KlineData）
    pub fn analyze_with_kline(
        &self,
        kline_data: &[crate::data_provider::KlineData],
        code: &str,
    ) -> TrendAnalysisResult {
        // 转换为StockData
        // 注意：K线数据从 API 返回时是降序（最新在前），但 trend_analyzer 需要升序（最旧在前）
        // 使用 rev() 迭代器直接生成升序数据，避免先 collect 再 reverse
        let stock_data: Vec<StockData> = kline_data
            .iter()
            .rev()
            .map(|k| StockData {
                date: k.date.format("%Y-%m-%d").to_string(),
                open: k.open,
                high: k.high,
                low: k.low,
                close: k.close,
                volume: k.volume,
                ma5: None,
                ma10: None,
                ma20: None,
                ma60: None,
            })
            .collect();

        self.analyze(&stock_data, code)
    }

    /// 分析股票趋势
    pub fn analyze(&self, data: &[StockData], code: &str) -> TrendAnalysisResult {
        let mut result = TrendAnalysisResult::new(code.to_string());

        if data.is_empty() || data.len() < 20 {
            warn!("{} 数据不足，无法进行趋势分析", code);
            result.risk_factors.push("数据不足，无法完成分析".to_string());
            return result;
        }

        // 计算均线
        let data_with_ma = self.calculate_mas(data);

        // 获取最新数据
        let latest = &data_with_ma[data_with_ma.len() - 1];
        result.current_price = latest.close;
        result.ma5 = latest.ma5.unwrap_or(0.0);
        result.ma10 = latest.ma10.unwrap_or(0.0);
        result.ma20 = latest.ma20.unwrap_or(0.0);
        result.ma60 = latest.ma60.unwrap_or(0.0);

        // 1. 趋势判断
        self.analyze_trend(&data_with_ma, &mut result);

        // 2. 乖离率计算
        self.calculate_bias(&mut result);

        // 3. 量能分析
        self.analyze_volume(&data_with_ma, &mut result);

        // 4. 支撑压力分析
        self.analyze_support_resistance(&data_with_ma, &mut result);

        // 5. 生成买入信号
        self.generate_signal(&mut result);

        result
    }

    /// 计算均线
    fn calculate_mas(&self, data: &[StockData]) -> Vec<StockData> {
        let mut result = Vec::with_capacity(data.len());

        for (i, item) in data.iter().enumerate() {
            let mut new_item = item.clone();

            // MA5
            if i >= 4 {
                let sum: f64 = data[i - 4..=i].iter().map(|d| d.close).sum();
                new_item.ma5 = Some(sum / 5.0);
            }

            // MA10
            if i >= 9 {
                let sum: f64 = data[i - 9..=i].iter().map(|d| d.close).sum();
                new_item.ma10 = Some(sum / 10.0);
            }

            // MA20
            if i >= 19 {
                let sum: f64 = data[i - 19..=i].iter().map(|d| d.close).sum();
                new_item.ma20 = Some(sum / 20.0);
            }

            // MA60
            if i >= 59 {
                let sum: f64 = data[i - 59..=i].iter().map(|d| d.close).sum();
                new_item.ma60 = Some(sum / 60.0);
            } else {
                // 数据不足时使用 MA20 替代
                new_item.ma60 = new_item.ma20;
            }

            result.push(new_item);
        }

        result
    }

    /// 分析趋势状态
    fn analyze_trend(&self, data: &[StockData], result: &mut TrendAnalysisResult) {
        let ma5 = result.ma5;
        let ma10 = result.ma10;
        let ma20 = result.ma20;

        if ma5 > ma10 && ma10 > ma20 {
            // 多头排列 - 检查间距是否在扩大（强势）
            let prev_idx = if data.len() >= 5 {
                data.len() - 5
            } else {
                data.len() - 1
            };
            let prev = &data[prev_idx];

            let prev_spread = if let (Some(prev_ma5), Some(prev_ma20)) = (prev.ma5, prev.ma20) {
                if prev_ma20 > 0.0 {
                    (prev_ma5 - prev_ma20) / prev_ma20 * 100.0
                } else {
                    0.0
                }
            } else {
                0.0
            };

            let curr_spread = if ma20 > 0.0 {
                (ma5 - ma20) / ma20 * 100.0
            } else {
                0.0
            };

            if curr_spread > prev_spread && curr_spread > 5.0 {
                result.trend_status = TrendStatus::StrongBull;
                result.ma_alignment = "强势多头排列，均线发散上行".to_string();
                result.trend_strength = 90.0;
            } else {
                result.trend_status = TrendStatus::Bull;
                result.ma_alignment = "多头排列 MA5>MA10>MA20".to_string();
                result.trend_strength = 75.0;
            }
        } else if ma5 > ma10 && ma10 <= ma20 {
            result.trend_status = TrendStatus::WeakBull;
            result.ma_alignment = "弱势多头，MA5>MA10 但 MA10≤MA20".to_string();
            result.trend_strength = 55.0;
        } else if ma5 < ma10 && ma10 < ma20 {
            let prev_idx = if data.len() >= 5 {
                data.len() - 5
            } else {
                data.len() - 1
            };
            let prev = &data[prev_idx];

            let prev_spread = if let (Some(prev_ma5), Some(prev_ma20)) = (prev.ma5, prev.ma20) {
                if prev_ma5 > 0.0 {
                    (prev_ma20 - prev_ma5) / prev_ma5 * 100.0
                } else {
                    0.0
                }
            } else {
                0.0
            };

            let curr_spread = if ma5 > 0.0 {
                (ma20 - ma5) / ma5 * 100.0
            } else {
                0.0
            };

            if curr_spread > prev_spread && curr_spread > 5.0 {
                result.trend_status = TrendStatus::StrongBear;
                result.ma_alignment = "强势空头排列，均线发散下行".to_string();
                result.trend_strength = 10.0;
            } else {
                result.trend_status = TrendStatus::Bear;
                result.ma_alignment = "空头排列 MA5<MA10<MA20".to_string();
                result.trend_strength = 25.0;
            }
        } else if ma5 < ma10 && ma10 >= ma20 {
            result.trend_status = TrendStatus::WeakBear;
            result.ma_alignment = "弱势空头，MA5<MA10 但 MA10≥MA20".to_string();
            result.trend_strength = 40.0;
        } else {
            result.trend_status = TrendStatus::Consolidation;
            result.ma_alignment = "均线缠绕，趋势不明".to_string();
            result.trend_strength = 50.0;
        }
    }

    /// 计算乖离率
    fn calculate_bias(&self, result: &mut TrendAnalysisResult) {
        let price = result.current_price;

        if result.ma5 > 0.0 {
            result.bias_ma5 = (price - result.ma5) / result.ma5 * 100.0;
        }
        if result.ma10 > 0.0 {
            result.bias_ma10 = (price - result.ma10) / result.ma10 * 100.0;
        }
        if result.ma20 > 0.0 {
            result.bias_ma20 = (price - result.ma20) / result.ma20 * 100.0;
        }
    }

    /// 分析量能
    fn analyze_volume(&self, data: &[StockData], result: &mut TrendAnalysisResult) {
        if data.len() < 5 {
            return;
        }

        let latest = &data[data.len() - 1];
        
        // 计算5日均量
        let vol_5d_avg: f64 = data[data.len() - 6..data.len() - 1]
            .iter()
            .map(|d| d.volume)
            .sum::<f64>()
            / 5.0;

        if vol_5d_avg > 0.0 {
            result.volume_ratio_5d = latest.volume / vol_5d_avg;
        }

        // 判断价格变化
        let prev_close = data[data.len() - 2].close;
        let price_change = (latest.close - prev_close) / prev_close * 100.0;

        // 量能状态判断
        if result.volume_ratio_5d >= self.volume_heavy_ratio {
            if price_change > 0.0 {
                result.volume_status = VolumeStatus::HeavyVolumeUp;
                result.volume_trend = "放量上涨，多头力量强劲".to_string();
            } else {
                result.volume_status = VolumeStatus::HeavyVolumeDown;
                result.volume_trend = "放量下跌，注意风险".to_string();
            }
        } else if result.volume_ratio_5d <= self.volume_shrink_ratio {
            if price_change > 0.0 {
                result.volume_status = VolumeStatus::ShrinkVolumeUp;
                result.volume_trend = "缩量上涨，上攻动能不足".to_string();
            } else {
                result.volume_status = VolumeStatus::ShrinkVolumeDown;
                result.volume_trend = "缩量回调，洗盘特征明显（好）".to_string();
            }
        } else {
            result.volume_status = VolumeStatus::Normal;
            result.volume_trend = "量能正常".to_string();
        }
    }

    /// 分析支撑压力位
    fn analyze_support_resistance(&self, data: &[StockData], result: &mut TrendAnalysisResult) {
        let price = result.current_price;

        // 检查是否在 MA5 附近获得支撑
        if result.ma5 > 0.0 {
            let ma5_distance = (price - result.ma5).abs() / result.ma5;
            if ma5_distance <= self.ma_support_tolerance && price >= result.ma5 {
                result.support_ma5 = true;
                result.support_levels.push(result.ma5);
            }
        }

        // 检查是否在 MA10 附近获得支撑
        if result.ma10 > 0.0 {
            let ma10_distance = (price - result.ma10).abs() / result.ma10;
            if ma10_distance <= self.ma_support_tolerance && price >= result.ma10 {
                result.support_ma10 = true;
                if !result.support_levels.contains(&result.ma10) {
                    result.support_levels.push(result.ma10);
                }
            }
        }

        // MA20 作为重要支撑
        if result.ma20 > 0.0 && price >= result.ma20 {
            result.support_levels.push(result.ma20);
        }

        // 近期高点作为压力
        if data.len() >= 20 {
            let recent_high = data[data.len() - 20..]
                .iter()
                .map(|d| d.high)
                .fold(f64::NEG_INFINITY, f64::max);
            if recent_high > price {
                result.resistance_levels.push(recent_high);
            }
        }
    }

    /// 生成买入信号
    fn generate_signal(&self, result: &mut TrendAnalysisResult) {
        let mut score = 0;
        let mut reasons = Vec::new();
        let mut risks = Vec::new();

        // === 趋势评分（35分）===
        let trend_score = match result.trend_status {
            TrendStatus::StrongBull => 35,
            TrendStatus::Bull => 30,
            TrendStatus::WeakBull => 22,
            TrendStatus::Consolidation => 13,
            TrendStatus::WeakBear => 8,
            TrendStatus::Bear => 4,
            TrendStatus::StrongBear => 0,
        };
        score += trend_score;

        match result.trend_status {
            TrendStatus::StrongBull | TrendStatus::Bull => {
                reasons.push(format!("✅ {}，顺势做多", result.trend_status));
            }
            TrendStatus::Bear | TrendStatus::StrongBear => {
                risks.push(format!("⚠️ {}，不宜做多", result.trend_status));
            }
            _ => {}
        }

        // === 乖离率评分（30分）===
        let bias = result.bias_ma5;
        if bias < 0.0 {
            // 价格在 MA5 下方（回调中）
            if bias > -3.0 {
                score += 30;
                reasons.push(format!("✅ 价格略低于MA5({:.1}%)，回踩买点", bias));
            } else if bias > -5.0 {
                score += 25;
                reasons.push(format!("✅ 价格回踩MA5({:.1}%)，观察支撑", bias));
            } else {
                score += 10;
                risks.push(format!("⚠️ 乖离率过大({:.1}%)，可能破位", bias));
            }
        } else if bias < 2.0 {
            score += 28;
            reasons.push(format!("✅ 价格贴近MA5({:.1}%)，介入好时机", bias));
        } else if bias < self.bias_threshold {
            score += 20;
            reasons.push(format!("⚡ 价格略高于MA5({:.1}%)，可小仓介入", bias));
        } else {
            score += 5;
            risks.push(format!("❌ 乖离率过高({:.1}%>5%)，严禁追高！", bias));
        }

        // === 量能评分（20分）===
        let vol_score = match result.volume_status {
            VolumeStatus::ShrinkVolumeDown => 20,
            VolumeStatus::HeavyVolumeUp => 15,
            VolumeStatus::Normal => 12,
            VolumeStatus::ShrinkVolumeUp => 8,
            VolumeStatus::HeavyVolumeDown => 0,
        };
        score += vol_score;

        match result.volume_status {
            VolumeStatus::ShrinkVolumeDown => {
                reasons.push("✅ 缩量回调，主力洗盘".to_string());
            }
            VolumeStatus::HeavyVolumeDown => {
                risks.push("⚠️ 放量下跌，注意风险".to_string());
            }
            _ => {}
        }

        // === 夏普比率评分（5分）=== 
        // 夏普比率衡量风险调整后收益，是重要的质量指标
        if let Some(sharpe) = result.sharpe_ratio {
            let sharpe_score = if sharpe >= 2.0 {
                5
            } else if sharpe >= 1.0 {
                3
            } else if sharpe >= 0.5 {
                1
            } else if sharpe >= 0.0 {
                0
            } else {
                // 负夏普比率，扣分
                -2
            };
            
            score += sharpe_score;
            
            if sharpe >= 2.0 {
                reasons.push(format!("✅ 夏普比率优秀({:.2}，风险调整后收益高)", sharpe));
            } else if sharpe >= 1.0 {
                reasons.push(format!("👍 夏普比率良好({:.2})", sharpe));
            } else if sharpe < 0.0 {
                risks.push(format!("⚠️ 夏普比率为负({:.2}，风险大于收益)", sharpe));
            }
        }
        
        // === 支撑评分（10分）===
        if result.support_ma5 {
            score += 5;
            reasons.push("✅ MA5支撑有效".to_string());
        }
        if result.support_ma10 {
            score += 5;
            reasons.push("✅ MA10支撑有效".to_string());
        }

        // === 综合判断 ===
        result.signal_score = score;
        result.signal_reasons = reasons;
        result.risk_factors = risks;

        // 生成买入信号
        result.buy_signal = if score >= 80
            && matches!(
                result.trend_status,
                TrendStatus::StrongBull | TrendStatus::Bull
            )
        {
            BuySignal::StrongBuy
        } else if score >= 65
            && matches!(
                result.trend_status,
                TrendStatus::StrongBull | TrendStatus::Bull | TrendStatus::WeakBull
            )
        {
            BuySignal::Buy
        } else if score >= 50 {
            BuySignal::Hold
        } else if score >= 35 {
            BuySignal::Wait
        } else if matches!(result.trend_status, TrendStatus::Bear | TrendStatus::StrongBear) {
            BuySignal::StrongSell
        } else {
            BuySignal::Sell
        };
    }

    /// 格式化分析结果为文本
    pub fn format_analysis(&self, result: &TrendAnalysisResult) -> String {
        let mut lines = vec![
            format!("=== {} 趋势分析 ===", result.code),
            String::new(),
            format!("📊 趋势判断: {}", result.trend_status),
            format!("   均线排列: {}", result.ma_alignment),
            format!("   趋势强度: {:.0}/100", result.trend_strength),
            String::new(),
            "📈 均线数据:".to_string(),
            format!("   现价: {:.2}", result.current_price),
            format!("   MA5:  {:.2} (乖离 {:+.2}%)", result.ma5, result.bias_ma5),
            format!("   MA10: {:.2} (乖离 {:+.2}%)", result.ma10, result.bias_ma10),
            format!("   MA20: {:.2} (乖离 {:+.2}%)", result.ma20, result.bias_ma20),
            String::new(),
            format!("📊 量能分析: {}", result.volume_status),
            format!("   量比(vs5日): {:.2}", result.volume_ratio_5d),
            format!("   量能趋势: {}", result.volume_trend),
            String::new(),
            format!("🎯 操作建议: {}", result.buy_signal),
            format!("   综合评分: {}/100", result.signal_score),
        ];

        if !result.signal_reasons.is_empty() {
            lines.push(String::new());
            lines.push("✅ 买入理由:".to_string());
            for reason in &result.signal_reasons {
                lines.push(format!("   {}", reason));
            }
        }

        if !result.risk_factors.is_empty() {
            lines.push(String::new());
            lines.push("⚠️ 风险因素:".to_string());
            for risk in &result.risk_factors {
                lines.push(format!("   {}", risk));
            }
        }

        lines.join("\n")
    }
}

/// 便捷函数：分析单只股票
pub fn analyze_stock(data: &[StockData], code: &str) -> TrendAnalysisResult {
    let analyzer = StockTrendAnalyzer::new();
    analyzer.analyze(data, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_data(days: usize) -> Vec<StockData> {
        let mut data = Vec::new();
        let base_price = 10.0;
        let mut price = base_price;

        for i in 0..days {
            // 模拟轻微上涨趋势
            price *= 1.003;

            data.push(StockData {
                date: format!("2025-01-{:02}", i + 1),
                open: price,
                high: price * 1.02,
                low: price * 0.98,
                close: price,
                volume: 1000000.0,
                ma5: None,
                ma10: None,
                ma20: None,
                ma60: None,
            });
        }

        data
    }

    #[test]
    fn test_trend_analyzer() {
        let data = create_test_data(60);
        let analyzer = StockTrendAnalyzer::new();
        let result = analyzer.analyze(&data, "000001");

        assert_eq!(result.code, "000001");
        assert!(result.current_price > 0.0);
        assert!(result.ma5 > 0.0);
        assert!(result.signal_score >= 0 && result.signal_score <= 100);

        println!("{}", analyzer.format_analysis(&result));
    }

    #[test]
    fn test_bull_trend() {
        let mut data = create_test_data(60);
        
        // 强制创建多头排列
        for item in data.iter_mut().rev().take(20) {
            item.ma5 = Some(item.close * 1.02);
            item.ma10 = Some(item.close * 1.00);
            item.ma20 = Some(item.close * 0.98);
        }

        let analyzer = StockTrendAnalyzer::new();
        let result = analyzer.analyze(&data, "TEST");

        // 应该检测到多头趋势
        assert!(matches!(
            result.trend_status,
            TrendStatus::Bull | TrendStatus::StrongBull
        ));
    }
}
