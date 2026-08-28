//! P4 M3 双进程 smoke: spawn 真 grpc_market_server 二进制 (fixture 模式, 随机端口) →
//! 测试进程 DATA_GATEWAY_GRPC=1 → 桥驱动网关全部已 hook op fetch →
//! 断言 GatewayBatch / evidence / 数据保真。
//!
//! 独立测试 binary 的原因: DATA_GATEWAY_GRPC 是进程级 env + OnceLock SOURCE 缓存,
//! 不能与 library 模式测试同进程 (env/缓存会跨测试泄漏)。
//!
//! 递归防护: server 子进程显式 env_remove("DATA_GATEWAY_GRPC") — delegate 内部调用
//! 本地网关 (fetch_technical_bars → fifteen_min_bars 等), 若继承 env 会形成
//! 桥 → 服务端 → 本地网关 → 桥 的无限递归。这是生产部署的强制约束 (M4 banner 文档化)。
use chrono::{Duration, NaiveDate, Timelike};
use magic_market_core::{
    AssetClass, Exchange, FlowInterval, InstrumentId, NorthboundChannel, ProviderId, StatementKind,
};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration as StdDuration, Instant};
use stock_analysis::data_gateway::board_ranking::BoardRankingGateway;
use stock_analysis::data_gateway::{
    grpc_source, BlockTradesGateway, BoardDataGateway, BoardKind, CapitalDataGateway,
    CompanyDataGateway, ConsensusDataGateway, DragonTigerGateway, GeneralWebResearchBatch,
    GeneralWebResearchGateway, GeneralWebResearchProvider, IndexDataGateway, MarketDataGateway,
    ReviewDataGateway,
};
use stock_analysis::database::DatabaseManager;
use stock_analysis::grpc_client::client::GrpcMarketClient;
use stock_analysis::grpc_client::envelope::QueryResult;
use stock_analysis::grpc_client::pb::magic::market::v1::{AdmissionState, Operation};

/// 拿空闲端口 (绑定后 drop; 竞态窗口对测试可接受)。
fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct FixtureServerGuard {
    child: Option<Child>,
}

impl FixtureServerGuard {
    fn terminate_and_reap(&mut self) -> std::io::Result<ExitStatus> {
        let child = self.child.as_mut().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "fixture server child already reaped",
            )
        })?;
        // kill may report InvalidInput if the server exited between the last
        // assertion and cleanup. wait remains mandatory so the child is reaped.
        let _ = child.kill();
        let status = child.wait()?;
        self.child = None;
        Ok(status)
    }
}

impl Drop for FixtureServerGuard {
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.terminate_and_reap();
        }
    }
}

/// spawn fixture 模式 server (cargo 为集成测试提供 CARGO_BIN_EXE_* 路径)。
fn spawn_fixture_server(port: u16) -> FixtureServerGuard {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_grpc_market_server"));
    cmd.env("GRPC_GATEWAY_TEST_FIXTURE", "1")
        .env("GRPC_MARKET_PORT", port.to_string())
        .env_remove("DATA_GATEWAY_GRPC") // 递归防护 (见文件头)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    FixtureServerGuard {
        child: Some(cmd.spawn().expect("spawn grpc_market_server")),
    }
}

/// sync 网关方法 (realtime_quotes/fifteen_min_bars/fetch_top/index/t0) 的调用
/// 契约: block_on 判别器三路安全 (async worker → 独立线程; spawn_blocking →
/// Handle::block_on; 纯同步线程 → BRIDGE_RUNTIME), 任何上下文都不 panic
/// (2026-08-15 生产事故修复)。这里仍用 spawn_blocking 保持与真实调用方
/// (monitor 盘后调度在 blocking 线程执行) 一致, 且不阻塞测试 worker。
async fn blocking<T>(f: impl FnOnce() -> T + Send + 'static, what: &str) -> T
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| panic!("spawn_blocking join 失败 ({what}): {e}"))
}

/// TCP 可达 + 短暂稳定窗口后返回。
async fn wait_ready(port: u16) {
    let deadline = Instant::now() + StdDuration::from_secs(15);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            tokio::time::sleep(StdDuration::from_millis(500)).await;
            return;
        }
        tokio::time::sleep(StdDuration::from_millis(100)).await;
    }
    panic!("grpc_market_server 15s 内未就绪 (port {port})");
}

async fn raw_fixture_wire(
    client: &mut GrpcMarketClient,
    operation: Operation,
    params: serde_json::Value,
    schema: &str,
    schema_version: u32,
    provider: &str,
    batch_id: &str,
) -> QueryResult {
    let result = client
        .query(operation, params)
        .await
        .unwrap_or_else(|error| panic!("{operation:?} raw fixture wire failed: {error}"));
    assert_eq!(result.admission, AdmissionState::Admitted);
    assert!(result.complete);
    assert_eq!(result.selected_provider, provider);
    assert_eq!(result.batch_id, batch_id);
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].schema, schema);
    assert_eq!(result.records[0].schema_version, schema_version);
    result
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_all_hooked_ops_fixture_roundtrip() {
    // env 是进程级 + bridge SOURCE 一次性缓存 → 单 test 覆盖全部 op。
    std::env::set_var("DATA_GATEWAY_GRPC", "1");
    // audit_gateway_result (桥路径 audit 留客户端) 写 DataAcquisitionAuditRecord →
    // 需数据库初始化 (与 e2e_dedup.rs 同模式)。
    std::fs::create_dir_all("./test_data").ok();
    let db_path = PathBuf::from(format!(
        "./test_data/grpc_bridge_e2e_{}.db",
        std::process::id()
    ));
    DatabaseManager::init(Some(db_path)).expect("audit database init");
    grpc_source::reset_bridge();
    let port = free_port();
    let mut server = spawn_fixture_server(port);
    std::env::set_var("GRPC_MARKET_ADDR", format!("http://127.0.0.1:{port}"));
    wait_ready(port).await;
    let bridge = grpc_source::bridge_for("OutcomeDailyBars")
        .expect("TEST_CODE bridge lookup")
        .expect("DATA_GATEWAY_GRPC enables one local bridge");
    let test_code = "TEST_CODE_600519".to_owned();
    let mut raw_client = GrpcMarketClient::connect(&format!("http://127.0.0.1:{port}"))
        .await
        .expect("connect raw fixture client");

    let date = NaiveDate::from_ymd_opt(2026, 8, 15).unwrap();

    // ---- Gate C: LocalBridge dormant typed wrappers ----
    let announcements = bridge.announcements_async().await.unwrap();
    assert_eq!(announcements.records().len(), 1);
    assert_eq!(announcements.evidence().batch_id, "fixture-b1");
    assert_eq!(announcements.records()[0].code, test_code);

    let futures_delivery = bridge.futures_delivery_async().await.unwrap();
    assert_eq!(futures_delivery.records().len(), 1);
    assert_eq!(futures_delivery.evidence().batch_id, "fixture-b1");
    assert_eq!(
        futures_delivery.records()[0].contract_code,
        "TEST_CODE_IF2608"
    );
    assert_eq!(futures_delivery.records()[0].product_code, "TEST_CODE_IF");

    let board_constituents = bridge.board_constituents_async(&test_code).await.unwrap();
    assert_eq!(board_constituents.records().len(), 1);
    assert_eq!(board_constituents.evidence().batch_id, "fixture-b1");
    assert_eq!(board_constituents.records()[0].instrument_code, test_code);
    assert_eq!(
        board_constituents.records()[0].board_code,
        "TEST_CODE_BK0475"
    );

    let research_reports = bridge.research_reports_async(&test_code, 5).await.unwrap();
    assert_eq!(research_reports.records().len(), 1);
    assert_eq!(research_reports.evidence().batch_id, "fixture-b1");
    assert_eq!(
        research_reports.records()[0].title,
        "TEST_CODE_测试证券:2026年中报点评"
    );

    let technical_bars = bridge
        .technical_bars_async(std::slice::from_ref(&test_code), 100)
        .await
        .unwrap();
    assert_eq!(technical_bars.records().len(), 1);
    assert_eq!(technical_bars.evidence().batch_id, "fixture-b1");

    let intraday_shape = bridge.intraday_shape_async(&test_code).await.unwrap();
    assert_eq!(intraday_shape.records().len(), 1);
    assert_eq!(intraday_shape.evidence().batch_id, "fixture-b1");

    for error in [
        bridge.foreign_exchange_async().await.unwrap_err(),
        bridge.economic_calendar_async().await.unwrap_err(),
        bridge
            .market_statistics_async(std::slice::from_ref(&test_code))
            .await
            .unwrap_err(),
        bridge.provider_top_n_pair_async(date).await.unwrap_err(),
    ] {
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    let news_error = bridge
        .instrument_news_async(std::slice::from_ref(&test_code), 5)
        .await
        .expect_err("production identity resolver rejects TEST_CODE before RPC");
    assert_eq!(news_error.reason_code(), "invalid_request");
    assert!(!news_error.retryable());

    // GlobalIndices is advertised but has no fixture response arm; preserve
    // the existing unsupported fail-closed contract instead of fabricating data.
    let global_indices_error = bridge.global_indices_async().await.unwrap_err();
    assert_eq!(global_indices_error.reason_code(), "invalid_request");
    assert!(!global_indices_error.retryable());

    // ---- M2 首批 ----
    let quotes = blocking(
        move || {
            MarketDataGateway::new()
                .realtime_quotes(&["TEST_CODE_600519".to_string()])
                .expect("RealtimeQuotes 桥")
        },
        "RealtimeQuotes",
    )
    .await;
    assert_eq!(
        quotes.records()[0].code,
        "TEST_CODE_600519",
        "RealtimeQuotes 保真"
    );
    assert_eq!(
        quotes.evidence().provider,
        ProviderId::Tdx,
        "RealtimeQuotes evidence.provider"
    );

    // Production HistoricalBarsGateway rejects TEST_CODE before RPC by design;
    // this raw fixture assertion covers only the admitted wire contract.
    let technical = raw_fixture_wire(
        &mut raw_client,
        Operation::TechnicalBars,
        serde_json::json!({"codes": ["TEST_CODE_600519"], "count": 100}),
        "market.technical_bars",
        1,
        "Tdx",
        "fixture-b1",
    )
    .await;
    let technical_payload: serde_json::Value =
        serde_json::from_slice(&technical.records[0].data).expect("TechnicalBars raw JSON");
    assert_eq!(technical_payload.as_array().map(Vec::len), Some(1));

    // ---- M3: 板块 ----
    let dir = BoardDataGateway::new()
        .directory(BoardKind::Concept, 50)
        .await
        .expect("BoardDirectory 桥");
    assert_eq!(
        dir.records()[0].code,
        "TEST_CODE_BK0475",
        "BoardDirectory 保真"
    );
    assert_eq!(dir.records()[0].kind, BoardKind::Concept, "BoardKind 保真");

    // Production board membership identity resolution rejects TEST_CODE before RPC.
    let memberships = raw_fixture_wire(
        &mut raw_client,
        Operation::BoardConstituents,
        serde_json::json!({"codes": ["TEST_CODE_600519"]}),
        "board.constituents",
        1,
        "Tdx",
        "fixture-b1",
    )
    .await;
    let memberships_payload: serde_json::Value =
        serde_json::from_slice(&memberships.records[0].data).expect("BoardConstituents raw JSON");
    assert_eq!(
        memberships_payload[0]["instrument_code"],
        "TEST_CODE_600519"
    );

    let flows = BoardDataGateway::new()
        .day1_flows(BoardKind::Concept, 20)
        .await
        .expect("BoardFlows 桥");
    assert_eq!(
        flows.records()[0].code,
        "TEST_CODE_BK0475",
        "BoardFlows 保真"
    );

    for fid in ["f3", "f62"] {
        let ranking = blocking(
            move || {
                BoardRankingGateway::new()
                    .fetch_top(fid, 20)
                    .expect("BoardRanking 桥")
            },
            "BoardRanking",
        )
        .await;
        assert_eq!(ranking.len(), 1, "BoardRanking({fid}) fixture 1 条");
        assert_eq!(
            ranking[0].code, "TEST_CODE_BK0475",
            "BoardRanking({fid}) 保真"
        );
    }

    // ---- M3: 市场/个股 ----
    // MarketStatistics converter infers exchange from production code prefixes.
    let stats = raw_fixture_wire(
        &mut raw_client,
        Operation::MarketStatistics,
        serde_json::json!({"codes": ["TEST_CODE_600519"]}),
        "market.market_statistics",
        1,
        "Tdx",
        "fixture-b1",
    )
    .await;
    let stats_payload: serde_json::Value =
        serde_json::from_slice(&stats.records[0].data).expect("MarketStatistics raw JSON");
    assert_eq!(stats_payload[0]["code"], "TEST_CODE_600519");

    let idx = blocking(
        || {
            IndexDataGateway::new()
                .realtime_quotes(&["TEST_CODE_INDEX_000001".to_string()])
                .expect("IndexQuotes 桥")
        },
        "IndexQuotes",
    )
    .await;
    assert_eq!(
        idx.records()[0].code,
        "TEST_CODE_INDEX_000001",
        "IndexQuotes 保真"
    );

    // ---- M3: 盘后/共识 ----
    let dt = DragonTigerGateway::new()
        .market_review(date, 100, 20)
        .await
        .expect("DragonTiger 桥");
    assert_eq!(dt.records()[0].code, "TEST_CODE_600519", "DragonTiger 保真");
    assert_eq!(
        dt.records()[0].disclosures.len(),
        1,
        "DragonTiger disclosures 解析"
    );

    let bt = BlockTradesGateway::new()
        .market_review(&["TEST_CODE_600519".to_string()], date)
        .await
        .expect("BlockTrades 桥");
    assert_eq!(bt.records()[0].code, "TEST_CODE_600519", "BlockTrades 保真");
    assert_eq!(bt.records()[0].price, 1490.0, "BlockTrades 保真");

    let cons = ConsensusDataGateway::new()
        .fetch("TEST_CODE_600519")
        .await
        .expect("Consensus 桥");
    assert_eq!(cons.records()[0].report_count, 12, "Consensus 保真");
    assert_eq!(
        cons.records()[0].rating_distribution.get("买入"),
        Some(&5u32),
        "Consensus rating_distribution 解析"
    );

    // ---- M3: R-03 涨停池 + T0 证据批 (rejections 不丢) ----
    let ulp = ReviewDataGateway
        .r03_upper_limit_pool(date)
        .await
        .expect("UpperLimitPoolReview 桥");
    assert_eq!(
        ulp.records()[0].code,
        "TEST_CODE_600519",
        "UpperLimitPoolReview 保真"
    );

    // T0's production converter intentionally rejects TEST_CODE exchange inference.
    // This verifies only the admitted v2 raw fixture wire; Task 8 owns the real
    // provider-backed typed server/client round-trip.
    let t0 = raw_fixture_wire(
        &mut raw_client,
        Operation::T0Evidence,
        serde_json::json!({"codes": ["TEST_CODE_600519"]}),
        "market.t0_evidence",
        2,
        "Tdx",
        "fixture-b1",
    )
    .await;
    let t0_payload: serde_json::Value =
        serde_json::from_slice(&t0.records[0].data).expect("T0Evidence raw JSON");
    assert_eq!(t0_payload["batch_id"], "fixture-b1");
    assert_eq!(t0_payload["records"][0]["instrument"], "TEST_CODE_600519");
    assert_eq!(t0_payload["records"][0]["code"], "TEST_CODE_600519");
    assert_eq!(
        t0_payload["records"][0]["completed_five_minute"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let bar_at = chrono::DateTime::parse_from_rfc3339(
        t0_payload["records"][0]["completed_five_minute"][0]["at"]
            .as_str()
            .expect("T0 five-minute at string"),
    )
    .expect("T0 five-minute at RFC3339");
    assert_eq!(bar_at.time().hour(), 13);
    assert_eq!(t0_payload["rejections"].as_array().map(Vec::len), Some(0));

    // InstrumentNews is intentionally absent here: production routes it through
    // authenticated ExternalV1, while this process only hosts the LocalBridgeV1
    // fixture server. Its strict ExternalV1 contract is covered in convert tests
    // and by the release bundle probe; silently falling back here would mask a
    // missing client-bundle in production.

    // ---- M4b 批次 1A: 6 个新桥 op (fixture 视图与 delegate fetch_* 对齐) ----
    let reports = raw_fixture_wire(
        &mut raw_client,
        Operation::ResearchReports,
        serde_json::json!({"codes": ["TEST_CODE_600519"], "page_size": 5}),
        "research.reports",
        1,
        "Tdx",
        "fixture-b1",
    )
    .await;
    let reports_payload: serde_json::Value =
        serde_json::from_slice(&reports.records[0].data).expect("ResearchReports raw JSON");
    assert_eq!(reports_payload[0]["code"], "TEST_CODE_600519");

    let northbound = CapitalDataGateway::new()
        .northbound_daily(date, NorthboundChannel::Shanghai)
        .await
        .expect("NorthboundDaily 桥");
    assert_eq!(
        northbound.records()[0].channel,
        NorthboundChannel::Shanghai,
        "NorthboundDaily channel 保真"
    );
    assert_eq!(
        northbound.records()[0].total_turnover,
        5.2e10,
        "NorthboundDaily 保真"
    );
    assert_eq!(
        northbound.records()[0].top_turnover[0].name,
        "TEST_CODE_测试证券",
        "NorthboundDaily top_turnover 解析"
    );

    let statements = CompanyDataGateway::new()
        .financial_statements(&["TEST_CODE_600519".to_string()], StatementKind::Balance)
        .await
        .expect("FinancialStatements 桥");
    assert_eq!(
        statements.records()[0].kind,
        StatementKind::Balance,
        "FinancialStatements kind 保真"
    );
    assert_eq!(
        statements.records()[0].lines[0].key.as_str(),
        "total_assets",
        "FinancialStatements lines 保真"
    );

    let flows = CapitalDataGateway::new()
        .instrument_fund_flow("TEST_CODE_600519", FlowInterval::Day1, 20)
        .await
        .expect("FundFlowSeries 桥");
    assert_eq!(flows.records()[0].main_net, 5e7, "FundFlowSeries 保真");
    assert_eq!(
        flows.records()[0].interval,
        FlowInterval::Day1,
        "FundFlowSeries interval 保真"
    );

    let pair = raw_fixture_wire(
        &mut raw_client,
        Operation::ProviderTopNRankings,
        serde_json::json!({"date": date.format("%Y-%m-%d").to_string()}),
        "market.provider_top_n_rankings",
        1,
        "Eastmoney",
        "fixture-b1",
    )
    .await;
    let pair_payload: serde_json::Value =
        serde_json::from_slice(&pair.records[0].data).expect("ProviderTopNRankings raw JSON");
    assert_eq!(pair_payload.as_array().map(Vec::len), Some(2));
    assert!(pair_payload
        .as_array()
        .expect("ProviderTopNRankings array")
        .iter()
        .all(|record| record["code"] == "TEST_CODE_600519"));

    let shape = raw_fixture_wire(
        &mut raw_client,
        Operation::IntradayShape,
        serde_json::json!({"codes": ["TEST_CODE_600519"]}),
        "market.intraday_shape",
        1,
        "Tdx",
        "fixture-b1",
    )
    .await;
    let shape_payload: serde_json::Value =
        serde_json::from_slice(&shape.records[0].data).expect("IntradayShape raw JSON");
    assert_eq!(shape_payload[0]["shape_label"], "稳步推高");

    // ---- M4b 批次 1B: semantic_search + corporate_actions (新桥方法) ----
    let ws = GeneralWebResearchGateway::from_environment(GeneralWebResearchProvider::Bocha)
        .search("白酒 景气", 10)
        .await
        .expect("SemanticSearch 桥");
    assert_eq!(
        ws.evidence().provider,
        GeneralWebResearchProvider::Bocha,
        "SemanticSearch 批级 provider 保真"
    );
    let ws_records = match &ws {
        GeneralWebResearchBatch::Available { records, .. } => records,
        GeneralWebResearchBatch::VerifiedEmpty(_) => panic!("SemanticSearch 不应为空 (fixture)"),
    };
    assert_eq!(
        ws_records[0].title, "白酒行业景气度跟踪",
        "SemanticSearch 保真"
    );
    assert_eq!(
        ws_records[0].evidence.batch_id, "fixture-b1",
        "SemanticSearch 记录级 evidence.batch_id 保真"
    );

    let security = raw_fixture_wire(
        &mut raw_client,
        Operation::SecurityMetadata,
        serde_json::json!({"codes": ["TEST_CODE_600519"]}),
        "market.security_metadata",
        1,
        "Tdx",
        "fixture-b1",
    )
    .await;
    let security_payload: serde_json::Value =
        serde_json::from_slice(&security.records[0].data).expect("SecurityMetadata raw JSON");
    assert_eq!(security_payload[0]["code"], "TEST_CODE_600519");

    let actions = raw_fixture_wire(
        &mut raw_client,
        Operation::CorporateActions,
        serde_json::json!({
            "code": "TEST_CODE_600519",
            "window_start": (date - Duration::days(180)).format("%Y-%m-%d").to_string(),
            "window_end": date.format("%Y-%m-%d").to_string()
        }),
        "market.corporate_actions",
        1,
        "Tdx",
        "fixture-b1",
    )
    .await;
    let actions_payload: serde_json::Value =
        serde_json::from_slice(&actions.records[0].data).expect("CorporateActions raw JSON");
    assert_eq!(actions_payload[0]["code"], "TEST_CODE_600519");

    // ---- P4 M3 批次 2: outcome_daily_bars 服务端真实现 (adaptive 视图重建) ----
    let raw = blocking(
        move || {
            grpc_source::bridge_for("OutcomeDailyBars")
                .expect("bridge")
                .expect("桥未启用")
                .outcome_daily_bars_adaptive(
                    InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600519", AssetClass::Equity)
                        .expect("instrument"),
                    "SH".to_string(),
                    "TEST_CODE_600519".to_string(),
                    1,
                    256,
                    date,
                )
                .expect("OutcomeDailyBars 桥")
        },
        "OutcomeDailyBars",
    )
    .await;
    assert_eq!(raw.batch.records().len(), 1, "OutcomeDailyBars batch 保真");
    assert_eq!(
        raw.batch.records()[0].close().get(),
        1500.0,
        "OutcomeDailyBars close 保真"
    );
    assert_eq!(
        raw.batch.records()[0].instrument().code(),
        "TEST_CODE_600519",
        "OutcomeDailyBars instrument 保真"
    );
    assert!(
        raw.batch.quality().is_complete(),
        "OutcomeDailyBars quality 保真"
    );

    server
        .terminate_and_reap()
        .expect("terminate and reap server");
    grpc_source::reset_bridge();
}
