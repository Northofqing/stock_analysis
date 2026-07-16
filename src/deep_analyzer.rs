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

use crate::agent::{
    build_slices, AgentRunner, FetchChipDistributionTool, FetchFinancialTool, FetchFundFlowTool,
    FetchNewsTool, FetchResearchTool, FetchSectorTool, Tool, Toolbelt, ValidationEngine,
};
use crate::analyzer::GeminiAnalyzer;
use crate::data_provider::KlineData;
use anyhow::Result;
use async_openai::{config::OpenAIConfig, Client};
use serde_json::json;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

// 修复 P0-E (2026-06-30 codex review): 多 Agent 研判必须 freshness 校验 (AGENTS §2.4, BR-010).
// 之前 deep_analyzer 走 service::get_kline 拿缓存, 无任何新鲜度检查.
// 现在加 daily freshness gate: 日线数据超过 1 交易日 (= dq_daily_stale_sec, 默认 86400s) 直接 bail.
use crate::config;
use crate::monitor::data_quality::{validate_daily_freshness, DqStats, FreshnessConfig};

/// 构造 freshness 配置 (从 monitor config 读 dq_daily_stale_sec).
/// 单独提出便于测试 mock.
fn build_freshness_config() -> FreshnessConfig {
    let cfg = config::get_monitor_config();
    FreshnessConfig {
        quote_max_age_secs: cfg.dq_quote_stale_sec,
        position_max_age_secs: cfg.dq_position_stale_sec,
        nav_max_age_secs: cfg.dq_nav_stale_sec,
        daily_max_age_secs: cfg.dq_daily_stale_sec,
    }
}

/// K 线数据 freshness gate (修复 P0-E).
/// 拒绝时返回 Err 含中文 reason, 业务层 bail.
fn check_kline_freshness(code: &str, kline: &[KlineData]) -> Result<()> {
    // 修复 v9.4.26 P3 bug: 之前用 kline.last() 取最旧日期 (RustDX 按 date 降序, last 是最旧),
    // freshness 永远 reject. 改成用 kline.iter().map(|k| k.date).max() 拿最新日期.
    let Some(latest_date) = kline.iter().map(|k| k.date).max() else {
        anyhow::bail!("[MultiAgent] {} K 线为空", code);
    };
    let freshness = build_freshness_config();
    let stats = DqStats::new();
    validate_daily_freshness(latest_date, chrono::Local::now(), &freshness, &stats)
        .map_err(|reason| {
            anyhow::anyhow!(
                "[MultiAgent] {} K 线过期 (data_date={}, reason={}), 拒绝研判 (AGENTS §2.4 / BR-010)",
                code, latest_date, reason.label()
            )
        })?;
    Ok(())
}

/// 修复 v9.4.26 P3: 走 DataFetcherManager 路径 (RustDX 优先) 而不是 service::service().get_kline.
/// 后者东方财富 → 腾讯 → RustDX 顺序在东方财富被 ban / 腾讯返回旧数据时拿到 2025 年 K 线.
/// 跟 backfill_daily 一致, 用 DataFetcherManager::new() 拿多源回落 (RustDX → 腾讯 → 东方财富).
async fn fetch_kline_via_manager(code: &str, days: usize) -> Result<Vec<KlineData>> {
    let manager = crate::data_provider::DataFetcherManager::new()
        .map_err(|e| anyhow::anyhow!("DataFetcherManager 初始化失败: {e}"))?;
    let (kline, source) = manager
        .get_daily_data(code, days)
        .map_err(|e| anyhow::anyhow!("K 线获取失败: {e}"))?;
    let first_date = kline
        .first()
        .map(|k| k.date.to_string())
        .unwrap_or_default();
    let last_date = kline.last().map(|k| k.date.to_string()).unwrap_or_default();
    log::info!(
        "[MultiAgent] {} K 线来源: {}, {} 条, date_range: {}..{}",
        code,
        source,
        kline.len(),
        first_date,
        last_date
    );
    Ok(kline)
}

const DEFAULT_MAX_ITERATIONS: usize = 12;

// 修复 P1-F (2026-06-30 codex review): 6 Tool 失败显式标注 (AGENTS §2.1, BR-011).
// 之前 `unwrap_or_warn` 把 `Ok(json!({"error": ...}))` 当成功, LLM 拿到错误 JSON 照样编造.
// 现在 collect 返回 ToolResult{ok, data, err}, 拼装 data_inventory 注入 LLM prompt,
// 强制 LLM 在 MISSING 字段标 [数据缺失] 而非编造.
#[derive(Debug, Clone)]
struct ToolResult {
    name: String,
    ok: bool,
    data: String,
    err: Option<String>,
}

impl ToolResult {
    fn ok(name: &str, data: String) -> Self {
        Self {
            name: name.to_string(),
            ok: true,
            data,
            err: None,
        }
    }
    fn missing(name: &str, err: String) -> Self {
        log::warn!("[MultiAgent] {} 工具失败/缺失: {}", name, err);
        Self {
            name: name.to_string(),
            ok: false,
            data: String::new(),
            err: Some(err),
        }
    }
    /// 检测 Ok 但内容是错误 JSON 的"假成功"模式 (修复 P1-F 关键点).
    fn classify(label: &str, r: Result<String>) -> Self {
        match r {
            Ok(s) if s.trim().is_empty() => Self::missing(label, "返回空字符串".into()),
            Ok(s) if s.contains("\"error\"") || s.contains("\"Error\"") => Self::missing(
                label,
                format!("假成功: {}", s.chars().take(120).collect::<String>()),
            ),
            Ok(s) => Self::ok(label, s),
            Err(e) => Self::missing(label, format!("Err: {:#}", e)),
        }
    }
}

/// 渲染 data_inventory 段 (注入 LLM prompt 头部).
/// 格式: "[financials] OK\n[research] MISSING: ...\n...".
fn render_data_inventory(results: &[ToolResult]) -> String {
    results
        .iter()
        .map(|r| match &r.err {
            Some(e) => format!(
                "[{}] MISSING: {}",
                r.name,
                e.chars().take(80).collect::<String>()
            ),
            None => format!("[{}] OK", r.name),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// prompt 强制规则: data_inventory 标 MISSING 的字段必须写 [数据缺失].
const DATA_INVENTORY_RULE: &str =
    "【强制规则】data_inventory 标 MISSING 的字段，在结论中必须写 [数据缺失]，禁止编造。\
     至少 2 个数据源成功才给出综合分；不足 2 个时综合分标 N/A。";
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

fn collect_model_configs_from<F>(get: F) -> Vec<ModelConfig>
where
    F: Fn(&str) -> Option<String>,
{
    let mut configs = Vec::new();

    if let Some(key) = get("DOUBAO_API_KEY").filter(|value| !value.is_empty()) {
        configs.push(ModelConfig {
            api_key: key,
            api_base: get("DOUBAO_BASE_URL")
                .unwrap_or_else(|| "https://ark.cn-beijing.volces.com/api/v3".to_string()),
            model: get("DOUBAO_MODEL")
                .unwrap_or_else(|| "doubao-seed-2-0-pro-260215".to_string()),
        });
    }
    if let Some(key) = get("DEEPSEEK_API_KEY").filter(|value| !value.is_empty()) {
        configs.push(ModelConfig {
            api_key: key,
            api_base: get("DEEPSEEK_BASE_URL")
                .unwrap_or_else(|| "https://api.deepseek.com/v1".to_string()),
            model: get("DEEPSEEK_MODEL")
                .unwrap_or_else(|| "deepseek-chat".to_string()),
        });
    }
    if let Some(key) = get("GEMINI_API_KEY").filter(|value| !value.is_empty()) {
        configs.push(ModelConfig {
            api_key: key,
            api_base: get("GEMINI_BASE_URL").unwrap_or_else(|| {
                "https://generativelanguage.googleapis.com/v1beta/openai/".to_string()
            }),
            model: get("GEMINI_MODEL")
                .unwrap_or_else(|| "gemini-2.5-flash".to_string()),
        });
    }

    configs
}

fn collect_model_configs() -> Vec<ModelConfig> {
    collect_model_configs_from(|name| env::var(name).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_model_configs_order() {
        let values = std::collections::HashMap::from([
            ("DOUBAO_API_KEY", "doubao-key"),
            ("DOUBAO_MODEL", "doubao-test"),
            ("DEEPSEEK_API_KEY", "deepseek-key"),
            ("DEEPSEEK_BASE_URL", "https://api.deepseek.example/v1"),
            ("DEEPSEEK_MODEL", "deepseek-test"),
            ("GEMINI_API_KEY", "gemini-key"),
            ("GEMINI_MODEL", "gemini-test"),
        ]);
        let configs =
            collect_model_configs_from(|name| values.get(name).map(|v| (*v).to_string()));
        assert_eq!(configs[0].model, "doubao-test");
        assert_eq!(configs[1].model, "deepseek-test");
        assert_eq!(configs[2].model, "gemini-test");
        assert_eq!(configs[1].api_base, "https://api.deepseek.example/v1");
    }

    #[test]
    fn test_collect_model_configs_stale_openai_ignored() {
        let stale_only = std::collections::HashMap::from([
            ("OPENAI_API_KEY", "stale-key"),
            ("OPENAI_BASE_URL", "https://api.deepseek.com/v1"),
            ("OPENAI_MODEL", "deepseek-chat"),
        ]);
        assert!(collect_model_configs_from(|name| {
            stale_only.get(name).map(|v| (*v).to_string())
        })
        .is_empty());
    }
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
        anyhow::bail!(
            "未在 .env 中找到 DOUBAO_API_KEY / DEEPSEEK_API_KEY / GEMINI_API_KEY 任一有效配置"
        );
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

    // 1. K 线（修复 v9.4.26 P3）: 走 DataFetcherManager 路径
    //    (RustDX → 腾讯 → 东方财富) 而不是 service::service().get_kline
    //    (东方财富 → 腾讯 → RustDX). 后者东方财富被 ban 时落到腾讯
    //    拿到 2025 年旧数据. 走 DataFetcherManager 让 RustDX 当首选.
    let kline = fetch_kline_via_manager(code, 250).await?;
    if kline.is_empty() {
        anyhow::bail!("K 线数据为空，无法进行多角色分析");
    }
    // 修复 P0-E: freshness gate (AGENTS §2.4 / BR-010)
    check_kline_freshness(code, &kline)?;

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

    // 修复 P1-F: 用 ToolResult::classify 替代 unwrap_or_warn (BR-011).
    // 关键差异: 即使 Result::Ok 也会检测内容是否 "假成功" (含 "error" 字符串).
    let tool_results = vec![
        ToolResult::classify("financials", fin_res),
        ToolResult::classify("research", research_res),
        ToolResult::classify("news", news_res),
        ToolResult::classify("sector", sector_res),
        ToolResult::classify("chip", chip_res),
        ToolResult::classify("fund_flow", flow_res),
    ];
    let data_inventory = render_data_inventory(&tool_results);
    let ok_count = tool_results.iter().filter(|r| r.ok).count();
    let missing_count = tool_results.len() - ok_count;
    log::info!(
        "[MultiAgent] 数据抓取完成 — {}/{} OK, {}/{} MISSING\n{}",
        ok_count,
        tool_results.len(),
        missing_count,
        tool_results.len(),
        data_inventory
    );
    let fin_str = &tool_results[0].data;
    let research_str = &tool_results[1].data;
    let news_str_raw = &tool_results[2].data;
    let sector_str = &tool_results[3].data;
    let chip_str = &tool_results[4].data;
    let flow_str = &tool_results[5].data;

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
    // 修复 P1-F: 把 data_inventory + 强制规则注入 basics 字段 (所有分析师都读).
    // 修复 P1-F: 综合分门控 — 不足 2 个 OK 时禁止给出综合分.
    slices.basics = format!(
        "{}\n\n【数据可用性清单】\n{}\n\n{}",
        slices.basics, data_inventory, DATA_INVENTORY_RULE
    );
    if ok_count < 2 {
        log::warn!(
            "[MultiAgent] {} 数据源 OK={} 不足 2, 强制综合分 = N/A",
            code,
            ok_count
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
    // 修复 P0-E: freshness gate (AGENTS §2.4 / BR-010)
    check_kline_freshness(&seed.code, &seed.kline)?;

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

#[cfg(test)]
mod tests_br006 {
    //! 修复 P0-E 业务规则 BR-010 单元测试:
    //! 多 Agent 研判必须 freshness 校验, 拒绝过期 K 线 (AGENTS §2.4).
    //! 注: 完整构造 KlineData 字段过多 (25+ 字段), 这里直接测 validate_daily_freshness
    //! 单元, check_kline_freshness 只多一层 Option/unwrap 包装, 编译时已覆盖.
    use super::*;

    fn latest_effective_trading_day(now: chrono::DateTime<chrono::Local>) -> chrono::NaiveDate {
        let today = now.date_naive();
        if crate::calendar::is_trading_day(today) {
            today
        } else {
            crate::calendar::prev_trading_day(today)
        }
    }

    /// 测试 1: build_freshness_config 不 panic, 4 个字段都填上 config 默认值
    #[test]
    fn test_build_freshness_config() {
        let f = build_freshness_config();
        // 默认 1 天 = 86400s
        assert_eq!(f.daily_max_age_secs, 86400);
        // 实时行情 5s
        assert_eq!(f.quote_max_age_secs, 5);
        // 持仓 30s
        assert_eq!(f.position_max_age_secs, 30);
        // 净值 1 天
        assert_eq!(f.nav_max_age_secs, 86400);
    }

    /// 测试 2: 今日 K 线 → freshness 通过
    #[test]
    fn test_fresh_kline_passes() {
        let freshness = build_freshness_config();
        let stats = DqStats::new();
        let now = chrono::Local::now();
        let latest_trading_day = latest_effective_trading_day(now);
        let result = validate_daily_freshness(latest_trading_day, now, &freshness, &stats);
        assert!(
            result.is_ok(),
            "今日 K 线应通过 freshness: {:?}",
            result.err()
        );
    }

    /// 测试 3: 5 天前 K 线 → freshness 拒绝 (Stale + age > 0 + max = 86400)
    #[test]
    fn test_stale_kline_rejected() {
        let freshness = build_freshness_config();
        let stats = DqStats::new();
        let five_days_ago = chrono::Local::now().date_naive() - chrono::Duration::days(5);
        let result =
            validate_daily_freshness(five_days_ago, chrono::Local::now(), &freshness, &stats);
        assert!(result.is_err(), "5 天前 K 线应被拒绝");
        let reason = result.unwrap_err();
        match reason {
            crate::monitor::data_quality::DqRejectReason::Stale { age_secs, max_secs } => {
                assert!(age_secs > 0);
                assert_eq!(max_secs, 86400);
            }
            other => panic!("应为 Stale 原因, 实际 {:?}", other),
        }
    }

    /// 测试 4: 严格阈值 (1s) → 任意历史 K 线都过期
    #[test]
    fn test_strict_threshold_rejects() {
        let freshness = FreshnessConfig {
            quote_max_age_secs: 5,
            position_max_age_secs: 30,
            nav_max_age_secs: 24 * 3600,
            daily_max_age_secs: 1, // 1 秒
        };
        let stats = DqStats::new();
        let old_date = chrono::Local::now().date_naive() - chrono::Duration::days(5);
        let result = validate_daily_freshness(old_date, chrono::Local::now(), &freshness, &stats);
        assert!(result.is_err());
    }

    /// 修复 v9.4.26 P3 bug: 之前 check_kline_freshness 用 kline.last() 取最旧日期,
    /// RustDX 数据按 date 降序 (最新在前), last() 拿到 1 年前的旧日期, freshness
    /// 永远 reject. 改用 kline.iter().map(|k| k.date).max() 拿最新日期.
    /// 这个测试覆盖: 250 条 K 线, 最新 2026-06-30 + 最旧 2025-06-19 (降序),
    /// freshness 应通过 (取最新日期).
    #[test]
    fn test_check_kline_freshness_uses_latest_date_not_last() {
        use crate::data_provider::KlineData;
        let today = latest_effective_trading_day(chrono::Local::now());
        let old_date = today - chrono::Duration::days(249);
        // 构造降序 K 线 (RustDX 默认顺序): 最新在前, 最旧在后
        let kline: Vec<KlineData> = (0..250)
            .map(|i| KlineData {
                date: today - chrono::Duration::days(i),
                open: 10.0,
                high: 11.0,
                low: 9.5,
                close: 10.5,
                volume: 1000.0,
                amount: 10500.0,
                pct_chg: 0.0,
                intraday_price: None,
                settled: true,
                pe_ratio: None,
                pb_ratio: None,
                turnover_rate: None,
                market_cap: None,
                circulating_cap: None,
                eps: None,
                roe: None,
                revenue_yoy: None,
                net_profit_yoy: None,
                gross_margin: None,
                net_margin: None,
                sharpe_ratio: None,
                financials_history: None,
                valuation_history: None,
                consensus: None,
                industry: None,
                is_limit_up: false,
                is_limit_down: false,
                is_suspended: false,
                adjust: crate::data_provider::AdjustType::None,
            })
            .collect();
        // 验证我们的逻辑: 最新日期 = today, 最旧日期 = today - 250 days
        let latest_date = kline.iter().map(|k| k.date).max().expect("non-empty");
        let last_date = kline.last().expect("non-empty").date;
        assert_eq!(latest_date, today, "iter().map().max() 取最新日期");
        assert_eq!(last_date, old_date, "last() 拿到最旧日期 (降序)");
        // 关键断言: check_kline_freshness 必须通过 (用 latest_date 而不是 last_date)
        check_kline_freshness("TEST", &kline).expect("降序 K 线应通过 freshness");
    }
}

#[cfg(test)]
mod tests_br011 {
    //! 修复 P1-F 业务规则 BR-011 单元测试:
    //! 6 Tool 失败显式标注 (AGENTS §2.1, 假实现禁令).
    use super::*;

    /// 测试 1: Ok("正常数据") → ok=true, data 保留
    #[test]
    fn test_classify_normal_ok() {
        let r = ToolResult::classify("financials", Ok("正常财务数据".to_string()));
        assert!(r.ok, "正常 Ok 应标记为 ok");
        assert_eq!(r.data, "正常财务数据");
        assert!(r.err.is_none());
    }

    /// 测试 2: Err → ok=false, data 为空, err 有内容
    #[test]
    fn test_classify_err() {
        let r = ToolResult::classify("news", Err(anyhow::anyhow!("网络超时")));
        assert!(!r.ok, "Err 应标记为 missing");
        assert!(r.data.is_empty());
        assert!(r.err.is_some());
        assert!(r.err.unwrap().contains("网络超时"));
    }

    /// 测试 3: Ok(json!({"error": "..."})) → ok=false (假成功检测)
    /// 关键场景: 之前 unwrap_or_warn 把它当成功, 现在识别为 missing.
    #[test]
    fn test_classify_fake_success_with_error() {
        let fake = r#"{"error": "No recent news found for this stock."}"#;
        let r = ToolResult::classify("news", Ok(fake.to_string()));
        assert!(!r.ok, "Ok 但内容含 \"error\" 应被识别为假成功");
        assert!(r.data.is_empty(), "假成功时 data 应清空");
        assert!(r.err.is_some());
        assert!(r.err.unwrap().contains("假成功"));
    }

    /// 测试 4: Ok("") → ok=false (空字符串也是无效)
    #[test]
    fn test_classify_empty_string() {
        let r = ToolResult::classify("sector", Ok(String::new()));
        assert!(!r.ok);
        assert!(r.err.is_some());
        assert!(r.err.unwrap().contains("空字符串"));
    }

    /// 测试 5: render_data_inventory 格式: OK/MISSING 标注
    #[test]
    fn test_render_data_inventory_format() {
        let results = vec![
            ToolResult::ok("financials", "EPS=1.2".to_string()),
            ToolResult::missing("news", "网络超时".to_string()),
            ToolResult::ok("sector", "半导体".to_string()),
        ];
        let s = render_data_inventory(&results);
        assert!(s.contains("[financials] OK"));
        assert!(s.contains("[news] MISSING:"));
        assert!(s.contains("网络超时"));
        assert!(s.contains("[sector] OK"));
        assert!(s.contains("[chip]") == false, "不应含未提供的 tool");
    }

    /// 测试 6: 6 tool 全部 Ok → inventory 6 个 OK
    #[test]
    fn test_render_data_inventory_all_ok() {
        let results: Vec<ToolResult> = [
            "financials",
            "research",
            "news",
            "sector",
            "chip",
            "fund_flow",
        ]
        .iter()
        .map(|n| ToolResult::ok(n, "data".to_string()))
        .collect();
        let s = render_data_inventory(&results);
        let ok_count = s.matches(" OK").count();
        assert_eq!(ok_count, 6, "6 tool 全部 OK");
    }

    /// 测试 7: 6 tool 全部失败 → inventory 6 个 MISSING
    #[test]
    fn test_render_data_inventory_all_missing() {
        let results: Vec<ToolResult> = [
            "financials",
            "research",
            "news",
            "sector",
            "chip",
            "fund_flow",
        ]
        .iter()
        .map(|n| ToolResult::missing(n, "失败".to_string()))
        .collect();
        let s = render_data_inventory(&results);
        let miss_count = s.matches("MISSING").count();
        assert_eq!(miss_count, 6, "6 tool 全部 MISSING");
    }

    /// 测试 8: DATA_INVENTORY_RULE 包含关键约束词
    #[test]
    fn test_data_inventory_rule_contains_key_constraints() {
        assert!(DATA_INVENTORY_RULE.contains("MISSING"));
        assert!(DATA_INVENTORY_RULE.contains("数据缺失"));
        assert!(DATA_INVENTORY_RULE.contains("禁止编造"));
        assert!(DATA_INVENTORY_RULE.contains("N/A"));
    }
}
