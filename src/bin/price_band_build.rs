//! 手工触发 op 67 价格带落库（P1.3 运维入口）.
//!
//! 用法:
//!   STOCK_DB=<path> cargo run --release --bin price_band_build -- --date 2026-10-16
//!   STOCK_DB=<path> cargo run --release --bin price_band_build          # 默认今天
//!
//! 为什么需要它: 生产调度只在**交易日 15:50** 触发一次。要当天验证 op 67 的
//! 真实响应、或补一个漏掉的快照日，必须有手工入口。
//!
//! 写入是幂等的（`ON CONFLICT (code, trade_date) DO UPDATE`），重复跑不会产生脏行。
//!
//! 注意: 对生产大库启动会触发 BR-159 全量审计链验证（十几分钟、CPU 不高、
//! 看起来像卡死）——不是死锁，别 kill。

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::trading_fact_snapshot::refresh_daily_snapshots;

fn arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .map(|value| value.as_str())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let date = match arg_value(&args, "--date") {
        Some(raw) => chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .map_err(|error| format!("--date 非法 ({raw}): {error}"))?,
        None => chrono::Local::now().date_naive(),
    };
    let date_text = date.format("%Y-%m-%d").to_string();

    // 必须显式指定库：init_retained_monitor(None) 的默认路径是 data/stock.db（不是生产主库），
    // 会让 CLI 打印"persisted=N"而主库表里查不到 —— 2026-10-17 实测踩过。
    let path = match std::env::var("STOCK_DB") {
        Ok(value) => std::path::PathBuf::from(value),
        Err(_) => {
            eprintln!(
                "[价格带] 必须设置 STOCK_DB（例如 STOCK_DB=data/stock_analysis.db）；\
                 不设会落到 data/stock.db，写完在主库查不到。"
            );
            std::process::exit(2);
        }
    };

    // 独立 CLI 不继承 launchd 注入的环境。未显式设置时回退到运行根的标准 bundle 目录，
    // 否则 ExternalV1 查询会以 external_bundle_unconfigured / external_bundle_invalid 失败
    // （2026-10-17 实测：既要设置，又必须是绝对路径）。
    if std::env::var_os("GRPC_MARKET_CLIENT_BUNDLE").is_none() {
        let bundle = std::env::current_dir()?.join("client-bundle");
        if bundle.join("connection.json").exists() {
            std::env::set_var("GRPC_MARKET_CLIENT_BUNDLE", &bundle);
            println!(
                "[价格带] 未设置 GRPC_MARKET_CLIENT_BUNDLE，回退到 {}",
                bundle.display()
            );
        } else {
            eprintln!(
                "[价格带] 警告: 未设置 GRPC_MARKET_CLIENT_BUNDLE 且 {} 不存在，\
                 op 67 查询会以 external_bundle_unconfigured 失败",
                bundle.display()
            );
        }
    }

    DatabaseManager::init_retained_monitor(path)?;
    let db = DatabaseManager::get();

    println!("[价格带] date={date_text} 开始（标的 = active tickets + 持仓）");
    let report = match refresh_daily_snapshots(db, &date_text).await {
        Ok(report) => report,
        Err(error) => {
            eprintln!("[价格带] 失败: {error}");
            std::process::exit(1);
        }
    };

    println!(
        "\n汇总: targets={} queried={} persisted={} without_price_band={}",
        report.targets, report.queried, report.persisted, report.without_price_band
    );
    if report.without_price_band > 0 {
        println!(
            "  （without_price_band>0 = 该日不是观测快照日，limit_up/down 按 BR-222 存 NULL）"
        );
    }
    for error in &report.errors {
        println!("  ! {error}");
    }
    Ok(())
}
