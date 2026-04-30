//! 仲裁 Agent：
//! 1. 数值化加权聚合 6 位分析师 score（技术 28% / 资金 22% / 基本面 15% / 消息 12% / 板块 13% / 时间窗口 10%）
//! 2. 根据 composite 得出操作建议
//! 3. 调用 LLM（Deep 模式）输出最终中文 markdown 报告

use anyhow::Result;

use crate::analyzer::types::AgentMode;
use crate::analyzer::GeminiAnalyzer;

use super::analysts::AnalystView;
use super::debate::DebateOutput;

const ARBITRATOR_SYS: &str = r#"你是 A 股**首席投资官 / 仲裁 Agent**。基于：
- 6 位分析师的结构化打分与关键信号（技术/资金/基本面/消息/行业板块/时间窗口）
- 多空研究员的最终辩论
- 风控官的风险清单
- 系统已计算的加权综合分（composite_score 0-100）与初步建议

任务：输出**最终中文 markdown 投研报告**。要求：
1. 不要重复分析师 JSON 原文，要做**信息融合**与**增量观点**
2. 必须**给出具体数字**：买入价区间（¥X.XX-¥X.XX）、目标价（¥X.XX）、止损价（¥X.XX）、建议仓位（百分比 N%）
3. 操作建议必须与 composite_score 一致：>=70 强烈买入；55-70 买入；45-55 观望；30-45 减持；<30 强烈卖出
4. 若多空分析师评分严重分歧（差>30）或风控提示极端风险，应**降级**操作建议（即便 composite 高也要更谨慎）
5. **必须**分别给出"短期(1-3 交易日)"与"中期(2-4 周)"两个时间窗口的操作建议（参考时间窗口分析师）
6. **必须**评估"所属板块当前风口/退潮"判断（参考行业板块分析师）
7. 章节固定：
   ## 综合判断
   ## 操作建议
   - 方向：xxx
   - 买入价区间：¥X.XX – ¥X.XX
   - 目标价：¥X.XX（+N%）
   - 止损价：¥X.XX（-N%）
   - 建议仓位：N%
   - 短期(1-3 日)：xxx
   - 中期(2-4 周)：xxx
   ## 板块联动
   ## 多空辩论关键
   ## 风险提示
   ## 评分明细
   - 技术面：N/100
   - 资金面：N/100
   - 基本面：N/100
   - 消息面：N/100
   - 行业板块：N/100
   - 时间窗口：N/100
   - **加权综合：N/100**
8. 全文不超过 900 字，要点精炼，禁止套话和"投资有风险"模板话术。"#;

pub(super) struct ArbitratorOutput {
    pub markdown: String,
    pub composite_score: i32,
}

#[allow(clippy::too_many_arguments)]
fn weighted_score(
    fund: &AnalystView,
    tech: &AnalystView,
    cap: &AnalystView,
    news: &AnalystView,
    sector: &AnalystView,
    timeframe: &AnalystView,
) -> i32 {
    // 权重合计 1.00：技术 28 / 资金 22 / 基本面 15 / 消息 12 / 板块 13 / 时间窗口 10
    let s = 0.28 * tech.score as f64
        + 0.22 * cap.score as f64
        + 0.15 * fund.score as f64
        + 0.12 * news.score as f64
        + 0.13 * sector.score as f64
        + 0.10 * timeframe.score as f64;
    s.round().clamp(0.0, 100.0) as i32
}

fn preliminary_advice(score: i32) -> &'static str {
    match score {
        70..=100 => "强烈买入",
        55..=69 => "买入",
        45..=54 => "观望",
        30..=44 => "减持",
        _ => "强烈卖出",
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_arbitrator(
    analyzer: &GeminiAnalyzer,
    basics: &str,
    fund: &AnalystView,
    tech: &AnalystView,
    cap: &AnalystView,
    news: &AnalystView,
    sector: &AnalystView,
    timeframe: &AnalystView,
    debate: &DebateOutput,
) -> Result<ArbitratorOutput> {
    let composite = weighted_score(fund, tech, cap, news, sector, timeframe);
    let advice = preliminary_advice(composite);

    let summary = format!(
        "{}\n\n{}\n\n{}\n\n{}\n\n{}\n\n{}",
        tech.to_markdown("技术面分析师"),
        cap.to_markdown("资金面分析师"),
        fund.to_markdown("基本面分析师"),
        news.to_markdown("消息面分析师"),
        sector.to_markdown("行业板块分析师"),
        timeframe.to_markdown("时间窗口分析师"),
    );

    let bull = if debate.bull.trim().is_empty() {
        "（无）"
    } else {
        debate.bull.as_str()
    };
    let bear = if debate.bear.trim().is_empty() {
        "（无）"
    } else {
        debate.bear.as_str()
    };
    let risk = if debate.risk.trim().is_empty() {
        "（无明确风险输入）"
    } else {
        debate.risk.as_str()
    };

    let user_prompt = format!(
        r#"# 仲裁与终稿任务

## 标的
{basics}

## 6 位分析师视角
{summary}

## 多头研究员最终论点
{bull}

## 空头研究员最终论点
{bear}

## 风控官清单
{risk}

## 系统计算
- 加权综合分（composite_score）：{composite}/100
- 初步操作建议：{advice}
- 权重：技术面 28% / 资金面 22% / 基本面 15% / 消息面 12% / 行业板块 13% / 时间窗口 10%

请按系统提示词的章节模板输出最终 markdown 报告。"#,
        basics = basics,
        summary = summary,
        bull = bull,
        bear = bear,
        risk = risk,
        composite = composite,
        advice = advice,
    );

    let markdown = analyzer
        .call_api_mode(&user_prompt, ARBITRATOR_SYS, AgentMode::Deep)
        .await?;

    Ok(ArbitratorOutput {
        markdown,
        composite_score: composite,
    })
}
