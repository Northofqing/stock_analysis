//! TDX 概念板块目录探针: 打印板块名, 用于 A-11 行业关键词匹配策略校准。
//! 用法: cargo run --bin board_directory_probe -- [关键词过滤, 可空]

use anyhow::{bail, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let kind = match std::env::args().nth(1).as_deref() { Some("industry") => stock_analysis::data_gateway::BoardKind::Industry, Some("region") => stock_analysis::data_gateway::BoardKind::Region, _ => stock_analysis::data_gateway::BoardKind::Concept }; let filter = std::env::args().nth(2);
    // BR-159 审计依赖数据库
    stock_analysis::database::DatabaseManager::init(Some(
        std::path::PathBuf::from("data/stock_analysis.db"),
    ))
    .map_err(|error| anyhow::anyhow!("数据库初始化失败: {error}"))?;
    let batch = stock_analysis::data_gateway::BoardDataGateway::production_tdx()
        .directory(kind, 200)
        .await?;
    let records = match &batch {
        stock_analysis::data_gateway::GatewayBatch::Available { records, .. } => records,
        other => bail!("不可用: {other:?}"),
    };
    println!("概念板块总数: {}", records.len());
    let mut shown = 0;
    for r in records {
        let name = &r.name;
        let matched = filter
            .as_ref()
            .map(|f| name.contains(f.as_str()))
            .unwrap_or(true);
        if matched {
            println!("{} ({}只)", name, r.member_count);
            shown += 1;
        }
    }
    println!("命中 {} 个板块", shown);
    Ok(())
}
