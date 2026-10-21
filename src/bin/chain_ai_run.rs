//! F10 取数 + AI 产业链分析（2026-10-18）。
//!
//! 用法:
//!   STOCK_DB=data/stock_analysis.db chain_ai_run 600396
//!   STOCK_DB=... chain_ai_run 600396 --skip-fetch   # 只跑 AI，不重新拉 F10
//!
//! 输出的是**候选**，全部标 [AI推断·未核实]，不参与任何买卖判断。

use std::env;

use stock_analysis::database::DatabaseManager;
use stock_analysis::llm::LlmRegistry;

#[tokio::main]
async fn main() {
    let db_path = env::var("STOCK_DB").unwrap_or_else(|_| {
        eprintln!("[链] 必须设置 STOCK_DB（例如 STOCK_DB=data/stock_analysis.db）");
        std::process::exit(2);
    });

    let args: Vec<String> = env::args().skip(1).collect();
    let Some(code) = args.first().cloned() else {
        eprintln!("[链] 用法: chain_ai_run <code> [--skip-fetch]");
        std::process::exit(2);
    };
    if code.len() != 6 || !code.chars().all(|ch| ch.is_ascii_digit()) {
        eprintln!("[链] 代码必须是 6 位数字，收到 {code}");
        std::process::exit(2);
    }
    let skip_fetch = args.iter().any(|arg| arg == "--skip-fetch");

    DatabaseManager::init_retained_monitor(Some(std::path::PathBuf::from(&db_path)))
        .unwrap_or_else(|error| {
            eprintln!("[链] 数据库初始化失败: {error}");
            std::process::exit(1);
        });
    let db = DatabaseManager::try_get().unwrap_or_else(|| {
        eprintln!("[链] 数据库不可用");
        std::process::exit(1);
    });

    // 1. F10 取数
    if !skip_fetch {
        println!("[链] 拉 F10 四类（经营分析/行业分析/最新提示/股本结构）…");
        match stock_analysis::monitor::f10_profile::refresh_f10(&db, std::slice::from_ref(&code)) {
            Ok(report) => report.log(),
            Err(error) => {
                eprintln!("[链] F10 取数失败: {error}");
                std::process::exit(1);
            }
        }
    }

    // 2. AI 分析
    let registry = LlmRegistry::from_env();
    let Some(provider) = registry.select("chain_analysis") else {
        eprintln!("[链] 没有可用的 LLM provider（检查 DEEPSEEK_API_KEY / LLM_DEFAULT_FALLBACK）");
        std::process::exit(1);
    };
    println!(
        "[链] provider={} model={}",
        provider.name(),
        provider.model()
    );

    match stock_analysis::monitor::chain_ai::analyze(provider, &db, &code).await {
        Ok(report) => {
            report.log();
        }
        Err(error) => {
            eprintln!("[链] AI 分析失败: {error}");
            std::process::exit(1);
        }
    }

    // 3. 打印候选
    let mut conn = match db.get_conn() {
        Ok(conn) => conn,
        Err(error) => {
            eprintln!("[链] 读库失败: {error}");
            std::process::exit(1);
        }
    };
    match stock_analysis::monitor::chain_ai::load_candidates(&mut conn, &code) {
        Ok(rows) if rows.is_empty() => {
            println!("\n[链] {code} 没有候选关系");
        }
        Ok(rows) => {
            println!(
                "\n[链] {code} 候选关系 {} 条（全部 [AI推断·未核实]）：",
                rows.len()
            );
            for (object_name, relation, evidence, confidence, _) in rows {
                let relation_cn = match relation.as_str() {
                    "supplier" => "供应商",
                    "customer" => "客户",
                    "competitor" => "竞争者",
                    other => other,
                };
                println!("  [{confidence:.2}] {relation_cn}  {object_name}");
                println!("        依据：{evidence}");
            }
        }
        Err(error) => eprintln!("[链] 读候选失败: {error}"),
    }
}
