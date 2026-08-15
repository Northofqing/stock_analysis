//! 集成测试: 真起 grpc_server (fixture 模式, 随机端口) → GrpcMarketClient 调用。
//! 离线确定性, 不连真实网络。
use stock_analysis::grpc_client::client::GrpcMarketClient;
use stock_analysis::grpc_client::pb::magic::market::v1::Operation;
use stock_analysis::grpc_server::{start, ServerConfig};

#[tokio::test(flavor = "multi_thread")]
async fn health_and_capabilities() {
    let (addr, handle) = start(ServerConfig {
        fixture_mode: true,
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let addr = format!("http://{addr}");
    let mut client = GrpcMarketClient::connect(&addr).await.unwrap();
    let health = client.get_health().await.unwrap();
    assert!(health.live && health.ready);
    let caps = client.get_capabilities().await.unwrap();
    assert_eq!(caps.len(), 24, "24 个生产 op 全部在 capability 表");
    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn six_representative_ops_fixture_roundtrip() {
    let (addr, handle) = start(ServerConfig {
        fixture_mode: true,
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let mut client = GrpcMarketClient::connect(&format!("http://{addr}")).await.unwrap();

    let cases: Vec<(Operation, &str, &str)> = vec![
        (Operation::RealtimeQuotes, "market.realtime_quotes", "600519"),
        (Operation::HistoricalBars, "market.historical_bars", "600519"),
        (Operation::MinuteData, "market.minute_data", "600519"),
        (Operation::Announcements, "news.announcements", "600519"),
        (Operation::GlobalNews, "news.global_news", "央行"),
        (Operation::SecurityMetadata, "market.security_metadata", "600519"),
    ];
    for (op, schema, probe) in cases {
        let result = client
            .query(op, serde_json::json!({}))
            .await
            .unwrap_or_else(|e| panic!("{schema} 查询失败: {e}"));
        assert!(result.complete, "{schema} complete=true");
        assert_eq!(result.records.len(), 1, "{schema} 1 条 fixture 记录");
        assert_eq!(result.records[0].schema, schema);
        let parsed: serde_json::Value = serde_json::from_slice(&result.records[0].data).unwrap();
        assert!(parsed[0].to_string().contains(probe), "{schema} 内容含 {probe}");
    }
    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_schema_rejected() {
    let (addr, handle) = start(ServerConfig {
        fixture_mode: true,
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let mut client = GrpcMarketClient::connect(&format!("http://{addr}")).await.unwrap();
    // OptionData 未实现 → 客户端拦截。
    let err = client
        .query(Operation::OptionData, serde_json::json!({}))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        stock_analysis::grpc_client::errors::GrpcError::Unimplemented
    ));
    handle.abort();
}
