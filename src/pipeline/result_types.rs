//! 分析结果三层分解 — 渐进从 AnalysisResult 70+ 字段结构体迁移（CQRS 思路）。
//!
//! 当前 AnalysisResult 同时充当数据、分析输出、通知内容、持久化模型（违反单一职责）。
//! 拆分为三个窄类型，下游可只依赖与自身相关的一层：
//! - StockFetchData: 原始数据抓取结果
//! - StockAnalysisOutput: 计算后的分析结果
//! - StockNotificationPayload: 格式化后的通知内容
//!
//! 迁移策略（strangler fig，零风险）：先提供从 AnalysisResult 的投影转换，
//! 下游消费者逐步改为依赖这三个窄类型，最终反转所有权、删除上帝结构体。
//! 本阶段为**附加式**，不改动既有数据流。

use super::AnalysisResult;

/// 原始数据抓取结果 — 从 provider 获取的未加工数据
#[allow(dead_code)]
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub struct StockNotificationPayload {
    pub code: String,
    pub name: String,
    pub emoji: String,
    pub operation_advice: String,
    pub score: i32,
    pub markdown_sections: Vec<String>,
}

impl From<&AnalysisResult> for StockFetchData {
    fn from(r: &AnalysisResult) -> Self {
        Self {
            code: r.code.clone(),
            name: r.name.clone(),
            close: r.current_price.unwrap_or(0.0),
            change_pct: r.chg_1d.unwrap_or(0.0),
            market_cap: r.market_cap,
            // 投影自上帝结构体时原始 K 线条数已不可得；真实抓取阶段会填充。
            kline_count: 0,
        }
    }
}

impl From<&AnalysisResult> for StockAnalysisOutput {
    fn from(r: &AnalysisResult) -> Self {
        let veto_reasons = r.veto_flags.clone().unwrap_or_default();
        let vetoed = !veto_reasons.is_empty();
        // AnalysisResult 不保存独立的 buy/sell 布尔，从操作建议文本派生
        let advice = r.operation_advice.as_str();
        let buy_signal = advice.contains('买') || advice.contains("加仓");
        let sell_signal =
            advice.contains('卖') || advice.contains("减仓") || advice.contains("止损");
        let (fq, vs) = r
            .score_breakdown
            .as_ref()
            .map(|b| (b.fundamental_quality as f64, b.valuation_safety as f64))
            .unwrap_or((0.0, 0.0));
        Self {
            code: r.code.clone(),
            name: r.name.clone(),
            sentiment_score: r.sentiment_score,
            buy_signal,
            sell_signal,
            ma_alignment: r.ma_alignment.clone().unwrap_or_default(),
            operation_advice: r.operation_advice.clone(),
            vetoed,
            veto_reasons,
            financial_quality_score: fq,
            valuation_score: vs,
            total_score: r.ranking_score as f64,
        }
    }
}

impl From<&AnalysisResult> for StockNotificationPayload {
    fn from(r: &AnalysisResult) -> Self {
        // 汇集所有已渲染的 Markdown 片段，过滤空段，保持通知中的展示顺序
        let candidates = [
            Some(r.analysis_summary.clone()),
            r.score_breakdown_section.clone(),
            r.money_flow_section.clone(),
            r.industry_section.clone(),
            r.quality_section.clone(),
            r.valuation_history_section.clone(),
            r.consensus_section.clone(),
            r.fin_history_section.clone(),
            r.veto_section.clone(),
        ];
        let markdown_sections = candidates
            .into_iter()
            .flatten()
            .filter(|s| !s.trim().is_empty())
            .collect();
        Self {
            code: r.code.clone(),
            name: r.name.clone(),
            emoji: r.get_emoji().to_string(),
            operation_advice: r.operation_advice.clone(),
            score: r.sentiment_score,
            markdown_sections,
        }
    }
}
