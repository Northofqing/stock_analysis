//! gRPC mock 服务端 (合同 grpc/grpc-external-api.md, 方案 A 委托 data_gateway)。
//! 默认 127.0.0.1:18082; GRPC_MARKET_PORT / GRPC_GATEWAY_TEST_FIXTURE / GRPC_EVENTS_SHADOW 可配。
//! 只读数据服务 + TDX 异动事件订阅。无账户/持仓/委托写接口。
//! 事件轮询: EVENT_POLL_INTERVAL_MS / EVENT_PRICE_THRESHOLD_PCT / EVENT_VOLUME_THRESHOLD_X 可配。

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // P4 M4 递归防护 (fail-closed, v15.x 出声): delegate 内部调用本地网关
    // (fetch_technical_bars → fifteen_min_bars 等), 而桥钩子就在网关方法里。
    // 本进程设 DATA_GATEWAY_GRPC=1 → 网关把请求桥回本服务端 → 无限递归。
    // 生产部署约束: grpc_market_server 与 monitor 是不同进程, 只有 monitor 设此 env。
    if std::env::var("DATA_GATEWAY_GRPC").as_deref() == Ok("1") {
        eprintln!(
            "[grpc_market_server] 启动失败: DATA_GATEWAY_GRPC=1 被禁止 — \
             服务端 delegate 走本地网关, 该 env 会把请求桥回自身形成无限递归。\
             请 unset DATA_GATEWAY_GRPC 后重启 (只有 monitor 进程可设此 env)。"
        );
        std::process::exit(2);
    }
    // M4c: A-10 delegate (op 44/45/61) 执行 build_for_date 计算+stage+publish,
    // 需要与 monitor 同一 SQLite 库 (DATABASE_PATH, 默认 ./data/stock_analysis.db)。
    // 未 init → DatabaseManager::try_get()=None → A-10 delegate 生产必失败。
    // fixture 模式 (GRPC_GATEWAY_TEST_FIXTURE=1) 全部 delegate 走 canned 数据,
    // 不需要 DB — 跳过 init (测试 cwd 无 data/ 目录, init 会失败)。
    if std::env::var("GRPC_GATEWAY_TEST_FIXTURE").as_deref() != Ok("1") {
        let db_path = std::env::var("DATABASE_PATH")
            .unwrap_or_else(|_| "./data/stock_analysis.db".to_string());
        stock_analysis::database::DatabaseManager::init(Some(std::path::PathBuf::from(&db_path)))
            .map_err(|e| anyhow::anyhow!("DatabaseManager::init({db_path}) 失败: {e}"))?;
        log::info!("[grpc_market_server] 数据库已初始化: {db_path} (A-10 链 delegate 依赖)");
        // M4c: build_for_date 的 BR-160 聚类合同来自 config/chain.toml
        // (config::load_chain_combined, monitor 启动时经 config::load_all() 加载)。
        // 服务端不加载 → get_chain_intelligence_config()=None → op 61 必失败
        // (chain_policy_unavailable, Task #75 生产探针实证)。load_all 与 monitor
        // 同语义: 读失败 → 合同 None → delegate 查询时 fail-closed 出声, 无静默兜底。
        stock_analysis::config::load_all();
        log::info!(
            "[grpc_market_server] config::load_all() 完成 (chain.toml 合同, A-10 delegate 依赖)"
        );
    }

    let config = stock_analysis::grpc_server::ServerConfig::default();
    let (addr, handle, hub) = stock_analysis::grpc_server::start(config).await?;
    log::info!("[grpc_market_server] 就绪: {addr} (Ctrl-C 退出)");

    // ---- 事件轮询 (Task 11 Step 4) ----
    let poll = stock_analysis::grpc_server::events::poll_interval_ms();
    let (price_t, vol_t) = stock_analysis::grpc_server::events::thresholds();
    // v15.x 出声: 统一行情 RealtimeMarketQuote 合同无 volume/amount 字段 →
    // 真实轮询仅产生 Price 事件; Volume/Amount/Status 事件在注入路径可用。
    log::info!(
        "[grpc_market_server] 事件轮询间隔 {poll}ms, 阈值 {price_t:.2}pp/{vol_t:.2}x; \
         volume/amount 无上游字段 → 真实轮询仅 Price 事件 (Volume/Amount/Status 走注入)"
    );

    let hub_for_poll = hub.clone();
    let poll_task = tokio::spawn(async move {
        let mut prev: Vec<stock_analysis::grpc_server::events::Quote> = Vec::new();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(poll)).await;
            let codes: Vec<String> = std::env::var("STOCK_LIST")
                .map(|s| {
                    s.split(',')
                        .map(|c| c.trim().to_string())
                        .filter(|c| !c.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            if codes.is_empty() {
                continue;
            }
            let Ok(batch) =
                stock_analysis::data_gateway::MarketDataGateway::new().realtime_quotes(&codes)
            else {
                continue; // 拉取失败跳过本周期, 保留上一快照
            };
            let next: Vec<stock_analysis::grpc_server::events::Quote> = batch
                .records()
                .iter()
                .map(|s| stock_analysis::grpc_server::events::Quote {
                    code: s.code.clone(),
                    name: s.name.clone(),
                    price: s.price,
                    prev_close: s.previous_close,
                    volume: 0,
                    amount: 0.0,
                })
                .collect();
            let events =
                stock_analysis::grpc_server::events::diff_snapshots(&prev, &next, price_t, vol_t);
            for e in events {
                hub_for_poll.push_event(&e);
            }
            prev = next;
        }
    });

    tokio::select! {
        r = handle => r??,
        r = poll_task => { r?; },
        _ = tokio::signal::ctrl_c() => log::info!("[grpc_market_server] 收到 Ctrl-C, 退出"),
    }
    Ok(())
}
