//! 多 Agent 分析流水线（受 TauricResearch/TradingAgents 启发）。
//!
//! ⚠️ 当前状态：已从个股分析路径中**摘除**（成本/失败率原因）。
//! 模块代码保留供后续改造为**新闻/宏观分析**专属多 Agent 使用：
//! 待新增产业链推演 / 政策解读 / 板块轮动 / 情绪师等宏观维度 agents 后再启用。
//!
//! 旧流程（仅文档保留）：
//!   1. 数据切片：build_slices 把行情/资金/财务/新闻/板块按领域切开
//!   2. 6 个领域分析师（Quick）并行 → 结构化 JSON（AnalystView）
//!      - 技术面 / 资金面 / 基本面 / 消息面 / 行业板块 / 时间窗口
//!   3. 多空研究员（Deep）2 轮辩论 + 风控（Quick）
//!   4. 仲裁（Deep）：数值化加权聚合 + LLM 终稿 → 最终中文 markdown
//!
//! 入口：GeminiAnalyzer::run_text_pipeline(slices) -> Result<String>

#![allow(dead_code)]

mod analysts;
mod arbitrator;
mod debate;
pub(super) mod slices;
pub(super) mod trace;

use anyhow::Result;
use log::info;

use super::types::AgentMode;
use super::GeminiAnalyzer;

pub(crate) use slices::{build_slices, DomainSlices};

impl GeminiAnalyzer {
    /// 运行多 Agent 文本流水线，返回最终 markdown 报告。
    pub(super) async fn run_text_pipeline(&self, slices: DomainSlices) -> Result<String> {
        let trace = trace::trace_enabled(self);
        let (provider, quick_model, deep_model) = if self.use_doubao {
            (
                "豆包",
                self.doubao_model_for(AgentMode::Quick),
                self.doubao_model_for(AgentMode::Deep),
            )
        } else if self.use_openai {
            (
                "OpenAI兼容",
                self.openai_model_for(AgentMode::Quick),
                self.openai_model_for(AgentMode::Deep),
            )
        } else {
            (
                "Gemini",
                self.gemini_model_for(AgentMode::Quick),
                self.gemini_model_for(AgentMode::Deep),
            )
        };
        info!(
            "[Agent流水线] 开始 — provider={}, quick={}, deep={}, thinking={}, trace={}",
            provider, quick_model, deep_model, self.config.enable_thinking, trace,
        );
        if trace {
            trace::print_slices(&slices);
        }

        // 1. 6 个分析师 Agent 并行（每人只看自己领域切片，输出结构化 JSON）
        let (fund, tech, cap, news, sector, timeframe) = tokio::join!(
            analysts::run_fundamental(self, &slices),
            analysts::run_technical(self, &slices),
            analysts::run_capital(self, &slices),
            analysts::run_news(self, &slices),
            analysts::run_sector(self, &slices),
            analysts::run_timeframe(self, &slices),
        );

        info!(
            "[Agent流水线] 分析师评分 — 技术={} 资金={} 基本面={} 消息={} 板块={} 时间窗={}",
            tech.score, cap.score, fund.score, news.score, sector.score, timeframe.score
        );
        trace::print_analyst(self, "技术面分析师", &tech);
        trace::print_analyst(self, "资金面分析师", &cap);
        trace::print_analyst(self, "基本面分析师", &fund);
        trace::print_analyst(self, "消息面分析师", &news);
        trace::print_analyst(self, "行业板块分析师", &sector);
        trace::print_analyst(self, "时间窗口分析师", &timeframe);

        // 2. 多空辩论（2 轮）+ 风控
        let debate_out = debate::run_debate(
            self,
            &slices.basics,
            &fund,
            &tech,
            &cap,
            &news,
            &sector,
            &timeframe,
        )
        .await;
        trace::print_debate(self, &debate_out);

        // 3. 仲裁：数值化加权 + LLM 终稿
        let arb = arbitrator::run_arbitrator(
            self,
            &slices.basics,
            &fund,
            &tech,
            &cap,
            &news,
            &sector,
            &timeframe,
            &debate_out,
        )
        .await?;

        info!(
            "[Agent流水线] 完成 — composite_score={}",
            arb.composite_score
        );
        trace::print_final(self, arb.composite_score, &arb.markdown);
        Ok(arb.markdown)
    }
}
