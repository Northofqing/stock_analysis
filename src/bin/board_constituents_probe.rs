//! BoardConstituents 探针（2026-10-19 修订）。
//!
//! 关键修正：Windows 侧 `TdxBoardProvider::board_constituents` 收的是
//! `BoardConstituentRequest { board_code, limit }`，按板块过滤后返回**成分股**。
//! 之前一直传 `codes`，方向用反，拿到的其实是 code→board 的反向归属。
//!
//!     GRPC_MARKET_ADDR=http://127.0.0.1:18082 board_constituents_probe --board tdx:industry:融资融券
//!     ... board_constituents_probe --board tdx:concept:半导体 --limit 30
//!     ... board_constituents_probe --code 600396      # 旧口径对照
//!
//! 不落库，只打印原始结构。

use stock_analysis::data_gateway::grpc_source::bridge_for;
use stock_analysis::data_gateway::GatewayBatch;

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let limit: u32 = arg_value(&args, "--limit")
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(20);

    let bridge = match bridge_for("BoardConstituents") {
        Ok(bridge) => bridge,
        Err(error) => {
            eprintln!("[BC] bridge 不可用: {error}");
            std::process::exit(1);
        }
    };

    // 旧口径对照
    if let Some(code) = arg_value(&args, "--code") {
        println!("[BC] 旧口径 codes=[{code}] （应当只回该票的板块归属）");
        match bridge.board_constituents_async(&code).await {
            Ok(GatewayBatch::Available { records, evidence }) => {
                println!("  {} 条 (batch={})", records.len(), evidence.batch_id);
                for record in records.iter().take(limit as usize) {
                    println!(
                        "    board={} {} kind={:?} instrument={}",
                        record.board_code,
                        record.board_name,
                        record.kind,
                        record.instrument_code
                    );
                }
            }
            other => println!("  {other:?}"),
        }
        return Ok(());
    }

    let boards: Vec<String> = match arg_value(&args, "--board") {
        Some(raw) => raw.split(',').map(|item| item.trim().to_owned()).collect(),
        None => vec![
            "tdx:industry:融资融券".to_owned(),
            "tdx:concept:半导体".to_owned(),
        ],
    };

    for board in &boards {
        println!("\n[BC] === board_code={board} limit={limit} ===");
        match bridge.board_constituents_by_board_async(board, limit).await {
            Ok(GatewayBatch::Available { records, evidence }) => {
                println!("  可用, {} 条 (batch={})", records.len(), evidence.batch_id);
                let mut instruments: Vec<&str> = Vec::new();
                for record in &records {
                    if !instruments.contains(&record.instrument_code.as_str()) {
                        instruments.push(&record.instrument_code);
                    }
                }
                println!(
                    "  去重标的 {} 只: {:?}",
                    instruments.len(),
                    &instruments[..instruments.len().min(15)]
                );
                for record in records.iter().take(8) {
                    println!(
                        "    {} <- {} ({}) kind={:?}",
                        record.instrument_code,
                        record.board_name,
                        record.board_code,
                        record.kind
                    );
                }
            }
            Ok(GatewayBatch::VerifiedEmpty(evidence)) => {
                println!("  已验证为空 (batch={})", evidence.batch_id);
            }
            Err(error) => {
                println!("  失败: {error}");
                println!("  Debug: {error:?}");
            }
        }
    }

    Ok(())
}
