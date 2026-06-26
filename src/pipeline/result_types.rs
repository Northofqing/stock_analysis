//! 分析结果三层分解 — 渐进从 AnalysisResult 130 字段结构体迁移。
//!
//! 当前 AnalysisResult 同时充当数据、分析输出、通知内容、持久化模型。
//! 规划拆分为：
//! - StockFetchData: 原始数据抓取结果
//! - StockAnalysisOutput: 计算后的分析结果
//! - StockNotificationPayload: 格式化后的通知内容
//!
//! 渐进迁移策略：先定义类型，逐步替换内部阶段，最终删除 AnalysisResult。

use super::AnalysisResult;

/// 原始数据抓取结果 — 从 provider 获取的未加工数据
#[allow(dead_code)]
pub struct StockFetchData {
    pub code: String,
    pub name: String,
    pub close: f64,
    pub change_pct: f64,
    pub market_cap: Option<f64>,
    pub kline_count: usize,
}

/// 计算后的分析结果 — pipeline 核心输出
#[allow(dead_code)]
pub struct StockAnalysisOutput {
    pub code: String,
    pub name: String,
    pub sentiment_score: i32,
    pub buy_signal: bool,
    pub sell_signal: bool,
    pub ma_alignment: String,
    pub operation_advice: String,
    pub vetoed: bool,
    pub veto_reasons: Vec<String>,
    pub financial_quality_score: f64,
    pub valuation_score: f64,
    pub total_score: f64,
}

/// 格式化后的通知内容
#[allow(dead_code)]
pub struct StockNotificationPayload {
    pub code: String,
    pub name: String,
    pub emoji: String,
    pub operation_advice: String,
    pub score: i32,
    pub markdown_sections: Vec<String>,
}

impl From<&AnalysisResult> for StockNotificationPayload {
    fn from(r: &AnalysisResult) -> Self {
        Self {
            code: r.code.clone(),
            name: r.name.clone(),
            emoji: r.get_emoji().to_string(),
            operation_advice: r.operation_advice.clone(),
            score: r.sentiment_score,
            markdown_sections: vec![r.analysis_summary.clone()],
        }
    }
}
