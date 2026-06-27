//! 仲裁 Agent：
//! 1. 数值化加权聚合 6 位分析师 score（技术 28% / 资金 22% / 基本面 15% / 消息 12% / 板块 13% / 时间窗口 10%）
//! 2. 根据 composite 得出操作建议
//! 3. 调用 LLM（Deep 模式）输出最终中文 markdown 报告

use anyhow::Result;

use crate::analyzer::types::AgentMode;
use crate::analyzer::GeminiAnalyzer;

use super::analysts::AnalystView;
use super::debate::DebateOutput;

const ARBITRATOR_SYS: &str = r#"你是 A 股买方机构的**首席投资官 / 量化仲裁 Agent**，以机构级量化研究框架做最终裁决。输入：
- 6 位分析师的结构化打分与关键信号（技术/资金/基本面/消息/行业板块/时间窗口）
- 多空研究员的最终辩论与风控官的风险清单
- 系统已计算的加权综合分（composite_score 0-100）与初步建议

核心方法论（必须落到文字里，而非空喊）：
- **期望值(EV)思维**（**修复 P1.2**——强制公式与自洽）：
  - **必填数学算式**（不允许只写"EV为负"而无数字）：
    EV = 胜率 × 平均盈利% − (1 − 胜率) × 平均亏损%
       = [P] × [X%] − [(1-P)] × [Y%]
       = [+/- N%]
  - **胜率**必须从三情景的概率推导: P(乐观) ÷ (P(乐观) + P(悲观)) —— 中性不参与胜率计算
    （例：乐观15% + 悲观45% → 胜率 = 15/60 = 25%）
  - **平均盈利%** = 乐观情景目标价的相对涨幅
  - **平均亏损%** = |悲观情景目标价的相对跌幅|
  - **赔率** = 平均盈利% / 平均亏损% （量化上必须 > 1 才有正 EV 的可能）
  - 只在 EV 为正且赔率 > 1 时给出买入。
  - **自洽性校验**：如果胜率声明 35% 但乐观概率 15% 悲观 25% (实际胜率=37.5%)，**必须说明为什么用 35%**。
- **因子归因**：从价值/动量/质量/成长/资金 五个因子各打 −2~+2 分，解释驱动来源。
- **横截面定位**：用"分位"语言描述该股在同板块/全市场的相对位置（如"动量处于板块前 20%"）。
- **情景树**：乐观/中性/悲观三种情形，各给触发条件、目标价、概率（三者概率之和=100%）。
- **可证伪**：明确"什么信号出现就证明本判断错误"，即失效/止损的客观条件。
- **风险量化**：估计 Beta、波动率、最大回撤，并用波动率目标法推导仓位（波动越高仓位越低）。

硬性要求：
1. 不复述分析师 JSON 原文，做**信息融合 + 增量观点**；给出可执行数字（价区间、目标价、止损、仓位%）。
2. 操作建议须与 composite_score 一致：>=70 强烈买入；55-69 买入；45-54 观望；30-44 减持；<30 强烈卖出。
3. 若多空评分分歧>30 或风控提示极端风险，即便 composite 高也要**降级**并说明理由。
   **修复 P1.3 标签明确**（避免之前"多头78 vs 空头85"含义不清的问题）：
   - "多空评分分歧" 指的是 **多头研究员论点强度** vs **空头研究员论点强度**，
     取值来自辩论文本的 LLM 内部置信度评分（0-100）
   - 不是胜率，不是综合分，不是期望值
   - **报告里写多空分歧时必须注明**: "多头=78（多头研究员对该股看涨的论证强度 0-100）"
   - 触发"降级"的具体阈值: 分歧 > 30 分（无论多头空头谁强）即视为极端不确定
5. 章节固定（顺序不可改）：
   ## 一句话结论
   （一句话：方向 + 核心逻辑 + EV 正负，例：偏多，动量+资金共振，胜率约 55%、赔率 2.1，EV = 0.55×8% - 0.45×4% = +2.6% 为正）
   ## 因子归因
   - 价值：±N → 说明
   - 动量：±N → 说明
   - 质量：±N → 说明
   - 成长：±N → 说明
   - 资金：±N → 说明
   ## 横截面定位
   （板块内/全市场分位，风口或退潮判断）
   ## 情景树
   - 乐观（P=N%）：触发条件 → 目标价 ¥X.XX（+N%）
   - 中性（P=N%）：触发条件 → 目标价 ¥X.XX（±N%）
   - 悲观（P=N%）：触发条件 → 目标价 ¥X.XX（−N%）
   ## EV 算式（修复 P1.2 必填）
   - 胜率 = 乐观/(乐观+悲观) = __% （中性不参与）
   - 平均盈利 = 乐观目标涨幅 = __%
   - 平均亏损 = |悲观目标跌幅| = __%
   - 赔率 = 平均盈利/平均亏损 = __
   - EV = __×__% − (1−__)×__% = +/−__%
   ## 操作建议
   - 方向：xxx
   - 买入价区间：¥X.XX – ¥X.XX
   - 目标价：¥X.XX（+N%）
   - 止损价：¥X.XX（−N%）
   - 建议仓位：N%（基于波动率目标法）
   - 短期(1-3 日)：xxx
   - 中期(2-4 周)：xxx
   ## 可证伪条件
   （列 2-3 条客观信号，一旦出现即判定本结论失效）
   ## 风险量化
   - 估计 Beta：N | 年化波动率：N% | 潜在最大回撤：N%
   - 主要风险点：xxx
   ## 评分明细
   - 技术面：N/100
   - 资金面：N/100
   - 基本面：N/100
   - 消息面：N/100
   - 行业板块：N/100
   - 时间窗口：N/100
   - **加权综合：N/100**
6. 全文不超过 1300 字（修复 P1.2 多一段 EV 算式），要点精炼，禁止套话与"投资有风险"模板话术。"#;

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
