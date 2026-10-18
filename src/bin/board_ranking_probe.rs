//! 板块榜 op 验证探针（2026-10-18）。
//!
//! 验证 `board_ranking_async` 走的两个 op 到底能不能调通：
//!
//!   f3  → Operation::ConceptHits     涨幅榜
//!   f62 → Operation::MarketRankings  主力净流入榜
//!
//! 三种结果对应三种修法：
//!   a) 返回数据         → 能直接用，接上 15:30 调度即可
//!   b) 返回格式/解析错   → 我这边改解析
//!   c) unimplemented    → 要 Windows 侧开发
//!
//! 用法:
//!   STOCK_DB=... cargo run --release --bin board_ranking_probe

use stock_analysis::data_gateway::board_ranking::BoardRankingGateway;

fn main() {
    let gateway = BoardRankingGateway::new();

    for fid in ["f3", "f62"] {
        let label = match fid {
            "f3" => "涨幅榜 ConceptHits",
            _ => "净流入榜 MarketRankings",
        };
        println!("\n[板块榜] === {fid} ({label}) ===");

        match gateway.fetch_top(fid, 5) {
            Ok(rows) if rows.is_empty() => {
                println!("  结果：空批次（op 通了但没有数据）");
            }
            Ok(rows) => {
                println!("  结果：成功，{} 条", rows.len());
                for (index, row) in rows.iter().enumerate() {
                    println!(
                        "  {}. {} {}  涨幅 {:+.2}%  主力净流入 {:.2}  领涨 {}  量比 {:.2}  换手 {:.2}%  当日占比 {:.2}%  5日占比 {:.2}%",
                        index + 1,
                        row.code,
                        row.name,
                        row.change_pct,
                        row.main_inflow,
                        row.leader_name,
                        row.vol_ratio,
                        row.turnover,
                        row.day1_ratio,
                        row.day5_ratio
                    );
                }
            }
            Err(error) => {
                println!("  结果：失败");
                println!("  错误：{error}");
                let text = error.to_string();
                if text.contains("unimplemented") || text.contains("UNIMPLEMENTED") {
                    println!("  → 判定：对端未实现，需要 Windows 侧开发");
                } else if text.contains("parse") || text.contains("decode") || text.contains("反序列化") {
                    println!("  → 判定：对端有数据但格式不符，我这边改解析");
                } else if text.contains("unavailable") || text.contains("transport") {
                    println!("  → 判定：传输层问题（桥未连 / 超时），不是 op 缺失");
                } else {
                    println!("  → 判定：需要看完整错误再定");
                }
            }
        }
    }
}
