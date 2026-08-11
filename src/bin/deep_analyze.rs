//! 个股深度 AI 研判 CLI — 传入 1+ 个股代码, 逐个跑 deep_analyzer 多角色研判。
//! 用法: cargo run --bin deep_analyze -- 600519 000858
//! 走生产同款 deep_analyzer::run_and_save (6 工具数据 + 多分析师 + 辩论仲裁),
//! 报告写入 reports/details/{date}_{code}.md。只读网络请求 + 写报告文件,
//! 无交易、无推送。

use stock_analysis::deep_analyzer;

fn main() {
    let codes: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    if codes.is_empty() {
        eprintln!("用法: cargo run --bin deep_analyze -- <code> [code...]");
        std::process::exit(2);
    }
    // .env: DOUBAO/DEEPSEEK/GEMINI_API_KEY 等模型凭据 + 数据网关配置。
    dotenvy::dotenv().ok();
    // BR-159: 数据网关 acquisition audit 需要 DatabaseManager; 用独立探针库,
    // 不写生产主库 (与 gateway_quote_probe 同模式)。
    let probe_db = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("deep_analyze_probe.db");
    std::env::set_var("DATABASE_PATH", &probe_db);
    stock_analysis::database::DatabaseManager::init(Some(probe_db))
        .expect("probe db init");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    for code in &codes {
        println!("== deep_analyze code={code} ==");
        match rt.block_on(deep_analyzer::run_and_save(code)) {
            Ok(path) => println!("REPORT OK: {}", path.display()),
            Err(error) => eprintln!("REPORT FAILED {code}: {error}"),
        }
    }
}
