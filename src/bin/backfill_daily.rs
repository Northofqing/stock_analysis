//! 一次性回填 stock_daily 数据 (R-3 修复)
//!
//! 用途: stock_daily 停更超过 1 个交易日时, 触发一次全量拉取 + 落盘。
//! 数据源: RustDX 通达信 (主) → GtimgProvider (备) → HttpProvider (备)。
//!
//! 用法:
//!   STOCK_DB=data/stock_analysis.db cargo run --bin backfill_daily
//!   STOCK_DB=data/stock_analysis.db cargo run --bin backfill_daily -- 000001,600519,002415
//!
//! 设计: 与 `backfill_predictions.rs` 保持一致风格 — 直接调用 lib 公共 API,
//!       不复用 monitor 的 pipeline (避免触发 dry-run 的全套分析)。

use std::env;
use std::path::PathBuf;
use stock_analysis::database::DatabaseManager;
use stock_analysis::data_provider::DataFetcherManager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 参数: STOCK_LIST 优先; 否则命令行第一个参数 (逗号分隔); 否则用监控自选.
    let stock_list_env = env::var("STOCK_LIST").ok();
    let arg1 = env::args().nth(1);

    let stock_codes: Vec<String> = match arg1.or(stock_list_env) {
        Some(s) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        None => {
            eprintln!("用法: STOCK_DB=... cargo run --bin backfill_daily -- 000001,600519");
            eprintln!("      或设环境变量 STOCK_LIST=000001,600519");
            std::process::exit(2);
        }
    };

    if stock_codes.is_empty() {
        eprintln!("[backfill_daily] 股票列表为空, 退出");
        std::process::exit(2);
    }

    // 2. 初始化 DB
    let db_path = env::var("STOCK_DB").ok().map(PathBuf::from);
    let _ = DatabaseManager::init(db_path.clone());
    let db = DatabaseManager::get();
    let source = "backfill_daily";

    // 3. 初始化数据获取器
    let fetcher = DataFetcherManager::new()?;

    // 4. 拉 90 天 K线 (保证能覆盖周末/节假日的滞后窗口)
    let days: usize = env::var("BACKFILL_DAYS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(90);

    let mut ok_count = 0usize;
    let mut fail_count = 0usize;

    for code in &stock_codes {
        match fetcher.get_daily_data(code, days) {
            Ok((data, _src)) if !data.is_empty() => {
                match db.save_kline_data(code, &data, source) {
                    Ok(n) => {
                        ok_count += 1;
                        println!("[backfill_daily] {} OK: 写入 {} 条 (latest={})",
                            code, n, data.first().map(|k| k.date.to_string()).unwrap_or_default());
                    }
                    Err(e) => {
                        fail_count += 1;
                        eprintln!("[backfill_daily] {} 写入失败: {}", code, e);
                    }
                }
            }
            Ok((_, _)) => {
                fail_count += 1;
                eprintln!("[backfill_daily] {} 数据为空", code);
            }
            Err(e) => {
                fail_count += 1;
                eprintln!("[backfill_daily] {} 拉取失败: {}", code, e);
            }
        }
    }

    println!("\n[backfill_daily] 完成: 成功 {} 只, 失败 {} 只, 共 {} 只",
        ok_count, fail_count, stock_codes.len());

    // 5. 验证 (用 sqlite3 直接查, 避免 async 嵌套)
    if let Some(path) = db_path.as_ref().or(Some(&PathBuf::from("data/stock_analysis.db"))) {
        if let Ok(output) = std::process::Command::new("sqlite3")
            .arg(path)
            .arg("SELECT MAX(date), COUNT(*) FROM stock_daily;")
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                println!("[backfill_daily] stock_daily MAX(date)|COUNT(*) = {}", stdout.trim());
            }
        }
    }

    Ok(())
}