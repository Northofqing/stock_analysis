//! 多空研究员（Deep 模式）+ 风控 Agent。
//!
//! 流程：
//! - Round 1: Bull / Bear 各自基于 4 位分析师的结构化视角独立陈述（并行）
//! - Round 2（可选, AI_DEBATE_ROUNDS>=2）: 各自看到对方观点后做出反驳（并行）
//! - Risk Agent: 综合双方最终观点输出风险清单

use crate::analyzer::types::AgentMode;
use crate::analyzer::GeminiAnalyzer;

use super::analysts::AnalystView;

pub(super) struct DebateOutput {
    pub bull: String,
    pub bear: String,
    pub risk: String,
}

const BULL_SYS_R1: &str = r#"你是 A 股**多头研究员**。基于 4 位分析师的结构化评分与信号，**只站在做多视角**提出买入论点。
- 找出最有说服力的多头证据（指出来自哪个分析师的哪个信号）
- 给出建议买入价区间（具体数字 ¥X.XX）、目标价（¥X.XX）、关键支撑位
- 即使整体偏空，也必须找出最佳的潜在做多机会，或明确说"无可建仓"
- 输出 5-8 句中文，逻辑严密，不要和稀泥"#;

const BEAR_SYS_R1: &str = r#"你是 A 股**空头研究员**。基于 4 位分析师的结构化评分与信号，**只站在做空/规避视角**提出卖出/观望论点。
- 找出最有说服力的空头证据（指出来自哪个分析师的哪个信号）
- 给出止损位（¥X.XX）、风险触发条件
- 即使整体偏多，也必须指出潜在风险点
- 输出 5-8 句中文，逻辑严密，不要和稀泥"#;

const BULL_SYS_R2: &str = r#"你是 A 股**多头研究员**。已读取空头方上一轮的论点。任务：
- 逐条反驳空头最薄弱的论据
- 巩固/修正自己的多头观点（可调整目标价/止损位）
- 输出 4-6 句中文，结尾给出最终多头评分（0-100）"#;

const BEAR_SYS_R2: &str = r#"你是 A 股**空头研究员**。已读取多头方上一轮的论点。任务：
- 逐条反驳多头最薄弱的论据
- 巩固/修正自己的空头观点（可调整止损位/风险阈值）
- 输出 4-6 句中文，结尾给出最终空头评分（0-100，越高越看空）"#;

const RISK_SYS: &str = r#"你是 A 股**风险控制官**。阅读多空双方最终论点，输出可执行的风险清单。
不要重复分析师内容，重点关注：
- 极端风险（涨停连板情绪、跌停、监管事件、业绩雷、解禁、地缘政治）
- 流动性与波动率风险
- 仓位建议（轻仓/标准/不建议）
- 明确的止损触发条件（具体价位/跌破均线/放量下跌）
输出 3-5 条要点，中文，每条一行（"- "开头）。"#;

fn analyst_summary(
    fund: &AnalystView,
    tech: &AnalystView,
    cap: &AnalystView,
    news: &AnalystView,
    sector: &AnalystView,
    timeframe: &AnalystView,
) -> String {
    format!(
        "{}\n\n{}\n\n{}\n\n{}\n\n{}\n\n{}",
        tech.to_markdown("技术面分析师"),
        cap.to_markdown("资金面分析师"),
        fund.to_markdown("基本面分析师"),
        news.to_markdown("消息面分析师"),
        sector.to_markdown("行业板块分析师"),
        timeframe.to_markdown("时间窗口分析师"),
    )
}

pub(super) async fn run_debate(
    analyzer: &GeminiAnalyzer,
    basics: &str,
    fund: &AnalystView,
    tech: &AnalystView,
    cap: &AnalystView,
    news: &AnalystView,
    sector: &AnalystView,
    timeframe: &AnalystView,
) -> DebateOutput {
    let summary = analyst_summary(fund, tech, cap, news, sector, timeframe);
    let rounds = analyzer.config.debate_rounds.clamp(1, 3) as u8;

    // Round 1
    let r1_user = format!(
        "# 多空辩论 第 1 轮\n\n## 标的\n{}\n## 6 位分析师视角\n{}\n",
        basics, summary
    );
    let (bull1, bear1) = tokio::join!(
        analyzer.call_api_mode(&r1_user, BULL_SYS_R1, AgentMode::Deep),
        analyzer.call_api_mode(&r1_user, BEAR_SYS_R1, AgentMode::Deep),
    );
    let mut bull = bull1.unwrap_or_else(|e| {
        log::warn!("[Bull R1] {}", e);
        String::new()
    });
    let mut bear = bear1.unwrap_or_else(|e| {
        log::warn!("[Bear R1] {}", e);
        String::new()
    });

    // Round 2: rebut each other
    if rounds >= 2 && !bull.is_empty() && !bear.is_empty() {
        let bull_r2_prompt = format!(
            "# 多空辩论 第 2 轮（反驳）\n\n## 标的\n{}\n## 你方第 1 轮论点\n{}\n\n## 对方（空头）第 1 轮论点\n{}\n",
            basics, bull, bear
        );
        let bear_r2_prompt = format!(
            "# 多空辩论 第 2 轮（反驳）\n\n## 标的\n{}\n## 你方第 1 轮论点\n{}\n\n## 对方（多头）第 1 轮论点\n{}\n",
            basics, bear, bull
        );
        let (bull2, bear2) = tokio::join!(
            analyzer.call_api_mode(&bull_r2_prompt, BULL_SYS_R2, AgentMode::Deep),
            analyzer.call_api_mode(&bear_r2_prompt, BEAR_SYS_R2, AgentMode::Deep),
        );
        if let Ok(b) = bull2 {
            bull = format!("{}\n\n--- 反驳空头 ---\n{}", bull, b);
        }
        if let Ok(b) = bear2 {
            bear = format!("{}\n\n--- 反驳多头 ---\n{}", bear, b);
        }
    }

    // Risk
    let risk_prompt = format!(
        "# 风控审查任务\n\n## 标的\n{}\n## 多头最终论点\n{}\n\n## 空头最终论点\n{}\n",
        basics,
        if bull.trim().is_empty() { "（无）" } else { &bull },
        if bear.trim().is_empty() { "（无）" } else { &bear },
    );
    let risk = analyzer
        .call_api_mode(&risk_prompt, RISK_SYS, AgentMode::Quick)
        .await
        .unwrap_or_else(|e| {
            log::warn!("[Risk Agent] {}", e);
            String::new()
        });

    DebateOutput { bull, bear, risk }
}
