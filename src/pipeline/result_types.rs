//! Registered business rule: BR-122.
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
    pub close: Option<f64>,
    pub change_pct: Option<f64>,
    pub market_cap: Option<f64>,
    pub kline_count: Option<usize>,
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
    pub ma_alignment: Option<String>,
    pub operation_advice: String,
    pub vetoed: Option<bool>,
    pub veto_reasons: Option<Vec<String>>,
    pub financial_quality_score: Option<f64>,
    pub valuation_score: Option<f64>,
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
            close: r.current_price,
            change_pct: r.chg_1d,
            market_cap: r.market_cap,
            // 投影自上帝结构体时原始 K 线条数已不可得；真实抓取阶段会填充。
            kline_count: None,
        }
    }
}

impl From<&AnalysisResult> for StockAnalysisOutput {
    fn from(r: &AnalysisResult) -> Self {
        let veto_reasons = r.veto_flags.clone();
        let vetoed = veto_reasons.as_ref().map(|reasons| !reasons.is_empty());
        // AnalysisResult 不保存独立的 buy/sell 布尔，从操作建议文本派生
        let advice = r.operation_advice.as_str();
        let buy_signal = advice.contains('买') || advice.contains("加仓");
        let sell_signal =
            advice.contains('卖') || advice.contains("减仓") || advice.contains("止损");
        let (fq, vs) = match r.score_breakdown.as_ref() {
            Some(breakdown) => (
                Some(breakdown.fundamental_quality as f64),
                Some(breakdown.valuation_safety as f64),
            ),
            None => (None, None),
        };
        Self {
            code: r.code.clone(),
            name: r.name.clone(),
            sentiment_score: r.sentiment_score,
            buy_signal,
            sell_signal,
            ma_alignment: r.ma_alignment.clone(),
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

#[cfg(test)]
mod tests {
    use super::{StockAnalysisOutput, StockFetchData, StockNotificationPayload};
    use crate::pipeline::{AnalysisResult, ScoreBreakdown};

    fn result() -> AnalysisResult {
        serde_json::from_value(serde_json::json!({
            "code": "TEST_CODE_000001",
            "name": "TEST_CODE_示例",
            "sentiment_score": 50,
            "ranking_score": 61,
            "operation_advice": "观望",
            "trend_prediction": "TEST_CODE_盘整",
            "analysis_summary": "TEST_CODE_正文",
            "is_limit_up": false,
            "contrarian_signal": false
        }))
        .expect("valid result fixture")
    }

    #[test]
    fn missing_fetch_and_analysis_facts_remain_none() {
        let result = result();
        let fetch = StockFetchData::from(&result);
        assert_eq!(fetch.close, None);
        assert_eq!(fetch.change_pct, None);
        assert_eq!(fetch.kline_count, None);
        assert_eq!(fetch.market_cap, None);

        let analysis = StockAnalysisOutput::from(&result);
        assert_eq!(analysis.ma_alignment, None);
        assert_eq!(analysis.vetoed, None);
        assert_eq!(analysis.veto_reasons, None);
        assert_eq!(analysis.financial_quality_score, None);
        assert_eq!(analysis.valuation_score, None);
        assert!(!analysis.buy_signal);
        assert!(!analysis.sell_signal);
        assert_eq!(analysis.total_score, 61.0);
    }

    #[test]
    fn complete_projection_preserves_signals_scores_and_section_order() {
        let mut result = result();
        result.current_price = Some(10.5);
        result.chg_1d = Some(2.0);
        result.market_cap = Some(1_000_000.0);
        result.ma_alignment = Some("TEST_CODE_多头".to_string());
        result.operation_advice = "建议买入并加仓后减仓止损卖出".to_string();
        result.veto_flags = Some(vec!["TEST_CODE_否决".to_string()]);
        result.score_breakdown = Some(ScoreBreakdown {
            fundamental_quality: 72,
            valuation_safety: 64,
            ..Default::default()
        });
        result.score_breakdown_section = Some("TEST_CODE_评分".to_string());
        result.money_flow_section = Some("  ".to_string());
        result.industry_section = Some("TEST_CODE_行业".to_string());
        result.veto_section = Some("TEST_CODE_风险".to_string());

        let fetch = StockFetchData::from(&result);
        assert_eq!(fetch.close, Some(10.5));
        assert_eq!(fetch.change_pct, Some(2.0));
        assert_eq!(fetch.market_cap, Some(1_000_000.0));

        let analysis = StockAnalysisOutput::from(&result);
        assert_eq!(analysis.ma_alignment.as_deref(), Some("TEST_CODE_多头"));
        assert_eq!(analysis.vetoed, Some(true));
        assert_eq!(
            analysis.veto_reasons.as_deref(),
            Some(&["TEST_CODE_否决".to_string()][..])
        );
        assert_eq!(analysis.financial_quality_score, Some(72.0));
        assert_eq!(analysis.valuation_score, Some(64.0));
        assert!(analysis.buy_signal);
        assert!(analysis.sell_signal);

        let payload = StockNotificationPayload::from(&result);
        assert_eq!(
            payload.markdown_sections,
            vec![
                "TEST_CODE_正文".to_string(),
                "TEST_CODE_评分".to_string(),
                "TEST_CODE_行业".to_string(),
                "TEST_CODE_风险".to_string(),
            ]
        );
        assert_eq!(payload.score, 50);
        assert!(!payload.emoji.is_empty());
    }

    #[test]
    fn empty_veto_batch_is_known_not_vetoed_and_each_signal_is_independent() {
        let mut result = result();
        result.veto_flags = Some(Vec::new());
        result.operation_advice = "建议加仓".to_string();
        let buy = StockAnalysisOutput::from(&result);
        assert_eq!(buy.vetoed, Some(false));
        assert!(buy.buy_signal);
        assert!(!buy.sell_signal);

        result.operation_advice = "建议减仓".to_string();
        let sell = StockAnalysisOutput::from(&result);
        assert!(!sell.buy_signal);
        assert!(sell.sell_signal);
    }
}
