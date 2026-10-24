//! ExternalV1（Windows gRPC）日线补缺工具。
//!
//! 本机桥 `HistoricalBars` 四路 provider 全挂时，用它从 Windows 侧的
//! ExternalV1 取日线。**只补缺，不覆盖**：目标 `(code,date)` 已存在则跳过，
//! 避免把 `tdx_day` 的成交额和来源标记冲掉（数据合同第 3 条第 4/5 款）。
//!
//! 用法：
//!   external_bars_import --codes 000001,600519 --date 2026-10-23 --limit 5
//!   external_bars_import --codes 000001 --limit 3 --dry-run
//!   external_bars_import --codes 000001 --db <path> --bundle <client-bundle>
//!
//! 退出码：0 全部成功；2 有失败（明细已打印）；3 参数错误。

use std::path::PathBuf;
use std::time::Duration;

use stock_analysis::data_gateway::external_daily_bars::{
    exchange_for_code, fetch_daily_bars_window, ExternalDailyBar,
};

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

/// 数据来源标签：必须如实反映实际 provider，不能一律写 tencent-web。
fn source_label_for(provider: &str) -> String {
    match provider {
        "Tonghuashun" | "HithinkFinance" => "hithink-web".to_owned(),
        "Tencent" => "tencent-web".to_owned(),
        other if other.is_empty() => "external-web".to_owned(),
        other => format!("{}-web", other.to_ascii_lowercase()),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let codes: Vec<String> = match arg_value(&args, "--codes") {
        Some(raw) => raw
            .split(',')
            .map(str::trim)
            .filter(|code| !code.is_empty())
            .map(str::to_owned)
            .collect(),
        None => {
            eprintln!("缺少 --codes（逗号分隔的 6 位代码）");
            std::process::exit(3);
        }
    };
    let limit: u32 = arg_value(&args, "--limit")
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(10);
    let target_date = arg_value(&args, "--date").unwrap_or_else(|| {
        eprintln!("缺少 --date（目标交易日 YYYY-MM-DD）");
        std::process::exit(3);
    });
    let target = match chrono::NaiveDate::parse_from_str(&target_date, "%Y-%m-%d") {
        Ok(date) => date,
        Err(error) => {
            eprintln!("--date 解析失败: {error}");
            std::process::exit(3);
        }
    };
    // 窗口天数：日期区间由服务端校验，太窄会被连续性检查拒绝。
    let window_days: i64 = arg_value(&args, "--window-days")
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(10);
    let start = target - chrono::Duration::days(window_days);
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let db_path = arg_value(&args, "--db")
        .map(PathBuf::from)
        .or_else(|| std::env::var("STOCK_DB").ok().map(PathBuf::from));
    let bundle = arg_value(&args, "--bundle")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("GRPC_MARKET_CLIENT_BUNDLE").map(PathBuf::from))
        .or_else(|| {
            let candidate = std::env::current_dir().ok()?.join("client-bundle");
            candidate.exists().then_some(candidate)
        });
    let bundle = match bundle {
        Some(path) => path,
        None => {
            eprintln!("找不到 client-bundle：请用 --bundle 或 GRPC_MARKET_CLIENT_BUNDLE");
            std::process::exit(3);
        }
    };

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("tokio runtime 失败: {error}");
            std::process::exit(3);
        }
    };

    let mut fetched = 0usize;
    let mut inserted = 0usize;
    let mut existing = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();
    let mut rows: Vec<ExternalDailyBar> = Vec::new();

    for code in &codes {
        let Some(exchange) = exchange_for_code(code) else {
            failed.push((code.clone(), "ExternalV1 不覆盖（非沪深股票）".to_owned()));
            continue;
        };
        match runtime.block_on(fetch_daily_bars_window(
            &bundle, exchange, code, start, target, limit,
        )) {
            Ok(bars) => {
                for bar in bars {
                    if bar.date != target_date {
                        continue;
                    }
                    fetched += 1;
                    rows.push(bar);
                }
            }
            Err(error) => failed.push((code.clone(), error)),
        }
    }

    if dry_run {
        for bar in &rows {
            println!(
                "{} {} O={:.2} H={:.2} L={:.2} C={:.2} vol={:.0} amount={:?} provider={}",
                bar.code, bar.date, bar.open, bar.high, bar.low, bar.close, bar.volume, bar.amount, bar.provider
            );
        }
        println!("dry_run targets={} fetched={} failed={}", codes.len(), fetched, failed.len());
        for (code, error) in &failed {
            println!("  ! {code}: {error}");
        }
        return;
    }

    let db_path = match db_path {
        Some(path) => path,
        None => {
            eprintln!("缺少 --db 或 STOCK_DB");
            std::process::exit(3);
        }
    };
    let connection = match rusqlite::Connection::open(&db_path) {
        Ok(connection) => connection,
        Err(error) => {
            eprintln!("打开数据库失败: {error}");
            std::process::exit(3);
        }
    };
    let _ = connection.busy_timeout(Duration::from_secs(30));

    for bar in &rows {
        // 只补缺：已存在的 (code,date) 一律跳过，保留既有来源与成交额。
        // 来源标记必须如实反映实际 provider，不能一律写成 tencent-web。
        let source_label = source_label_for(&bar.provider);
        let changed = connection.execute(
            "INSERT INTO stock_daily (code, date, open, high, low, close, volume, amount, data_source) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT(code, date) DO NOTHING",
            rusqlite::params![
                bar.code,
                bar.date,
                bar.open,
                bar.high,
                bar.low,
                bar.close,
                bar.volume,
                bar.amount,
                source_label,
            ],
        );
        match changed {
            Ok(1) => inserted += 1,
            Ok(_) => existing += 1,
            Err(error) => failed.push((format!("{} {}", bar.code, bar.date), error.to_string())),
        }
    }

    println!(
        "done targets={} fetched={} inserted={} existing={} failed={}",
        codes.len(),
        fetched,
        inserted,
        existing,
        failed.len()
    );
    for (code, error) in &failed {
        println!("  ! {code}: {error}");
    }
    if !failed.is_empty() {
        std::process::exit(2);
    }
}
