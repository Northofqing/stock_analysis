// -*- coding: utf-8 -*-
//! 公共 trait 接口定义
//!
//! 提供跨模块共享的抽象接口，消除重复代码：
//! - `ScoreDisplay`：为持有 `sentiment_score` 的结构体提供统一的 emoji 与评级标签。
//! - `AiContentGenerator`：AI 内容生成器的通用接口，供大盘复盘与个股分析共用。

use anyhow::Result;

// ============================================================================
// ScoreDisplay — 情绪分数展示 trait
// ============================================================================

/// 情绪分数展示接口。
///
/// 所有包含 `sentiment_score` 字段的分析结果结构体均应实现此 trait，
/// 以复用 emoji 映射与评级标签逻辑，避免各模块各自维护一份。
///
/// # 示例
/// ```rust
/// use stock_analysis::traits::ScoreDisplay;
///
/// struct MyResult { score: i32 }
/// impl ScoreDisplay for MyResult {
///     fn sentiment_score(&self) -> i32 { self.score }
///     fn operation_advice(&self) -> &str { "买入" }
/// }
/// let r = MyResult { score: 85 };
/// assert_eq!(r.score_emoji(), "💚");
/// ```
pub trait ScoreDisplay {
    /// 获取综合情绪评分（0–100）。
    fn sentiment_score(&self) -> i32;

    /// 获取操作建议文本（如 "买入"、"持有"）。
    fn operation_advice(&self) -> &str;

    /// 根据 `sentiment_score` 返回对应 emoji。
    ///
    /// | 分段       | emoji |
    /// |-----------|-------|
    /// | 80 – 100  | 💚    |
    /// | 65 – 79   | 🟢    |
    /// | 55 – 64   | 🟡    |
    /// | 45 – 54   | ⚪    |
    /// | 35 – 44   | 🟠    |
    /// | 0  – 34   | 🔴    |
    fn score_emoji(&self) -> &'static str {
        match self.sentiment_score() {
            80.. => "💚",
            65..=79 => "🟢",
            55..=64 => "🟡",
            45..=54 => "⚪",
            35..=44 => "🟠",
            _ => "🔴",
        }
    }

    /// 根据 `sentiment_score` 返回中文评级标签。
    fn score_label(&self) -> &'static str {
        match self.sentiment_score() {
            80.. => "强烈看多",
            65..=79 => "看多",
            55..=64 => "中性偏多",
            45..=54 => "中性",
            35..=44 => "中性偏空",
            _ => "看空",
        }
    }
}

// ============================================================================
// AiContentGenerator — AI 内容生成器 trait
// ============================================================================

/// AI 内容生成器通用接口。
///
/// 抽象 Gemini / 其他大模型的调用方式，使业务层（大盘复盘、个股分析等）
/// 无需关心底层模型实现。
///
/// 该 trait 替代了原先分散在 `market_analyzer` 中的 `AiAnalyzer` trait，
/// 统一在此处定义。
pub trait AiContentGenerator: Send + Sync {
    /// 判断当前模型是否可用（已配置 API Key 且模型已加载）。
    fn is_available(&self) -> bool;

    /// 调用模型生成文本内容。
    ///
    /// # 参数
    /// - `prompt`      — 输入提示词
    /// - `temperature` — 生成温度（0.0 = 确定性，1.0 = 随机）
    /// - `max_tokens`  — 最大输出 token 数
    fn generate_content(&self, prompt: &str, temperature: f32, max_tokens: usize) -> Result<String>;
}
