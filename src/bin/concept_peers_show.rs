//! 概念共现 CLI（2026-10-18）。
//!
//! 给定一只票，列出与它共享概念最多的其他票。
//! 共现直接从 `stock_concepts`（1265 只票的 JSON 概念标签）现算，
//! 不用那张空的 `concept_cooccurrence` 表。
//!
//! 用法：
//!   STOCK_DB=data/stock_analysis.db concept_peers_show 601811
//!   STOCK_DB=... concept_peers_show 601811 --min 3 --limit 20

use std::env;

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::concept_peers::{peers_of, DEFAULT_MIN_SHARED};

fn main() {
    let db_path = env::var("STOCK_DB").unwrap_or_else(|_| {
        eprintln!("[概念共现] 必须设置 STOCK_DB（例如 STOCK_DB=data/stock_analysis.db）");
        std::process::exit(2);
    });

    let args: Vec<String> = env::args().skip(1).collect();
    let Some(code) = args.first().cloned() else {
        eprintln!("[概念共现] 用法: concept_peers_show <code> [--min N] [--limit N]");
        std::process::exit(2);
    };

    let mut min_shared = DEFAULT_MIN_SHARED;
    let mut limit = 20usize;
    let mut index = 1usize;
    while index < args.len() {
        match args[index].as_str() {
            "--min" => {
                if let Some(value) = args.get(index + 1).and_then(|v| v.parse().ok()) {
                    min_shared = value;
                }
                index += 2;
            }
            "--limit" => {
                if let Some(value) = args.get(index + 1).and_then(|v| v.parse().ok()) {
                    limit = value;
                }
                index += 2;
            }
            other => {
                eprintln!("[概念共现] 未知参数 {other}");
                std::process::exit(2);
            }
        }
    }

    DatabaseManager::init_retained_monitor(Some(std::path::PathBuf::from(&db_path)))
        .unwrap_or_else(|error| {
            eprintln!("[概念共现] 数据库初始化失败: {error}");
            std::process::exit(1);
        });

    let db = DatabaseManager::try_get().unwrap_or_else(|| {
        eprintln!("[概念共现] 数据库不可用");
        std::process::exit(1);
    });

    let peers = peers_of(&db, &code, min_shared).unwrap_or_else(|error| {
        eprintln!("[概念共现] 查询失败: {error}");
        std::process::exit(1);
    });

    println!(
        "[概念共现] {code} 共享概念 >= {min_shared} 的票数 = {}（显示前 {limit}）",
        peers.len()
    );
    for peer in peers.iter().take(limit) {
        println!("  {}", peer.render());
    }
}
