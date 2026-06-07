//! 宏观新闻推荐（从 analyzer.rs 拆分）。
//!
//! 两条路径：
//! - 多 Agent（默认，受 `MACRO_AGENT_PIPELINE` 控制）：4 位宏观专家并行（产业链推演 /
//!   政策解读 / 板块轮动 / 情绪风险）→ 首席策略师融合终稿。
//! - 单 prompt 回退：多 Agent 关闭或终稿失败时使用。

use anyhow::{anyhow, Result};
use log::{info, warn};

use super::{types::AgentMode, GeminiAnalyzer};

/// 宏观专家智能体定义：角色名 + 系统提示词。
struct MacroSpecialist {
    role: &'static str,
    sys: &'static str,
}

const INDUSTRY_CHAIN_SYS: &str = r#"你是 A 股**产业链推演专家**。只做"自上而下的产业链多层传导推演"，不写完整选股报告。
针对今日最核心的 1-3 条新闻，输出：
- 🔥 核心触发事件
- ➡️ 第一层传导（直接受益/情绪驱动）
- ➡️ 第二层传导（卡脖子节点/利润沉淀层）
- ➡️ 第三层传导（扩散层/发散基建）
- 🧱 BOM 结构与重定价点：哪些部件材质/技术更替会引发产业链重新定价，谁在供应
- 🕵️ 预期差与阶段（朦胧/发酵/兑现/退潮）
- ⚠️ 逻辑证伪点
要求：穿透表面行业到卡脖子环节与衍生基建；只输出 Markdown 要点，不超过 450 字。"#;

const POLICY_SYS: &str = r#"你是 A 股**政策解读专家**。只解读今日新闻中的政策/监管/宏观调控信号，不写完整选股报告。
输出：
- 📜 关键政策/事件清单（逐条：方向、力度、持续性）
- ✅ 实质受益方向（有真实订单/产能/补贴落地支撑的）
- ❌ 受压制/收紧方向
- ⏱️ 兑现节奏（一次性事件 vs 长期主线）
要求：区分"口号式利好"与"真金白银落地"；只输出 Markdown 要点，不超过 400 字。"#;

const ROTATION_SYS: &str = r#"你是 A 股**板块轮动与资金风口专家**。只判断当前资金主线与轮动方向，不写完整选股报告。
输出：
- 🌊 当前市场主线（1-2 条）及所处阶段（朦胧/发酵/高潮/退潮）
- 🔁 轮动方向：资金正从哪里流出、流向哪里
- 🚀 正在加速但尚未涨透的低位卡位方向（避免只追高位龙头）
- 🛑 需回避的过热/退潮板块
要求：强调"预期差"和"未被主流资金定价"的方向；只输出 Markdown 要点，不超过 400 字。"#;

const SENTIMENT_SYS: &str = r#"你是 A 股**市场情绪与风险专家**。只评估情绪温度与系统性风险，不写完整选股报告。
输出：
- 🌡️ 市场情绪温度（亢奋/中性/低迷）及依据
- 🧠 逆向思维：当前共识可能错在哪里
- ⚠️ 今日主要风险点（过热兑现/政策反转/外部冲击）
- 🚨 仓位与节奏建议（进攻/均衡/防守）
要求：客观冷静，给出可操作的仓位倾向；只输出 Markdown 要点，不超过 350 字。"#;

const MACRO_SPECIALISTS: &[MacroSpecialist] = &[
    MacroSpecialist { role: "产业链推演专家", sys: INDUSTRY_CHAIN_SYS },
    MacroSpecialist { role: "政策解读专家", sys: POLICY_SYS },
    MacroSpecialist { role: "板块轮动专家", sys: ROTATION_SYS },
    MacroSpecialist { role: "情绪与风险专家", sys: SENTIMENT_SYS },
];

impl GeminiAnalyzer {
    /// 基于宏观新闻，让 AI 推荐当日 A 股受益板块和股票。
    ///
    /// 默认走多 Agent 流水线（4 专家并行 → 首席策略师融合）；
    /// `MACRO_AGENT_PIPELINE=false` 或多 Agent 终稿失败时回退单 prompt。
    pub async fn analyze_macro_recommendations(&self, macro_news: &str) -> Result<String> {
        if !self.is_available() {
            return Err(anyhow!("AI 模型未配置"));
        }
        if macro_news.trim().is_empty() {
            return Err(anyhow!("宏观新闻为空，无法进行推荐"));
        }

        let multi_agent = std::env::var("MACRO_AGENT_PIPELINE")
            .map(|v| v.to_lowercase() != "false")
            .unwrap_or(true);

        if multi_agent {
            match self.analyze_macro_recommendations_multi(macro_news).await {
                Ok(report) => return Ok(report),
                Err(e) => warn!("宏观多 Agent 流水线失败，回退单 prompt：{}", e),
            }
        }
        self.run_macro_synthesis(macro_news, None).await
    }

    /// 多 Agent 路径：4 位宏观专家并行 → 首席策略师融合终稿。
    async fn analyze_macro_recommendations_multi(&self, macro_news: &str) -> Result<String> {
        let today = chrono::Local::now().format("%Y年%m月%d日").to_string();
        info!("🤝 宏观多 Agent 流水线启动（{} 位专家并行）", MACRO_SPECIALISTS.len());

        // 1. 4 位专家并行，各自只输出本领域要点（best-effort，失败不阻断）
        let futures = MACRO_SPECIALISTS
            .iter()
            .map(|sp| self.run_macro_specialist(sp, macro_news, &today));
        let sections = futures::future::join_all(futures).await;

        // 2. 汇总专家视角
        let mut specialist_ctx = String::new();
        let mut ok_count = 0;
        for (sp, sec) in MACRO_SPECIALISTS.iter().zip(sections.iter()) {
            if let Some(text) = sec {
                ok_count += 1;
                specialist_ctx.push_str(&format!("### 【{}】\n{}\n\n", sp.role, text.trim()));
            }
        }
        if ok_count == 0 {
            return Err(anyhow!("全部宏观专家 Agent 失败"));
        }
        info!("✅ 宏观专家完成 {}/{}，进入首席策略师融合", ok_count, MACRO_SPECIALISTS.len());

        // 3. 首席策略师融合终稿
        self.run_macro_synthesis(macro_news, Some(&specialist_ctx)).await
    }

    /// 单个宏观专家 Agent（Quick 模式）。失败返回 None，不阻断流水线。
    async fn run_macro_specialist(
        &self,
        sp: &MacroSpecialist,
        macro_news: &str,
        today: &str,
    ) -> Option<String> {
        let prompt = format!(
            "今天是 {today}。\n\n以下是今日宏观市场最新新闻：\n<macro_news>\n{macro_news}\n</macro_news>\n\n请严格按你的专家角色职责输出本领域要点（只输出 Markdown 要点，不要写完整选股报告，不要捏造股票代码）。",
            today = today,
            macro_news = macro_news
        );
        match self.call_api_mode(&prompt, sp.sys, AgentMode::Quick).await {
            Ok(text) if !text.trim().is_empty() => Some(text),
            Ok(_) => {
                warn!("[宏观Agent] {} 返回空", sp.role);
                None
            }
            Err(e) => {
                warn!("[宏观Agent] {} 失败: {}", sp.role, e);
                None
            }
        }
    }

    /// 首席策略师融合终稿（Deep 模式）：输出最终选股报告。
    ///
    /// `specialist_ctx` 为各专家要点汇总；为 None 时退化为单 prompt 直接分析。
    async fn run_macro_synthesis(
        &self,
        macro_news: &str,
        specialist_ctx: Option<&str>,
    ) -> Result<String> {
        let today = chrono::Local::now().format("%Y年%m月%d日").to_string();

        let specialist_block = match specialist_ctx {
            Some(ctx) => format!(
                "\n以下是 4 位宏观专家的领域研判（请融合、去重、互相印证，提炼为终稿，不要照抄）：\n<specialist_views>\n{}\n</specialist_views>\n",
                ctx
            ),
            None => String::new(),
        };

        let prompt = format!(
r#"今天是 {today}。

以下是今日宏观市场最新新闻：
<macro_news>
{macro_news}
</macro_news>
{specialist_block}
请基于上述宏观信息，从 Top-Down 视角进行 **A 股板块和个股推荐**。

【一、 输出格式要求】
请严格按照以下 Markdown 结构输出（不要输出 JSON，直接输出正文）：

## 📊 宏观环境解读
（2-3 句话：客观概括当前宏观核心驱动因素与市场主线特征）

## 🔗 产业链深度推演（Chain-of-Thought & Expectation Gap）
针对今日最核心的 1-3 条新闻，展示多层次推演链路，穿透利润沉淀点和预期差。
（要求：参考以下范例结构进行推演）
> **推演范例参考**：
> - **🔥 核心触发事件**：国内多地正式出台低空经济产业补贴政策。
> - **➡️ 第一层传导（直接受益/情绪驱动）**：整机制造（eVTOL 飞行器研发）。
> - **➡️ 第二层传导（卡脖子节点/利润沉淀层）**：适航认证壁垒极高的核心零部件（如航空级碳纤维、高能量密度固态电池、航空电机）。
> - **➡️ 第三层传导（扩散层/发散基建）**：空管系统基础设施（低空雷达、5G-A 通感一体化微站）。
> - **➡️ BOM结构分析 **：整机制造的成本构成中，核心零部件占比超过 50%，且具有较高的技术壁垒和议价能力；空管系统基建则是整个产业链的扩散层，虽然目前关注度较低，但未来订单增量巨大；哪些部件在发生材质或技术的更替时，可能会引发整个产业链的重新定价，形成巨大的预期差；这些部件是谁供应的。
> - **🕵️ 预期差与阶段评估**：整机制造已处于发酵期，但第三层空管基建（雷达、通信）尚处于朦胧期，存在未被定价的预期差。
> - **⚠️ 逻辑证伪点**：低空空域开放审批进度不及预期。
> - **📈 资金博弈分析**：目前市场对第一层整机制造炒作过热，第二层核心零部件开始有资金关注，但第三层空管基建仍未被主流资金重视，存在较大预期差。
> - **🧠 逆向思维**：虽然整体制造受政策刺激，但如果核心环节研发进展不及预期，可能导致整个产业链的兑现风险，投资者应警惕过热炒作带来的回调风险。

## 🏭 核心受益板块与个股解析（Top 3-5 个板块）
对筛选出的板块进行深度解析（需涵盖真实业务增量分析），并直接输出重点关注个股：

### 1. [板块名称]（产业链位置：如 中游利润层）
- **核心逻辑与业务支撑**：说明该板块是否有真实的订单暴增、产能吃紧、产品涨价或重磅政策落地作为支撑。
- **阶段与风险**：指出当前的炒作阶段（朦胧/高潮等）及主要风险点。
- **重点个股池**：
  | 股票代码 | 股票名称 | 个股逻辑与地位（龙头/中军/补涨） |
  | :--- | :--- | :--- |
  | 600XXX | XXXA | 市场绝对龙头，拥有核心卡脖子技术，市占率第一 |

### 2. [同上...]
（继续输出第2-5个板块...）

## ⚠️ 今日需回避的板块
（列出 1-3 个宏观不利板块并说明实质性利空原因，如受关税影响、供给过剩等）

## 📋 操作建议摘要
（100字以内：总结今日整体仓位建议与应对突发宏观事件的操作策略）

## 📌 推荐代码汇总
【推荐代码】在此处按精确的逗号分隔列出全篇提及的 A 股 6 位代码（如 600519,000001），不要换行，不要遗漏。

【二、 核心研判法则（必须严格遵守）】
1. **深度透视，拒绝平铺**：不要只推荐新闻表面提到的行业，必须向深处推演卡脖子环节及衍生基建。
2. **防追高与防幻觉**：优先推荐有真实基本面的细分龙头和中军。**绝不可编造股票代码**，若对某公司的 A 股 6 位代码不确定，请以公司名本身代替代码或直接放弃推荐该股。
3. **强相关性**：所有的板块和个股推荐逻辑，必须能从 <macro_news> 中直接或间接推导出来。
4. **逆向思维**：在推荐受益板块的同时，务必指出至少 1 个板块风险点，并给出实质性理由。
"#,
            today = today,
            macro_news = macro_news,
            specialist_block = specialist_block,
        );

        // 使用宏观推荐专用系统提示词
        const MACRO_SYSTEM_PROMPT: &str = "\
你是一位顶尖的 A 股机构宏观量化策略师，专精于自上而下 (Top-Down) 宏观驱动选股，深谙A股资金博弈与产业链传导逻辑。
你的核心能力：
1. 产业链多层推演：不仅能找到直接受益板块，还能精准定位产业链的【卡脖子节点/利润沉淀层】与衍生基建。
2. 剥离伪概念：能够区分“纯情绪炒作”与“有实际订单/产能/涨价支撑的核心主线”。
3. 预期差分析：评估事件的定价阶段（朦胧/发酵/兑现/退潮），警惕高潮兑现风险。

【严格约束】：
1. 语气必须是纯粹的机构研报风格：客观、冷静、数据驱动。
2. 严禁任何形式的套话、废话（如“好的，以下是分析”、“希望这能帮到您”等），直接从正文标题开始输出。
3. 绝对禁止捏造A股股票代码，不知道代码的股票坚决不写代码。";

        let mode = if specialist_ctx.is_some() {
            AgentMode::Deep
        } else {
            AgentMode::Quick
        };
        info!("🤖 首席策略师生成宏观推荐终稿（mode={:?}）...", mode);
        let response = self
            .call_api_mode(&prompt, MACRO_SYSTEM_PROMPT, mode)
            .await?;
        Ok(response)
    }
}
