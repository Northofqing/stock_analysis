//! 手工补齐日线 + 均线（P2 数据基础运维入口）.
//!
//! 用法:
//!   STOCK_DB=<path> cargo run --release --bin daily_bars_build
//!   STOCK_DB=<path> cargo run --release --bin daily_bars_build -- --date 2026-10-16
//!
//! 生产调度在**交易日 15:55** 自动跑一次（monitor 内）。要当天验证
//! HistoricalBars 能否拿到足够历史、或补一天漏掉的均线，用这个入口。
//!
//! 写入是幂等的（`ON CONFLICT (code, date) DO UPDATE`），重复跑不会产生脏行。
//!
//! 注意: 对生产大库启动会触发 BR-159 全量审计链验证（约 10 分钟、CPU 不高、
//! 看起来像卡死）——不是死锁，别 kill。

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::daily_bars_indicators::refresh_daily_bars_and_ma;

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

    // 与 price_band_build 同一约束：init_retained_monitor(None) 默认落 data/stock.db。
    let path = match std::env::var("STOCK_DB") {
        Ok(value) => std::path::PathBuf::from(value),
        Err(_) => {
            eprintln!(
                "[日线均线] 必须设置 STOCK_DB（例如 STOCK_DB=data/stock_analysis.db）；\
                 不设会落到 data/stock.db，写完在主库查不到。"
            );
            std::process::exit(2);
        }
    };

    // 独立 CLI 不继承 launchd 注入的 bundle 环境，且必须是绝对路径。
    if std::env::var_os("GRPC_MARKET_CLIENT_BUNDLE").is_none() {
        let bundle = std::env::current_dir()?.join("client-bundle");
        if bundle.join("connection.json").exists() {
            std::env::set_var("GRPC_MARKET_CLIENT_BUNDLE", &bundle);
            println!(
                "[日线均线] 未设置 GRPC_MARKET_CLIENT_BUNDLE，回退到 {}",
                bundle.display()
            );
        } else {
            eprintln!(
                "[日线均线] 警告: 未设置 GRPC_MARKET_CLIENT_BUNDLE 且 {} 不存在，\
                 HistoricalBars 查询会失败",
                bundle.display()
            );
        }
    }

    DatabaseManager::init_retained_monitor(Some(path))?;
    let db = DatabaseManager::get();

    println!("[日线均线] as_of={date_text} 开始（标的 = active tickets + 持仓）");
    let report = match refresh_daily_bars_and_ma(db, &date_text).await {
        Ok(report) => report,
        Err(error) => {
            eprintln!("[日线均线] 失败: {error}");
            std::process::exit(1);
        }
    };

    println!(
        "\n汇总: targets={} fetched={} persisted_bars={} with_ma20={}",
        report.targets, report.fetched, report.persisted_bars, report.with_ma20
    );
    if report.fetched > 0 && report.with_ma20 == 0 {
        println!(
            "  （with_ma20=0 = 拉到的历史不足 20 个交易日，ma20 全 NULL，P2 仍不可做）"
        );
    }
    for error in &report.errors {
        println!("  ! {error}");
    }
    Ok(())
}
