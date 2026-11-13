//! 深度分析入口
//!
//! 提供多角色流水线：
//! - [`run_multi_agent_analysis`]：方案 X 多角色流水线
//!     1. 串行/并行调用 6 个工具拿真实数据（财务/研报/资金/筹码/板块/新闻）
//!     2. `build_slices` 转 [`crate::analyzer::DomainSlices`]
//!     3. 复用 `GeminiAnalyzer::run_text_pipeline` 跑 6 分析师 + 多空辩论 + 仲裁
//!
//! 输出规范：
//! - 报告文件落盘到 `reports/details/{date}_{code}.md`
//! - [`run_and_save`] 默认走多角色路径

use crate::agent::{
    build_slices, FetchChipDistributionTool, FetchFinancialTool, FetchFundFlowTool, FetchNewsTool,
    FetchResearchTool, FetchSectorTool, Tool,
};
use crate::analyzer::AgentMode;
use crate::analyzer::GeminiAnalyzer;
use crate::data_provider::KlineData;
use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

// 修复 P0-E (2026-06-30 codex review): 多 Agent 研判必须 freshness 校验 (AGENTS §2.4, BR-010).
// 之前 deep_analyzer 走 service::get_kline 拿缓存, 无任何新鲜度检查.
// 现在加 daily freshness gate: 日线数据超过 1 交易日 (= dq_daily_stale_sec, 默认 86400s) 直接 bail.
use crate::config;
use crate::data_gateway::{daily_bar_provider_label, HistoricalBarsGateway};
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
    // 修复 v9.4.26 P3 bug: 之前用 kline.last() 取最旧日期 (数据按 date 降序, last 是最旧),
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

#[cfg(test)]
async fn await_kline_fetch<F>(
    future: F,
    timeout: std::time::Duration,
) -> Result<(Vec<KlineData>, &'static str)>
where
    F: std::future::Future<Output = Result<(Vec<KlineData>, &'static str)>>,
{
    tokio::time::timeout(timeout, future)
        .await
        .map_err(|_| anyhow::anyhow!("K 线获取超过 {}ms", timeout.as_millis()))?
}

/// BR-010/BR-156/BR-164: async consumers call the unified Gateway directly.
/// The Gateway owns the blocking isolation, provider routing and admission.
async fn fetch_kline_via_gateway(code: &str, days: usize) -> Result<Vec<KlineData>> {
    let batch = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        HistoricalBarsGateway::new().required_daily_bars_async(code, days),
    )
    .await
    .map_err(|_| anyhow::anyhow!("K 线获取超过 30000ms"))?
    .map_err(anyhow::Error::from)?;
    let (kline, evidence) = batch.into_parts();
    let source = daily_bar_provider_label(evidence.provider);
    let Some(first) = kline.first() else {
        anyhow::bail!("K 线数据源 {source} 返回空序列: {code}");
    };
    let Some(last) = kline.last() else {
        anyhow::bail!("K 线数据源 {source} 返回空序列: {code}");
    };
    let first_date = first.date;
    let last_date = last.date;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn br156_async_kline_path_does_not_nest_runtime() {
        let (bars, source) = await_kline_fetch(
            async { Ok((Vec::new(), "TEST_CODE_ASYNC_SOURCE")) },
            std::time::Duration::from_millis(50),
        )
        .await
        .expect("ready async result");
        assert!(bars.is_empty());
        assert_eq!(source, "TEST_CODE_ASYNC_SOURCE");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn br156_async_kline_timeout_is_explicit() {
        let result = await_kline_fetch(
            async {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                Ok((Vec::new(), "TEST_CODE_LATE_SOURCE"))
            },
            std::time::Duration::from_millis(1),
        )
        .await;
        assert!(result
            .expect_err("late source must time out")
            .to_string()
            .contains("1ms"));
    }
}

/// 报告尾注：数据时点 + 数据源 + 链路/模型 + 免责。
///
/// 2026-11-10 补齐口径：此前 187 份报告 0 处数据时点、0 处免责声明，
/// 读者无法判断数字属于哪一刻、也无法区分研究记录与交易指令。
fn append_provenance_footer(
    markdown: &str,
    data_date: &str,
    data_sources: Option<&str>,
    model: &str,
) -> String {
    let generated_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%:z");
    let sources = data_sources
        .map(|inventory| inventory.replace('\n', "；"))
        .unwrap_or_else(|| "复用主流程已抓取数据（K 线/资金/新闻/财务）".to_string());
    format!(
        "{markdown}\n\n---\n\n\
         数据时点：{data_date} 收盘（日线，最近交易日）\n\
         数据源：{sources}\n\
         分析链路：6 领域分析师 + 多空 2 轮辩论 + 风控 + 仲裁；模型 {model}\n\
         生成时间：{generated_at}\n\
         本报告由自动化多角色分析生成，仅供研究记录，不构成投资建议。\n"
    )
}

fn active_model_label(analyzer: &GeminiAnalyzer) -> String {
    if analyzer.use_doubao {
        format!("豆包 {}", analyzer.doubao_model_for(AgentMode::Deep))
    } else if analyzer.use_deepseek {
        format!("DeepSeek {}", analyzer.deepseek_model_for(AgentMode::Deep))
    } else {
        format!("Gemini {}", analyzer.gemini_model_for(AgentMode::Deep))
    }
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

    // 1. K 线（BR-010/BR-156/BR-164）：统一 Magic Gateway。
    // 250 日窗口一旦命中“未获权威证据的历史停牌缺口”，整批会被 fail-closed；
    // 对深度研判降级到 120 日窗口（完全避开缺口），并在切片里显式标注，
    // 不把缺口推断成停牌，也不丢整只票。
    let mut window_note = None;
    let kline = match fetch_kline_via_gateway(code, 250).await {
        Ok(kline) => kline,
        Err(error) if error.to_string().contains("suspension_evidence_unavailable_v1") => {
            log::warn!(
                "[MultiAgent] {} 250 日窗口含未证实停牌缺口，降级到 120 日窗口: {:#}",
                code,
                error
            );
            window_note = Some(
                "【数据窗口】250 日窗口含未证实停牌缺口，本次使用 120 日窗口；\
                 缺口原因未获权威停牌证据，本报告不推断其性质。"
                    .to_string(),
            );
            fetch_kline_via_gateway(code, 120).await?
        }
        Err(error) => return Err(error),
    };
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
        extra.push_str(flow_str);
        extra.push('\n');
    }
    if !chip_str.is_empty() {
        extra.push_str(chip_str);
        extra.push('\n');
    }
    let extra_ctx = if extra.trim().is_empty() {
        None
    } else {
        Some(extra)
    };

    // 拼装 news_context（消息面分析师读取：新闻 + 研报；板块原文单独进 sector 切片）
    let mut news_ctx = String::new();
    if !news_str_raw.is_empty() {
        news_ctx.push_str("【新闻舆情】\n");
        news_ctx.push_str(news_str_raw);
        news_ctx.push_str("\n\n");
    }
    if !research_str.is_empty() {
        news_ctx.push_str("【机构研报】\n");
        news_ctx.push_str(research_str);
        news_ctx.push('\n');
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
    if let Some(note) = window_note {
        slices.basics.push_str(&format!("\n{}", note));
    }
    // 板块原文属于板块分析师，不再塞进消息面（否则消息面会在“无新闻”时靠板块内容
    // 拿到默认 50 分，虚增权重）。
    if !sector_str.is_empty() {
        slices
            .sector
            .push_str("\n【板块/概念原文（BoardConstituents）】\n");
        slices.sector.push_str(sector_str);
        slices.sector.push('\n');
    }

    // 5. 把财务/估值工具数据注入基本面切片
    if !fin_str.is_empty() {
        // 日线视图没有估值字段时切片会写「（无估值与财务数据）」；拿到工具数据后
        // 必须替换掉这个占位，否则模型会把"有数据"读成"缺失"。
        let base = slices.fundamental.trim();
        slices.fundamental = if base.is_empty() || base == "（无估值与财务数据）" {
            format!("【真实财务/估值数据（gRPC，客户端派生比率）】\n{}", fin_str)
        } else {
            format!(
                "{}\n【真实财务/估值数据（gRPC，客户端派生比率）】\n{}",
                base, fin_str
            )
        };
    } else {
        // 财务/估值工具整体失败：基本面维度弃权，不按中性 50 分混进合成。
        slices.availability.fundamental = false;
    }
    // 修复 P1-F: 把 data_inventory + 强制规则注入 basics 字段 (所有分析师都读).
    // 修复 P1-F: 综合分门控 — 不足 2 个 OK 时禁止给出综合分.
    slices.basics = format!(
        "{}\n\n【数据可用性清单】\n{}\n\n{}",
        slices.basics, data_inventory, DATA_INVENTORY_RULE
    );
    // 2026-11-10: 数据源门从提示词搬进代码 —— 不足 2 个可用来源不出报告。
    // 之前只打 warn，是否给综合分取决于模型自觉（提示词规则）。
    if ok_count < 2 {
        anyhow::bail!(
            "[MultiAgent] {} 可用数据源 {} < 2（{}），按数据门拒绝出报告：{}",
            code,
            ok_count,
            tool_results.len(),
            data_inventory.replace('\n', "；")
        );
    }

    // 6. 调用 GeminiAnalyzer 多 Agent 流水线
    let analyzer = GeminiAnalyzer::from_env();
    log::info!("[MultiAgent] 进入 6 分析师 + 辩论 + 仲裁阶段");
    let markdown = analyzer.run_text_pipeline(slices).await?;
    let data_date = kline.iter().map(|bar| bar.date).max().unwrap_or_default().to_string();
    Ok(append_provenance_footer(
        &markdown,
        &data_date,
        Some(&data_inventory),
        &active_model_label(&analyzer),
    ))
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
    } else {
        slices.availability.fundamental = false;
    }

    let analyzer = GeminiAnalyzer::from_env();
    log::info!("[MultiAgent] 进入 6 分析师 + 辩论 + 仲裁阶段（复用数据）");
    let markdown = analyzer.run_text_pipeline(slices).await?;
    let data_date = seed
        .kline
        .iter()
        .map(|bar| bar.date)
        .max()
        .unwrap_or_default()
        .to_string();
    Ok(append_provenance_footer(
        &markdown,
        &data_date,
        None,
        &active_model_label(&analyzer),
    ))
}

/// 报告文件名用「数据时点」（复盘业务日），不是运行日。
///
/// R-11 复盘在 T+1 的 13:10–15:43 才执行，按运行日命名会让 11-04 的复盘落成
/// `20261105_*.md`，按业务日找不到归档（2026-11-11 报障）。尾注缺失时退回运行日。
fn report_file_date(markdown: &str) -> String {
    const MARKER: &str = "数据时点：";
    if let Some(start) = markdown.find(MARKER) {
        let candidate: String = markdown[start + MARKER.len()..].chars().take(10).collect();
        let bytes = candidate.as_bytes();
        if bytes.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-' {
            return candidate.replace('-', "");
        }
    }
    chrono::Local::now().format("%Y%m%d").to_string()
}

/// 执行深度分析并将结果写入 `DEEP_ANALYZE_REPORT_DIR`（默认
/// `reports/details`）下的 `{date}_{code}.md`。
/// 返回报告文件路径。默认走方案 X（多角色）。
pub async fn run_and_save(code: &str) -> Result<PathBuf> {
    let report = run_multi_agent_analysis(code).await?;
    let date = report_file_date(&report);
    let dir = std::env::var("DEEP_ANALYZE_REPORT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("reports/details"));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}_{}.md", date, code));
    std::fs::write(&path, &report)?;
    log::info!("[DeepAnalyzer] 报告已保存: {}", path.display());
    Ok(path)
}

#[cfg(test)]
mod report_date_tests {
    use super::report_file_date;

    #[test]
    fn report_file_date_prefers_data_time_over_run_time() {
        let report = "## 一句话结论\n...\n\n---\n\n数据时点：2026-11-04 收盘（日线，最近交易日）\n数据源：...";
        assert_eq!(report_file_date(report), "20261104");
    }

    #[test]
    fn report_file_date_falls_back_when_footer_missing() {
        let today = chrono::Local::now().format("%Y%m%d").to_string();
        assert_eq!(report_file_date("## 一句话结论\n无尾注"), today);
        assert_eq!(report_file_date("数据时点：not-a-date"), today);
    }
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
    /// 数据按 date 降序 (最新在前), last() 拿到 1 年前的旧日期, freshness
    /// 永远 reject. 改用 kline.iter().map(|k| k.date).max() 拿最新日期.
    /// 这个测试覆盖: 250 条 K 线, 最新 2026-06-30 + 最旧 2025-06-19 (降序),
    /// freshness 应通过 (取最新日期).
    #[test]
    fn test_check_kline_freshness_uses_latest_date_not_last() {
        use crate::data_provider::KlineData;
        let today = latest_effective_trading_day(chrono::Local::now());
        let old_date = today - chrono::Duration::days(249);
        // 构造降序 K 线: 最新在前, 最旧在后
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
        assert!(!s.contains("[chip]"), "不应含未提供的 tool");
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
