//! AI 分析结果与配置数据类型。
//!
//! 从 analyzer.rs 拆分而来，不改变公开 API。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// AI 分析结果数据类 - 决策仪表盘版
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub code: String,
    pub name: String,

    // ========== 核心指标 ==========
    /// 综合评分 0-100 (>70强烈看多, >60看多, 40-60震荡, <40看空)
    pub sentiment_score: i32,
    /// 趋势预测：强烈看多/看多/震荡/看空/强烈看空
    pub trend_prediction: String,
    /// 操作建议：买入/加仓/持有/减仓/卖出/观望
    pub operation_advice: String,
    /// 置信度：高/中/低
    pub confidence_level: String,

    // ========== 决策仪表盘 ==========
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dashboard: Option<Value>,

    // ========== 走势分析 ==========
    #[serde(default)]
    pub trend_analysis: String,
    #[serde(default)]
    pub short_term_outlook: String,
    #[serde(default)]
    pub medium_term_outlook: String,

    // ========== 技术面分析 ==========
    #[serde(default)]
    pub technical_analysis: String,
    #[serde(default)]
    pub ma_analysis: String,
    #[serde(default)]
    pub volume_analysis: String,
    #[serde(default)]
    pub pattern_analysis: String,

    // ========== 基本面分析 ==========
    #[serde(default)]
    pub fundamental_analysis: String,
    #[serde(default)]
    pub sector_position: String,
    #[serde(default)]
    pub company_highlights: String,

    // ========== 情绪面/消息面分析 ==========
    #[serde(default)]
    pub news_summary: String,
    #[serde(default)]
    pub market_sentiment: String,
    #[serde(default)]
    pub hot_topics: String,

    // ========== 综合分析 ==========
    #[serde(default)]
    pub analysis_summary: String,
    #[serde(default)]
    pub key_points: String,
    #[serde(default)]
    pub risk_warning: String,
    #[serde(default)]
    pub buy_reason: String,

    // ========== 元数据 ==========
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_response: Option<String>,
    pub search_performed: bool,
    #[serde(default)]
    pub data_sources: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl AnalysisResult {
    /// 获取核心结论（一句话）
    pub fn get_core_conclusion(&self) -> String {
        if let Some(dashboard) = &self.dashboard {
            if let Some(core) = dashboard.get("core_conclusion") {
                if let Some(sentence) = core.get("one_sentence") {
                    if let Some(s) = sentence.as_str() {
                        return s.to_string();
                    }
                }
            }
        }
        self.analysis_summary.clone()
    }

    /// 获取持仓建议
    pub fn get_position_advice(&self, has_position: bool) -> String {
        if let Some(dashboard) = &self.dashboard {
            if let Some(core) = dashboard.get("core_conclusion") {
                if let Some(advice) = core.get("position_advice") {
                    let key = if has_position { "has_position" } else { "no_position" };
                    if let Some(val) = advice.get(key) {
                        if let Some(s) = val.as_str() {
                            return s.to_string();
                        }
                    }
                }
            }
        }
        self.operation_advice.clone()
    }

    /// 根据操作建议返回对应 emoji
    pub fn get_emoji(&self) -> &'static str {
        match self.operation_advice.as_str() {
            "买入" | "加仓" => "🟢",
            "强烈买入" => "💚",
            "持有" => "🟡",
            "观望" => "⚪",
            "减仓" => "🟠",
            "卖出" => "🔴",
            "强烈卖出" => "❌",
            _ => "🟡",
        }
    }

    /// 返回置信度星级
    pub fn get_confidence_stars(&self) -> &'static str {
        match self.confidence_level.as_str() {
            "高" => "⭐⭐⭐",
            "中" => "⭐⭐",
            "低" => "⭐",
            _ => "⭐⭐",
        }
    }
}

// ============================================================================
// GeminiAnalyzer 主结构
// ============================================================================

/// Gemini AI 分析器配置
#[derive(Debug, Clone)]
pub struct GeminiConfig {
    /// Gemini API Key
    pub api_key: Option<String>,
    /// 主模型名称
    pub model_name: String,
    /// 备选模型名称
    pub fallback_model: String,
    /// 最大重试次数
    pub max_retries: usize,
    /// 重试基础延迟（秒）
    pub retry_delay: f64,
    /// 请求前延迟（秒）
    pub request_delay: f64,
    /// OpenAI 兼容 API 配置
    pub openai_api_key: Option<String>,
    pub openai_base_url: Option<String>,
    pub openai_model: String,
    /// 豆包 (Doubao) API 配置
    pub doubao_api_key: Option<String>,
    pub doubao_base_url: Option<String>,
    pub doubao_model: String,
}

impl Default for GeminiConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            model_name: "gemini-2.0-flash-exp".to_string(),
            fallback_model: "gemini-1.5-flash".to_string(),
            max_retries: 3,
            retry_delay: 5.0,
            request_delay: 1.0,
            openai_api_key: None,
            openai_base_url: None,
            openai_model: "gpt-4".to_string(),
            doubao_api_key: None,
            doubao_base_url: Some("https://ark.cn-beijing.volces.com/api/v3".to_string()),
            doubao_model: "ep-20241230184254-j6pvd".to_string(),
        }
    }
}
