//! BoardFlows（op 36）验证探针。
//!
//! Windows 侧 2026-10-18 回复指出：D1（主力净流入 f62）和 D2（涨幅榜 f3）
//! 不应该走 `MarketRankings`（那个 op 的结构只放行 VolumeRatio / MainNetInflow，
//! 拿它请求板块榜会被能力闸门拒掉），正确入口是 `BoardFlows`，
//! 一次调用同时返回：
//!
//!   return_pct      → 涨幅（原 f3）
//!   main_net_yuan   → 主力净流入（原 f62）
//!
//! 用法：
//!   GRPC_MARKET_ADDR=http://127.0.0.1:18082 board_flows_probe

use std::env;
use std::path::PathBuf;

use stock_analysis::data_gateway::board::{BoardDataGateway, BoardKind};
use stock_analysis::database::DatabaseManager;

fn main() {
    // BR-159 要求每次采集都写审计，审计要数据库；不初始化的话
    // provider 明明 accepted 了也会被报成 acquisition_audit_unavailable。
    let db_path = env::var("STOCK_DB").unwrap_or_else(|_| {
        eprintln!("[BoardFlows] 必须设置 STOCK_DB（例如 STOCK_DB=<运行根>/data/stock_analysis.db）");
        std::process::exit(2);
    });
    DatabaseManager::init_retained_monitor(Some(PathBuf::from(&db_path))).unwrap_or_else(|error| {
        eprintln!("[BoardFlows] 数据库初始化失败: {error}");
        std::process::exit(1);
    });

    let gateway = BoardDataGateway::default();

    for (kind, label) in [
        (BoardKind::Industry, "行业"),
        (BoardKind::Concept, "概念"),
        (BoardKind::Region, "地域"),
    ] {
        println!("\n[BoardFlows] === {label} ({kind:?}) ===");
        match gateway.day1_flows_blocking(kind, 5) {
            Ok(batch) => {
                let rows = batch.records();
                if rows.is_empty() {
                    println!("  结果：空批次（op 通了但没有数据）");
                    continue;
                }
                println!("  结果：成功，{} 条", rows.len());
                for (i, r) in rows.iter().enumerate() {
                    let inflow = r
                        .main_net_yuan
                        .map(|v| format!("{:.2} 亿", v / 1e8))
                        .unwrap_or_else(|| "NULL".to_string());
                    let pct = r
                        .return_pct
                        .map(|v| format!("{v:+.2}%"))
                        .unwrap_or_else(|| "NULL".to_string());
                    println!(
                        "  {}. {:<10} {:<12} 涨幅 {:>8}  主力净流入 {:>12}  领涨 {} {}",
                        i + 1,
                        r.code,
                        r.name,
                        pct,
                        inflow,
                        r.leader_code.as_deref().unwrap_or("-"),
                        r.leader_name.as_deref().unwrap_or("-"),
                    );
                }
            }
            Err(error) => {
                println!("  结果：失败");
                println!("  错误：{error}");
                println!("  判定：{error:#}");
            }
        }
    }
}
