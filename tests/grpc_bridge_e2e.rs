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
use chrono::{Duration, NaiveDate, Utc};
use magic_market_core::{
    FlowInterval, MarketRankingKind, NorthboundChannel, ProviderId, StatementKind,
};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration as StdDuration, Instant};
use stock_analysis::database::DatabaseManager;
use stock_analysis::data_gateway::board_ranking::BoardRankingGateway;
use stock_analysis::data_gateway::{
    grpc_source, BlockTradesGateway, BoardDataGateway, BoardKind, CapitalDataGateway,
    CompanyDataGateway, ConsensusDataGateway, DragonTigerGateway, HistoricalBarsGateway,
    IndexDataGateway, IntradayShapeGateway, MagicTdxGateway, MarketDataGateway,
    ResearchDataGateway, ReviewDataGateway, SinaInstrumentNewsGateway,
};

/// 拿空闲端口 (绑定后 drop; 竞态窗口对测试可接受)。
fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// spawn fixture 模式 server (cargo 为集成测试提供 CARGO_BIN_EXE_* 路径)。
fn spawn_fixture_server(port: u16) -> std::process::Child {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_grpc_market_server"));
    cmd.env("GRPC_GATEWAY_TEST_FIXTURE", "1")
        .env("GRPC_MARKET_PORT", port.to_string())
        .env_remove("DATA_GATEWAY_GRPC") // 递归防护 (见文件头)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn().expect("spawn grpc_market_server")
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

#[tokio::test(flavor = "multi_thread")]
async fn bridge_all_hooked_ops_fixture_roundtrip() {
    // env 是进程级 + bridge SOURCE 一次性缓存 → 单 test 覆盖全部 op。
    std::env::set_var("DATA_GATEWAY_GRPC", "1");
    // audit_gateway_result (桥路径 audit 留客户端) 写 DataAcquisitionAuditRecord →
    // 需数据库初始化 (与 e2e_dedup.rs 同模式)。
    std::fs::create_dir_all("./test_data").ok();
    let db_path = PathBuf::from(format!("./test_data/grpc_bridge_e2e_{}.db", std::process::id()));
    DatabaseManager::init(Some(db_path)).expect("audit database init");
    grpc_source::reset_bridge();
    let port = free_port();
    let mut server = spawn_fixture_server(port);
    std::env::set_var("GRPC_MARKET_ADDR", format!("http://127.0.0.1:{port}"));
    wait_ready(port).await;

    let date = NaiveDate::from_ymd_opt(2026, 8, 15).unwrap();
    let now = Utc::now();

    // ---- M2 首批 ----
    let quotes = blocking(
        move || {
            MarketDataGateway::new()
                .realtime_quotes(&["600519".to_string()])
                .expect("RealtimeQuotes 桥")
        },
        "RealtimeQuotes",
    )
    .await;
    assert_eq!(quotes.records()[0].code, "600519", "RealtimeQuotes 保真");
    assert_eq!(
        quotes.evidence().provider,
        ProviderId::Tdx,
        "RealtimeQuotes evidence.provider"
    );

    let bars = blocking(
        || {
            HistoricalBarsGateway::new()
                .fifteen_min_bars("600519", 100)
                .expect("TechnicalBars 桥")
        },
        "TechnicalBars",
    )
    .await;
    assert_eq!(bars.len(), 1, "TechnicalBars fixture 1 条");
    assert_eq!(bars[0].close, 1500.0, "TechnicalBars 保真");

    // ---- M3: 板块 ----
    let dir = BoardDataGateway::new()
        .directory(BoardKind::Concept, 50)
        .await
        .expect("BoardDirectory 桥");
    assert_eq!(dir.records()[0].code, "BK0475", "BoardDirectory 保真");
    assert_eq!(dir.records()[0].kind, BoardKind::Concept, "BoardKind 保真");

    let memberships = BoardDataGateway::new()
        .memberships("600519")
        .await
        .expect("BoardConstituents 桥");
    assert_eq!(memberships.records()[0].instrument_code, "600519", "BoardConstituents 保真");

    let flows = BoardDataGateway::new()
        .day1_flows(BoardKind::Concept, 20)
        .await
        .expect("BoardFlows 桥");
    assert_eq!(flows.records()[0].code, "BK0475", "BoardFlows 保真");

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
        assert_eq!(ranking[0].code, "BK0475", "BoardRanking({fid}) 保真");
    }

    // ---- M3: 市场/个股 ----
    let stats = CompanyDataGateway::new()
        .market_statistics(&["600519".to_string()])
        .await
        .expect("MarketStatistics 桥");
    assert_eq!(
        stats.records()[0].instrument().code().to_string(),
        "600519",
        "MarketStatistics 保真"
    );
    assert!(
        stats.records()[0].trailing_pe().is_some(),
        "MarketStatistics trailing_pe 解析"
    );

    let idx = blocking(
        || {
            IndexDataGateway::new()
                .realtime_quotes(&["sh000001".to_string()])
                .expect("IndexQuotes 桥")
        },
        "IndexQuotes",
    )
    .await;
    assert_eq!(idx.records()[0].code, "sh000001", "IndexQuotes 保真");

    // ---- M3: 盘后/共识 ----
    let dt = DragonTigerGateway::new()
        .market_review(date, 100, 20)
        .await
        .expect("DragonTiger 桥");
    assert_eq!(dt.records()[0].code, "600519", "DragonTiger 保真");
    assert_eq!(dt.records()[0].disclosures.len(), 1, "DragonTiger disclosures 解析");

    let bt = BlockTradesGateway::new()
        .market_review(&["600519".to_string()], date)
        .await
        .expect("BlockTrades 桥");
    assert_eq!(bt.records()[0].code, "600519", "BlockTrades 保真");
    assert_eq!(bt.records()[0].price, 1490.0, "BlockTrades 保真");

    let cons = ConsensusDataGateway::new()
        .fetch("600519")
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
    assert_eq!(ulp.records()[0].code, "600519", "UpperLimitPoolReview 保真");

    let t0 = blocking(
        move || {
            MagicTdxGateway::new()
                .get_t0_evidence_batch(&["600519".to_string()], now)
                .expect("T0Evidence 桥")
        },
        "T0Evidence",
    )
    .await;
    assert_eq!(t0.records.len(), 1, "T0Evidence records");
    assert_eq!(t0.records[0].code, "600519", "T0Evidence 保真");
    assert!(t0.rejections.is_empty(), "T0Evidence rejections 空 (fixture)");
    // 批级 batch_id = 信封 batch_id (fixture-b1); 记录级 batch_id 保留在 records[i]。
    assert_eq!(t0.batch_id, "fixture-b1", "T0Evidence 批级 batch_id");
    assert_eq!(t0.records[0].batch_id, "fixture-t0", "T0Evidence 记录级 batch_id 保真");

    // ---- M3: 个股新闻 (from_days=30 契约) ----
    let news = SinaInstrumentNewsGateway::new()
        .instrument_news_in_range("600519", now - Duration::days(30), now)
        .await
        .expect("InstrumentNews 桥");
    assert_eq!(
        news.records()[0].persistence_item().code.as_deref(),
        Some("600519"),
        "InstrumentNews 保真"
    );

    // ---- M4b 批次 1A: 6 个新桥 op (fixture 视图与 delegate fetch_* 对齐) ----
    let reports = ResearchDataGateway::new()
        .instrument_reports("600519", 5)
        .await
        .expect("ResearchReports 桥");
    assert_eq!(reports.records()[0].report_id, "fixture-r1", "ResearchReports 保真");
    assert_eq!(
        reports.records()[0].source_target_price_upper,
        Some(1600.0),
        "ResearchReports target_price_upper 解析"
    );

    let northbound = CapitalDataGateway::new()
        .northbound_daily(date, NorthboundChannel::Shanghai)
        .await
        .expect("NorthboundDaily 桥");
    assert_eq!(northbound.records()[0].channel, NorthboundChannel::Shanghai, "NorthboundDaily channel 保真");
    assert_eq!(northbound.records()[0].total_turnover, 5.2e10, "NorthboundDaily 保真");
    assert_eq!(
        northbound.records()[0].top_turnover[0].name, "贵州茅台",
        "NorthboundDaily top_turnover 解析"
    );

    let statements = CompanyDataGateway::new()
        .financial_statements(&["600519".to_string()], StatementKind::Balance)
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
        .instrument_fund_flow("600519", FlowInterval::Day1, 20)
        .await
        .expect("FundFlowSeries 桥");
    assert_eq!(flows.records()[0].main_net, 5e7, "FundFlowSeries 保真");
    assert_eq!(
        flows.records()[0].interval,
        FlowInterval::Day1,
        "FundFlowSeries interval 保真"
    );

    let pair = CapitalDataGateway::new()
        .provider_top_n_pair(date)
        .await
        .expect("ProviderTopNRankings 桥");
    assert_eq!(
        pair.volume_ratio.records()[0].instrument.code().to_string(),
        "600519",
        "ProviderTopNRankings volume 保真"
    );
    assert_eq!(pair.volume_ratio.records()[0].metric, MarketRankingKind::VolumeRatio);
    assert_eq!(pair.main_net_inflow.records()[0].metric, MarketRankingKind::MainNetInflow);

    let shape = IntradayShapeGateway::new()
        .current_shape("600519")
        .await
        .expect("IntradayShape 桥");
    assert_eq!(shape.records()[0].shape_label, "稳步推高", "IntradayShape 保真");

    server.kill().expect("kill server");
    grpc_source::reset_bridge();
}
