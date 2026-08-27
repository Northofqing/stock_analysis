//! 集成测试: 真起 grpc_server (fixture 模式, 随机端口) → GrpcMarketClient 调用。
//! 离线确定性, 不连真实网络。
use stock_analysis::grpc_client::client::GrpcMarketClient;
use stock_analysis::grpc_client::pb::magic::market::v1::{EventCursor, EventFilter, Operation};
use stock_analysis::grpc_server::events::{DetectedEvent, EventHub, EventKind};
use stock_analysis::grpc_server::{start, ServerConfig};

#[tokio::test(flavor = "multi_thread")]
async fn health_and_capabilities() {
    let (addr, handle, _hub) = start(ServerConfig {
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
    assert_eq!(
        caps.len(),
        40,
        "M1: 38 个生产 op + M4c ChainBatch + BR-251 BenchmarkBars 全部在 capability 表"
    );
    assert!(caps
        .iter()
        .any(|capability| capability.operation == Operation::BenchmarkBars as i32));
    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn six_representative_ops_fixture_roundtrip() {
    let (addr, handle, _hub) = start(ServerConfig {
        fixture_mode: true,
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let mut client = GrpcMarketClient::connect(&format!("http://{addr}"))
        .await
        .unwrap();

    let cases: Vec<(Operation, &str, &str)> = vec![
        (
            Operation::RealtimeQuotes,
            "market.realtime_quotes",
            "600519",
        ),
        (
            Operation::HistoricalBars,
            "market.historical_bars",
            "600519",
        ),
        (Operation::MinuteData, "market.minute_data", "600519"),
        (Operation::Announcements, "news.announcements", "600519"),
        (Operation::GlobalNews, "news.global_news", "央行"),
        (
            Operation::SecurityMetadata,
            "market.security_metadata",
            "600519",
        ),
        // M1 扩展 (P4): 6 个新 op fixture roundtrip。
        (Operation::IndexQuotes, "market.index_quotes", "sh000001"),
        (Operation::InstrumentNews, "news.instrument_news", "600519"),
        (
            Operation::IntradayShape,
            "market.intraday_shape",
            "稳步推高",
        ),
        (Operation::T0Evidence, "market.t0_evidence", "600519"),
        (
            Operation::OutcomeDailyBars,
            "market.outcome_daily_bars",
            "2026-08-14",
        ),
        (
            Operation::UpperLimitPoolReview,
            "market.upper_limit_pool_review",
            "600519",
        ),
        // M4c 扩展: A-10 完整 batch (fixture, 字段与 converter 重建一致)。
        (Operation::ChainBatch, "market.chain_batch", "fixture-cb"),
    ];
    for (op, schema, probe) in cases {
        let result = client
            .query(op, serde_json::json!({}))
            .await
            .unwrap_or_else(|e| panic!("{schema} 查询失败: {e}"));
        assert!(result.complete, "{schema} complete=true");
        assert_eq!(result.records.len(), 1, "{schema} 1 条 fixture 记录");
        assert_eq!(result.records[0].schema, schema);
        if op == Operation::T0Evidence {
            assert_eq!(result.records[0].schema_version, 2);
        }
        let parsed: serde_json::Value = serde_json::from_slice(&result.records[0].data).unwrap();
        // 多数 op 视图是数组; T0Evidence 是 {"records": [...], "rejections": [...]} 对象,
        // 非数组时整体检查 (serde_json Index<usize> 对非数组返回 Null)。
        let haystack = if parsed.is_array() {
            parsed[0].to_string()
        } else {
            parsed.to_string()
        };
        assert!(haystack.contains(probe), "{schema} 内容含 {probe}");
    }
    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn subscribe_receives_injected_events_with_monotonic_cursor() {
    let (addr, handle, hub) = start(ServerConfig {
        fixture_mode: true,
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let mut client = GrpcMarketClient::connect(&format!("http://{addr}"))
        .await
        .unwrap();
    let mut stream = client
        .subscribe(
            EventFilter {
                instruments: vec![],
                event_kinds: vec![],
            },
            None,
        )
        .await
        .unwrap();

    let d = DetectedEvent {
        kind: EventKind::Price,
        code: "600519".into(),
        name: "贵州茅台".into(),
        price: 1520.0,
        prev_close: 1500.0,
        change_pct: 1.33,
        volume: 100,
        amount: 1e8,
        reason: "涨跌幅变化".into(),
    };
    hub.push_event(&d);

    use futures::StreamExt;
    let envelope = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .expect("5s 内收到事件")
        .expect("流未结束")
        .expect("事件无错误");
    assert_eq!(envelope.instrument, "600519");
    assert_eq!(envelope.event_kind, "price");
    let cursor = envelope.cursor.unwrap();
    assert_eq!(cursor.sequence, 1);
    assert_eq!(cursor.generation, hub.latest_cursor().generation);
    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn replay_returns_bounded_events_same_generation() {
    let hub = EventHub::new("g1".to_string(), false);
    let d = DetectedEvent {
        kind: EventKind::Price,
        code: "600519".into(),
        name: "贵州茅台".into(),
        price: 1520.0,
        prev_close: 1500.0,
        change_pct: 1.33,
        volume: 100,
        amount: 1e8,
        reason: "涨跌幅变化".into(),
    };
    hub.push_event(&d);
    let q = hub
        .replay_after(Some(EventCursor {
            generation: "g1".into(),
            sequence: 0,
        }))
        .unwrap();
    assert_eq!(q.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn set_watchlist_then_status_match_then_subscribe_filter() {
    // README (client-bundle) 流程: SetWatchlist 完全替换 → GetListenerStatus
    // 直到 desired/applied revisions+lists 匹配 → Subscribe.filter.instruments
    // 只过滤投递事件。本地 server 同步应用 (desired==applied), 一次即匹配。
    let (addr, handle, hub) = start(ServerConfig {
        fixture_mode: true,
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let mut client = GrpcMarketClient::connect(&format!("http://{addr}"))
        .await
        .unwrap();

    // 1. SetWatchlist 完全替换。
    let set = client
        .set_watchlist(vec!["600519".to_string(), "000001".to_string()])
        .await
        .unwrap();
    assert_eq!(set.state, "APPLIED");
    assert_eq!(set.instruments.len(), 2);
    assert_eq!(
        set.desired_revision, 2,
        "初始 STOCK_LIST 为空时 revision 1 → 2"
    );

    // 2. GetListenerStatus: desired 与 applied 立即匹配 (本地同步应用)。
    let status = client.get_listener_status().await.unwrap();
    assert_eq!(
        status.desired_watchlist_revision, status.applied_watchlist_revision,
        "desired == applied (本地 server 无异步应用流程)"
    );
    assert_eq!(
        status.desired_instruments,
        vec!["600519".to_string(), "000001".to_string()]
    );
    assert_eq!(status.applied_instruments, status.desired_instruments);

    // 3. Subscribe.filter.instruments 只投递匹配事件 (README 语义)。
    let mut stream = client
        .subscribe(
            EventFilter {
                instruments: vec!["600519".to_string()],
                event_kinds: vec![],
            },
            None,
        )
        .await
        .unwrap();
    let mk = |code: &str| DetectedEvent {
        kind: EventKind::Price,
        code: code.into(),
        name: "测试".into(),
        price: 1500.0,
        prev_close: 1490.0,
        change_pct: 0.67,
        volume: 100,
        amount: 1e8,
        reason: "涨跌幅变化".into(),
    };
    hub.push_event(&mk("600519"));
    hub.push_event(&mk("000001"));

    use futures::StreamExt;
    let got = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .expect("5s 内收到事件")
        .expect("流未结束")
        .expect("事件无错误");
    assert_eq!(
        got.instrument, "600519",
        "filter 只投递 600519, 000001 被过滤"
    );
    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_schema_rejected() {
    let (addr, handle, _hub) = start(ServerConfig {
        fixture_mode: true,
        port: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let mut client = GrpcMarketClient::connect(&format!("http://{addr}"))
        .await
        .unwrap();
    // OptionData 未实现 → 客户端拦截。
    let err = client
        .query(Operation::OptionData, serde_json::json!({}))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        stock_analysis::grpc_client::errors::GrpcError::Unimplemented { .. }
    ));
    handle.abort();
}
