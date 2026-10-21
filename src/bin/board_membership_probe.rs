//! BoardMemberships（op 29）+ preferred_provider=Eastmoney 探测。
//!
//! Windows release e2319f5 给这个 op 加了第二个 provider：
//!   {"codes":[...],"preferred_provider":"Eastmoney"}
//!   → board_code = eastmoney:industry:<名称> / eastmoney:concept:<名称> / eastmoney:region:<名称>
//!     board_name = 东财个股页原始名称
//! 与 BoardFlows 的 board_name 同源，可精确 join，不再依赖 stock_concepts 的
//! 「AIGC概念 vs AIGC」这种模糊匹配。
//!
//! 本探针只打印原始结构，不落库 —— 先确认每个 CanonicalRecord 是一只票还是
//! 一个板块归属，再决定表结构。
//!
//! 用法：
//!   GRPC_MARKET_CLIENT_BUNDLE=<绝对路径>/connection.json \
//!   board_membership_probe 600519 000338 600396

use std::env;
use std::path::Path;

use anyhow::{Context, Result};

use stock_analysis::grpc_client::client::GrpcMarketClient;
// ExternalV1 的 op 枚举在 external_pb 里，和 LocalBridge 的 pb 是两套
// （序号会撞车，所以 query() 只接受 LocalBridge 的 Operation）。
// 走 bundle 必须用 query_external_native + ExternalOperation。
use stock_analysis::grpc_client::external_pb::magic::market::v1::Operation as ExternalOperation;

#[tokio::main]
async fn main() -> Result<()> {
    let bundle = env::var("GRPC_MARKET_CLIENT_BUNDLE")
        .context("必须设置 GRPC_MARKET_CLIENT_BUNDLE（绝对路径）")?;
    let codes: Vec<String> = env::args().skip(1).collect();
    if codes.is_empty() {
        anyhow::bail!("用法: board_membership_probe <code> [code...]（1..=50 只沪深 A 股）");
    }

    println!(
        "[成员] 请求 {} 只: {:?}  provider=Eastmoney",
        codes.len(),
        codes
    );

    let mut client = GrpcMarketClient::connect_client_bundle(Path::new(&bundle))
        .await
        .map_err(|error| anyhow::anyhow!("连接 bundle 失败: {error}"))?;

    let payload = serde_json::json!({
        "codes": codes,
        "preferred_provider": "Eastmoney",
    });

    let result = match client
        .query_external_native(ExternalOperation::BoardMemberships, payload)
        .await
    {
        Ok(result) => result,
        Err(error) => {
            println!("[成员] 失败 Display : {error}");
            println!("[成员] 失败 因果链  : {error:#}");
            println!("[成员] 失败 Debug   : {error:?}");
            anyhow::bail!("BoardMemberships 调用失败");
        }
    };

    println!(
        "[成员] batch_id={} provider={} complete={} records={} source_at={:?}",
        result.batch_id,
        result.selected_provider,
        result.complete,
        result.records.len(),
        result.source_at,
    );
    println!("[成员] observed_at={}", result.observed_at);

    for (index, record) in result.records.iter().enumerate() {
        println!(
            "\n--- record {} schema={} v{} bytes={} ---",
            index,
            record.schema,
            record.schema_version,
            record.data.len()
        );
        match serde_json::from_slice::<serde_json::Value>(&record.data) {
            Ok(value) => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                );
            }
            Err(error) => {
                println!("不是 JSON: {error}");
                println!(
                    "前 200 字节: {:?}",
                    &record.data[..record.data.len().min(200)]
                );
            }
        }
    }

    Ok(())
}
