//! 从 Windows 持仓桥同步一次实际持仓到本地快照。
//!
//!     STOCK_DB=<path> holdings_bridge_sync
//!     STOCK_DB=<path> holdings_bridge_sync --url http://10.211.55.3:18090 --dry-run
//!
//! 数据源是 TDX「资金股份」导出文件（不是券商 API）。桥只读、不刷新，
//! 所以 `snapshot_at` 是导出时刻，`age_seconds` 太大时会拒绝写入。
//!
//! 写入路径与 `import_user_position_snapshot` 完全一致：
//! save_user_position_snapshot → reconcile_stock_position_from_confirmed_snapshot
//! → trade_ledger::record_snapshot_buys。

use std::path::PathBuf;

use stock_analysis::database::{self, DatabaseManager};
use stock_analysis::monitor::holdings_bridge;
use stock_analysis::portfolio::user_position_snapshot::user_position_snapshot_input_from_json;

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().collect();
    let dry_run = args.iter().any(|arg| arg == "--dry-run");

    let url = arg_value(&args, "--url")
        .or_else(|| std::env::var("HOLDINGS_BRIDGE_URL").ok())
        .unwrap_or_else(|| "http://10.211.55.3:18090".to_string());

    let token = match std::env::var("HOLDINGS_BRIDGE_TOKEN") {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => {
            let path = arg_value(&args, "--token-file").unwrap_or_else(|| {
                "client-bundle/holdings-bridge-token.txt".to_string()
            });
            std::fs::read_to_string(&path)
                .map_err(|error| format!("读取 token 文件 {path} 失败: {error}"))?
                .trim()
                .to_string()
        }
    };
    if token.is_empty() {
        return Err("token 为空".into());
    }

    let holdings = holdings_bridge::fetch(&url, &token)?;
    println!(
        "桥返回: 快照 {}  年龄 {:.0}s  stale={}  账户尾号 {}  持仓 {} 只",
        holdings.snapshot_at,
        holdings.age_seconds,
        holdings.stale,
        holdings.account_tail,
        holdings.positions.len()
    );
    for position in &holdings.positions {
        println!(
            "  {} {:<8} {:>6} 股 @ {:.3}{}",
            position.code,
            position.name,
            position.stock_quantity as u64,
            position.cost_price,
            if position.flags.is_empty() {
                String::new()
            } else {
                format!("  [{}]", position.flags.join(","))
            }
        );
    }

    if let Err(reason) = holdings_bridge::accept(&holdings) {
        // 显式构造：String → Box<dyn Error> 有多条 Into 实现，`.into()` 会歧义。
        return Err(Box::<dyn std::error::Error>::from(reason));
    }

    if dry_run {
        println!("--dry-run：未写库");
        return Ok(());
    }

    let json = holdings_bridge::to_snapshot_json(&holdings);
    let confirmed_at = chrono::Local::now().fixed_offset();
    let input = user_position_snapshot_input_from_json(&json, confirmed_at)?;

    let db_path = std::env::var("STOCK_DB").ok().map(PathBuf::from);
    DatabaseManager::init(db_path)?;
    let receipt = database::user_position_snapshot::save_user_position_snapshot(&input)?;
    println!(
        "快照写入: inserted={} item_count={}",
        receipt.inserted,
        input.items.len()
    );

    let reconciliation = database::reconcile_stock_position_from_confirmed_snapshot()?;
    println!(
        "投影对齐: snapshot_id={} updated={} inserted={} unchanged={} unconfirmed_open={:?}",
        reconciliation.snapshot_id,
        reconciliation.updated,
        reconciliation.inserted,
        reconciliation.unchanged,
        reconciliation.unconfirmed_open
    );

    let effective = input.effective_at.format("%Y-%m-%d %H:%M:%S").to_string();
    match stock_analysis::monitor::trade_ledger::record_snapshot_buys(
        DatabaseManager::get(),
        &effective,
    ) {
        Ok(report) => {
            report.log();
            println!(
                "交易流水: inferred_buys={} sells_needing_manual_price={} already_recorded={}",
                report.inferred_buys,
                report.sells_needing_manual_price.len(),
                report.already_recorded
            );
        }
        Err(error) => eprintln!("trade_ledger 推断失败（快照与投影已写入）: {error}"),
    }

    Ok(())
}
