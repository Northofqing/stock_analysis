//! gRPC mock 服务端 (合同 grpc/grpc-external-api.md, 方案 A 委托 data_gateway)。
//! 默认 127.0.0.1:18082; GRPC_MARKET_PORT / GRPC_GATEWAY_TEST_FIXTURE / GRPC_EVENTS_SHADOW 可配。
//! 只读数据服务 + TDX 异动事件订阅。无账户/持仓/委托写接口。

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let config = stock_analysis::grpc_server::ServerConfig::default();
    let (addr, handle) = stock_analysis::grpc_server::start(config).await?;
    log::info!("[grpc_market_server] 就绪: {addr} (Ctrl-C 退出)");
    tokio::select! {
        r = handle => r??,
        _ = tokio::signal::ctrl_c() => log::info!("[grpc_market_server] 收到 Ctrl-C, 退出"),
    }
    Ok(())
}
