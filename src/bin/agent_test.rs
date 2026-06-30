use stock_analysis::agent::loop_runner::AgentRunner;
use stock_analysis::agent::toolbelt::Toolbelt;
use stock_analysis::agent::tools_money_flow::FetchFundFlowTool;
use stock_analysis::agent::tools_chip::FetchChipDistributionTool;
use stock_analysis::agent::tools_news::FetchNewsTool;
use stock_analysis::agent::tools::FetchFinancialTool;
use stock_analysis::agent::tools_sector::FetchSectorTool;
use stock_analysis::agent::tools_research::FetchResearchTool;
use stock_analysis::agent::validation::ValidationEngine;
use stock_analysis::database::DatabaseManager;
use async_openai::{config::OpenAIConfig, Client};
use dotenvy::dotenv;

use std::env;

struct ActiveModelConfig {
    api_key: String,
    api_base: String,
    model: String,
}

fn collect_model_configs() -> Vec<ActiveModelConfig> {
    let mut configs = Vec::new();

    if let Some(key) = env::var("DOUBAO_API_KEY").ok().filter(|k| !k.is_empty()) {
        let base = env::var("DOUBAO_BASE_URL").unwrap_or_else(|_| "https://ark.cn-beijing.volces.com/api/v3".to_string());
        let model = env::var("DOUBAO_MODEL").unwrap_or_else(|_| "doubao-seed-2-0-pro-260215".to_string());
        configs.push(ActiveModelConfig { api_key: key, api_base: base, model });
    }
    if let Some(key) = env::var("OPENAI_API_KEY").ok().filter(|k| !k.is_empty()) {
        let base = env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
        let model = env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
        configs.push(ActiveModelConfig { api_key: key, api_base: base, model });
    }
    if let Some(key) = env::var("GEMINI_API_KEY").ok().filter(|k| !k.is_empty()) {
        let base = env::var("GEMINI_BASE_URL").unwrap_or_else(|_| "https://generativelanguage.googleapis.com/v1beta/openai/".to_string());
        let model = env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-2.5-flash".to_string());
        configs.push(ActiveModelConfig { api_key: key, api_base: base, model });
    }

    if configs.is_empty() {
        panic!("未在 .env 中找到任何有效的 DOUBAO_API_KEY, OPENAI_API_KEY 或 GEMINI_API_KEY");
    }
    println!(">>> 加载了 {} 个模型配置，优先级顺序：{}",
        configs.len(),
        configs.iter().map(|c| c.model.as_str()).collect::<Vec<_>>().join(" -> "));
    configs
}

fn build_client(cfg: &ActiveModelConfig) -> Client<OpenAIConfig> {
    let openai_cfg = OpenAIConfig::new()
        .with_api_key(cfg.api_key.clone())
        .with_api_base(cfg.api_base.clone());
    Client::with_config(openai_cfg)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    // 1. 初始化环境变量和日志
    dotenv().ok();
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();

    println!("正在初始化 Agent 测试环境...");

    // 2. 初始化数据库以支持 Scratchpad 记录持久化
    DatabaseManager::init(None).map_err(|e| anyhow::anyhow!("DB init error: {:?}", e))?;

    // 3. 配置智能体客户端（主模型 + fallback 模型）
    let model_configs = collect_model_configs();
    let primary = &model_configs[0];
    let primary_client = build_client(primary);
    let primary_model_name = primary.model.clone();
    let fallback_pairs: Vec<(Client<OpenAIConfig>, String)> = model_configs[1..]
        .iter()
        .map(|c| (build_client(c), c.model.clone()))
        .collect();

    // 6. 定义 System Prompt
    let system_prompt = "你是一个专精于 A 股深研的金融量化 Agent。
你的任务是通过调用提供的工具（Tool）来获取真实数据，严禁编造财务数据。
在拿到数据后，你需要做交叉分析。如果在分析中触发了系统校验失败，请立刻通过报错提示进行修正，不要固执己见。
请根据个股数据和行业板块联动，给出最终深入的评估。".to_string();

    let stock_list_env = env::var("STOCK_LIST").unwrap_or_else(|_| "000338".to_string());
    let stock_codes: Vec<&str> = stock_list_env.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();

    println!(">>> 从环境变量 STOCK_LIST 获取到 {} 只待评股票: {:?}", stock_codes.len(), stock_codes);

    for code in stock_codes {
        // 4. 组装工具箱 (Toolbelt)
        let mut toolbelt = Toolbelt::new();
        toolbelt.register(FetchFinancialTool::new());
        toolbelt.register(FetchSectorTool::new());
        toolbelt.register(FetchResearchTool::new());
        toolbelt.register(FetchFundFlowTool::new());
        toolbelt.register(FetchChipDistributionTool::new());
        toolbelt.register(FetchNewsTool::new());

        // 5. 组装自我校验器 (A股特色 Validation)
        let validation = ValidationEngine::new_with_defaults();

        // 每次分析创建新的 Agent 实例，避免 Session ID 混淆
        let mut agent = AgentRunner::new(primary_client.clone(), toolbelt, validation, system_prompt.clone(), primary_model_name.clone())
            .with_fallbacks(fallback_pairs.clone());

        let query = format!("请帮我重点看看股票 {} 的财报，以及它最近的板块联动效应。要求内容详实。", code);
        println!("\n========================================================");
        println!(">>> 给 Agent 派发任务: {}", query);
        println!(">>> Agent 正在思考和收集数据 (ReAct 循环开始)，这可能需要数十秒...\n");

        match agent.run(&query, 12).await {
            Ok(res) => {
                println!("-------------------------------------");
                println!("     {} 最终报告产出     ", code);
                println!("-------------------------------------");
                println!("{}\n", res);

                // 将报告输出成md文档，不再使用邮件发送
                let _ = std::fs::create_dir_all("reports");
                let report_path = format!("reports/{}_agent_report.md", code);
                if let Err(e) = std::fs::write(&report_path, &res) {
                    log::error!("写入报告文件失败: {}", e);
                } else {
                    println!(">>> [报告生成] {} 的报告已保存至 {}", code, report_path);
                }
            }
            Err(e) => {
                println!("=====================================");
                println!("        Agent 执行中断 / 失败        ");
                println!("=====================================");
                println!("错误内容: {:?}", e);
            }
        }

        // 8. 打印落库的 Scratchpad 日志记录，验证第三阶段 (持久化)
        println!(">>> 从 SQLite 数据库读取 Agent 内部思考日志 (Scratchpad) for Session {}:", agent.session_id);
        let pool = DatabaseManager::get();
        if let Ok(mut conn) = pool.get_conn() {
            use diesel::prelude::*;
            let results = diesel::sql_query(format!(
                "SELECT step, log_type, content FROM agent_scratchpad WHERE session_id = '{}' ORDER BY id ASC", 
                agent.session_id
            )).load::<DbLogRow>(&mut conn);

            if let Ok(rows) = results {
                for r in rows {
                    println!("[Step {} | {}] {}", r.step, r.log_type, r.content);
                }
            }
        }
    }

    Ok(())
}

// 用于提取日志的临时结构体
use diesel::sql_types::{Integer, Text};
#[derive(Debug, diesel::QueryableByName)]
struct DbLogRow {
    #[diesel(sql_type = Integer)]
    step: i32,
    #[diesel(sql_type = Text)]
    log_type: String,
    #[diesel(sql_type = Text)]
    content: String,
}
