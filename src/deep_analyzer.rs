//! 深度分析入口
//!
//! 提供两条路径：
//! - [`run_react_analysis`]：单角色 ReAct Agent + Critic（保留备用，原 `--deep-analysis` 实现）
//! - [`run_multi_agent_analysis`]：方案 X 多角色流水线
//!     1. 串行/并行调用 6 个工具拿真实数据（财务/研报/资金/筹码/板块/新闻）
//!     2. `build_slices` 转 [`crate::analyzer::DomainSlices`]
//!     3. 复用 `GeminiAnalyzer::run_text_pipeline` 跑 6 分析师 + 多空辩论 + 仲裁
//!
//! 输出规范：
//! - 报告文件落盘到 `reports/details/{date}_{code}.md`
//! - [`run_and_save`] 默认走多角色路径

use crate::agent::loop_runner::AgentRunner;
use crate::agent::multi_agent::build_slices;
use crate::agent::tool::Tool;
use crate::agent::toolbelt::Toolbelt;
use crate::agent::tools::FetchFinancialTool;
use crate::agent::tools_chip::FetchChipDistributionTool;
use crate::agent::tools_money_flow::FetchFundFlowTool;
use crate::agent::tools_news::FetchNewsTool;
use crate::agent::tools_research::FetchResearchTool;
use crate::agent::tools_sector::FetchSectorTool;
use crate::agent::validation::ValidationEngine;
use crate::analyzer::GeminiAnalyzer;
use crate::data_provider::service;
use crate::data_provider::KlineData;
use anyhow::Result;
use async_openai::{config::OpenAIConfig, Client};
use serde_json::json;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

const DEFAULT_MAX_ITERATIONS: usize = 12;
const DEFAULT_SYSTEM_PROMPT: &str = "你是一个专精于 A 股深研的金融量化 Agent。\n\
你的任务是通过调用提供的工具（Tool）来获取真实数据，严禁编造财务数据。\n\
在拿到数据后，你需要做交叉分析。如果在分析中触发了系统校验失败，请立刻通过报错提示进行修正，不要固执己见。\n\
请根据个股数据和行业板块联动，给出最终深入的评估。";

/// 主流程（`process_stock`）已抓取数据的快照，供重点股深度研判复用，
/// 避免在 [`run_multi_agent_analysis`] 中重复抓取 K线/资金/新闻/财务等数据。
#[derive(Debug, Clone)]
pub struct DeepAnalysisSeed {
    pub code: String,
    pub name: String,
    /// 主流程抓取的 K 线（Arc 共享，零拷贝传递给深度分析）。
    pub kline: Arc<Vec<KlineData>>,
    /// 资金面上下文：真实主力资金流 + 筹码 + 多周期（资金面切片复用）。
    pub extra_context: Option<String>,
    /// 消息面上下文：新闻舆情（消息面切片复用）。
    pub news_context: Option<String>,
    /// 宏观背景。
    pub macro_context: Option<String>,
    /// 主流程已渲染的财务/估值/研报/行业对标（基本面切片增强，复用避免重复抓取）。
    pub fundamental_ctx: Option<String>,
    /// 系统趋势研判的中间证据快照（仅技术面 / 时间窗口分析师可见）。
    pub trend_snapshot: TrendSnapshot,
}

/// 系统趋势研判的中间证据快照。
///
/// 只携带「证据型」字段，**刻意排除** `signal_score` / `buy_signal` 等最终结论，
/// 避免深度分析师直接照搬系统结论形成自证循环。
#[derive(Debug, Clone)]
pub struct TrendSnapshot {
    pub trend_status: String,
    pub ma_alignment: String,
    pub trend_strength: f64,
    pub bias_ma5: f64,
    pub volume_status: String,
    pub volume_ratio_5d: f64,
    pub support_levels: Vec<f64>,
    pub resistance_levels: Vec<f64>,
    /// 系统记录的技术线索（来自 signal_reasons，作为证据而非结论）。
    pub evidence_reasons: Vec<String>,
    pub risk_factors: Vec<String>,
}

#[derive(Debug, Clone)]
struct ModelConfig {
    api_key: String,
    api_base: String,
    model: String,
}

fn collect_model_configs() -> Vec<ModelConfig> {
    let mut configs = Vec::new();

    if let Some(key) = env::var("DOUBAO_API_KEY").ok().filter(|k| !k.is_empty()) {
        configs.push(ModelConfig {
            api_key: key,
            api_base: env::var("DOUBAO_BASE_URL")
                .unwrap_or_else(|_| "https://ark.cn-beijing.volces.com/api/v3".to_string()),
            model: env::var("DOUBAO_MODEL")
                .unwrap_or_else(|_| "doubao-seed-2-0-pro-260215".to_string()),
        });
    }
    if let Some(key) = env::var("OPENAI_API_KEY").ok().filter(|k| !k.is_empty()) {
        configs.push(ModelConfig {
            api_key: key,
            api_base: env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
            model: env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string()),
        });
    }
    if let Some(key) = env::var("GEMINI_API_KEY").ok().filter(|k| !k.is_empty()) {
        configs.push(ModelConfig {
            api_key: key,
            api_base: env::var("GEMINI_BASE_URL").unwrap_or_else(|_| {
                "https://generativelanguage.googleapis.com/v1beta/openai/".to_string()
            }),
            model: env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-2.5-flash".to_string()),
        });
    }
    configs
}

fn build_client(cfg: &ModelConfig) -> Client<OpenAIConfig> {
    let openai_cfg = OpenAIConfig::new()
        .with_api_key(cfg.api_key.clone())
        .with_api_base(cfg.api_base.clone());
    Client::with_config(openai_cfg)
}

fn build_toolbelt() -> Toolbelt {
    let mut toolbelt = Toolbelt::new();
    toolbelt.register(FetchFinancialTool::new());
    toolbelt.register(FetchSectorTool::new());
    toolbelt.register(FetchResearchTool::new());
    toolbelt.register(FetchFundFlowTool::new());
    toolbelt.register(FetchChipDistributionTool::new());
    toolbelt.register(FetchNewsTool::new());
    toolbelt
}

/// 对单只股票运行 ReAct Agent 深度分析（旧路径，保留备用）。
///
/// 返回最终的 Markdown 报告文本（即使 Critic 未通过，也会返回带警示的草稿）。
pub async fn run_react_analysis(code: &str) -> Result<String> {
    let model_configs = collect_model_configs();
    if model_configs.is_empty() {
        anyhow::bail!("未在 .env 中找到 DOUBAO_API_KEY / OPENAI_API_KEY / GEMINI_API_KEY 任一有效配置");
    }
    log::info!(
        "[DeepAnalyzer] 加载 {} 个模型配置，主模型: {}，fallback 链: {}",
        model_configs.len(),
        model_configs[0].model,
        model_configs[1..]
            .iter()
            .map(|c| c.model.as_str())
            .collect::<Vec<_>>()
            .join(" -> ")
    );

    let primary = &model_configs[0];
    let primary_client = build_client(primary);
    let primary_model = primary.model.clone();
    let fallback_pairs: Vec<(Client<OpenAIConfig>, String)> = model_configs[1..]
        .iter()
        .map(|c| (build_client(c), c.model.clone()))
        .collect();

    let toolbelt = build_toolbelt();
    let validation = ValidationEngine::new_with_defaults();

    let mut agent = AgentRunner::new(
        primary_client,
        toolbelt,
        validation,
        DEFAULT_SYSTEM_PROMPT.to_string(),
        primary_model,
    )
    .with_fallbacks(fallback_pairs);

    let query = format!(
        "请帮我重点看看股票 {} 的财报，以及它最近的板块联动效应。要求内容详实。",
        code
    );

    agent.run(&query, DEFAULT_MAX_ITERATIONS).await
}

/// 方案 X：多角色 + 工具数据 + 辩论仲裁 流水线。
///
/// 步骤：
/// 1. 并行调用 6 个工具拿真实数据（财务/研报/资金流/筹码/板块/新闻）+ K 线
/// 2. 把工具结果拼装成 `extra_context` / `news_context` 文本
/// 3. `build_slices` 切片
/// 4. `GeminiAnalyzer::run_text_pipeline` 跑 6 分析师 + 多空辩论 + 仲裁
pub async fn run_multi_agent_analysis(code: &str) -> Result<String> {
    log::info!("[MultiAgent] 开始抓取数据：{}", code);

    // 1. K 线（service 缓存）
    let kline = service::service().get_kline(code, 250).await?;
    if kline.is_empty() {
        anyhow::bail!("K 线数据为空，无法进行多角色分析");
    }

    // 2. 6 个工具并行
    let fin_tool = FetchFinancialTool::new();
    let research_tool = FetchResearchTool::new();
    let news_tool = FetchNewsTool::new();
    let sector_tool = FetchSectorTool::new();
    let chip_tool = FetchChipDistributionTool::new();
    let flow_tool = FetchFundFlowTool::new();

    let code_input = json!({"code": code});
    let news_input = json!({"code": code, "name": ""});

    let (fin_res, research_res, news_res, sector_res, chip_res, flow_res) = tokio::join!(
        fin_tool.call(code_input.clone()),
        research_tool.call(code_input.clone()),
        news_tool.call(news_input),
        sector_tool.call(code_input.clone()),
        chip_tool.call(code_input.clone()),
        flow_tool.call(code_input),
    );

    let unwrap_or_warn = |label: &str, r: Result<String>| -> String {
        match r {
            Ok(s) => s,
            Err(e) => {
                log::warn!("[MultiAgent] {} 工具失败: {:#}", label, e);
                String::new()
            }
        }
    };
    let fin_str = unwrap_or_warn("financials", fin_res);
    let research_str = unwrap_or_warn("research", research_res);
    let news_str_raw = unwrap_or_warn("news", news_res);
    let sector_str = unwrap_or_warn("sector", sector_res);
    let chip_str = unwrap_or_warn("chip", chip_res);
    let flow_str = unwrap_or_warn("fund_flow", flow_res);

    log::info!(
        "[MultiAgent] 数据抓取完成 — 财务={} 研报={} 新闻={} 板块={} 筹码={} 资金={}",
        !fin_str.is_empty(),
        !research_str.is_empty(),
        !news_str_raw.is_empty(),
        !sector_str.is_empty(),
        !chip_str.is_empty(),
        !flow_str.is_empty()
    );

    // 3. 拼装 extra_context（资金面分析师读取）= 资金流 + 筹码
    let mut extra = String::new();
    if !flow_str.is_empty() {
        extra.push_str(&flow_str);
        extra.push_str("\n");
    }
    if !chip_str.is_empty() {
        extra.push_str(&chip_str);
        extra.push_str("\n");
    }
    let extra_ctx = if extra.trim().is_empty() {
        None
    } else {
        Some(extra)
    };

    // 拼装 news_context（消息面分析师 + 行业板块分析师读取）
    let mut news_ctx = String::new();
    if !news_str_raw.is_empty() {
        news_ctx.push_str("【新闻舆情】\n");
        news_ctx.push_str(&news_str_raw);
        news_ctx.push_str("\n\n");
    }
    if !sector_str.is_empty() {
        news_ctx.push_str("【板块/概念】\n");
        news_ctx.push_str(&sector_str);
        news_ctx.push_str("\n\n");
    }
    if !research_str.is_empty() {
        news_ctx.push_str("【机构研报】\n");
        news_ctx.push_str(&research_str);
        news_ctx.push_str("\n");
    }
    let news_ctx_opt = if news_ctx.trim().is_empty() {
        None
    } else {
        Some(news_ctx)
    };

    // 4. build_slices
    let mut slices = build_slices(
        code,
        None,
        &kline,
        extra_ctx.as_deref(),
        news_ctx_opt.as_deref(),
        None,
        None,
    );

    // 5. 把财务工具数据注入基本面切片
    if !fin_str.is_empty() {
        slices.fundamental = format!(
            "{}\n【真实财务指标 (来自 fetch_financials 工具)】\n{}",
            slices.fundamental, fin_str
        );
    }

    // 6. 调用 GeminiAnalyzer 多 Agent 流水线
    let analyzer = GeminiAnalyzer::from_env();
    log::info!("[MultiAgent] 进入 6 分析师 + 辩论 + 仲裁阶段");
    analyzer.run_text_pipeline(slices).await
}

/// 复用主流程数据的多角色流水线（重点股深度研判走此路径）。
///
/// 与 [`run_multi_agent_analysis`] 的区别：**不再重复抓取**任何数据，
/// 直接复用 [`DeepAnalysisSeed`] 中主流程已获取的 K线/资金/新闻/财务，
/// 并把系统趋势快照注入技术面 / 时间窗口分析师。
pub async fn run_multi_agent_analysis_with_seed(seed: &DeepAnalysisSeed) -> Result<String> {
    log::info!("[MultiAgent] 复用主流程数据，跳过重复抓取：{}", seed.code);
    if seed.kline.is_empty() {
        anyhow::bail!("K 线数据为空，无法进行多角色分析");
    }

    let mut slices = build_slices(
        &seed.code,
        Some(&seed.name),
        &seed.kline,
        seed.extra_context.as_deref(),
        seed.news_context.as_deref(),
        seed.macro_context.as_deref(),
        Some(&seed.trend_snapshot),
    );

    if let Some(fund) = seed.fundamental_ctx.as_deref() {
        slices.fundamental = format!(
            "{}\n【主流程已抓取的财务/估值/研报/行业数据】\n{}",
            slices.fundamental, fund
        );
    }

    let analyzer = GeminiAnalyzer::from_env();
    log::info!("[MultiAgent] 进入 6 分析师 + 辩论 + 仲裁阶段（复用数据）");
    analyzer.run_text_pipeline(slices).await
}

/// 执行深度分析并将结果写入到 `reports/details/{date}_{code}.md`。
/// 返回报告文件路径。默认走方案 X（多角色）。
pub async fn run_and_save(code: &str) -> Result<PathBuf> {
    let report = run_multi_agent_analysis(code).await?;
    let date = chrono::Local::now().format("%Y%m%d").to_string();
    let dir = PathBuf::from("reports/details");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}_{}.md", date, code));
    std::fs::write(&path, &report)?;
    log::info!("[DeepAnalyzer] 报告已保存: {}", path.display());
    Ok(path)
}
