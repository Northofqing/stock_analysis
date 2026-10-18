//! 票级复盘 CLI（2026-10-18）。
//!
//! 按票聚合 `paper_trades` 历史，算已实现盈亏、成交率、未平数量。
//! 口径是净额法（卖出额 − 买入额），不是逐笔配对 —— 未平仓时数字不完整，
//! 输出会显式提示。
//!
//! 用法：
//!   STOCK_DB=data/stock_analysis.db code_review_show
//!   STOCK_DB=... code_review_show --top 20
//!   STOCK_DB=... code_review_show --code 600396

use std::env;

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::code_review::build_code_reviews;

fn main() {
    let db_path = env::var("STOCK_DB").unwrap_or_else(|_| {
        eprintln!("[复盘] 必须设置 STOCK_DB（例如 STOCK_DB=data/stock_analysis.db）");
        std::process::exit(2);
    });

    let args: Vec<String> = env::args().skip(1).collect();
    let mut top: Option<usize> = None;
    let mut only_code: Option<String> = None;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--top" => {
                top = args.get(index + 1).and_then(|value| value.parse().ok());
                index += 2;
            }
            "--code" => {
                only_code = args.get(index + 1).cloned();
                index += 2;
            }
            other => {
                eprintln!("[复盘] 未知参数 {other}");
                std::process::exit(2);
            }
        }
    }

    DatabaseManager::init_retained_monitor(Some(std::path::PathBuf::from(&db_path)))
        .unwrap_or_else(|error| {
            eprintln!("[复盘] 数据库初始化失败: {error}");
            std::process::exit(1);
        });

    let db = DatabaseManager::try_get().unwrap_or_else(|| {
        eprintln!("[复盘] 数据库不可用");
        std::process::exit(1);
    });

    let mut reviews = build_code_reviews(&db).unwrap_or_else(|error| {
        eprintln!("[复盘] 查询失败: {error}");
        std::process::exit(1);
    });

    if let Some(target) = &only_code {
        reviews.retain(|review| &review.code == target);
    }

    // 先算全库统计再截断 —— 否则 --top 会把"标的数"一起截掉，
    // 看起来像"全库只有 10 只票"，这是实测发现的误导。
    let total_codes = reviews.len();
    let total_trades: i64 = reviews
        .iter()
        .map(|review| review.buy_count + review.sell_count)
        .sum();

    reviews.sort_by(|left, right| {
        right
            .realized_pnl()
            .partial_cmp(&left.realized_pnl())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if let Some(limit) = top {
        reviews.truncate(limit);
    }

    println!(
        "[复盘] 全库标的={total_codes} 成交笔数={total_trades}，本次显示 {}",
        reviews.len()
    );
    for review in &reviews {
        println!("\n{}", review.render());
    }
}
