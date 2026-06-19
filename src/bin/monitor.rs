//! 实盘监控模式入口。
//!
//! 用法：
//!   cargo run --bin monitor             # 正常监控（等交易日+交易时段）
//!   cargo run --bin monitor -- --test   # 测试模式（跳过日历，立即跑一次扫描验证）
//!
//! 依赖 .env 中 MONITOR_ENABLED=true

use std::io::Write;
use stock_analysis::calendar::{self, current_session, is_market_active, MarketSession};
use stock_analysis::monitor::detector::{AlertCategory, AlertDetail, AlertEvent, AlertLevel, Detector, DetectorConfig, StockSnapshot};
use stock_analysis::monitor::signal_state::SignalStateMachine;
use stock_analysis::monitor::scanner::TieredScanner;
use stock_analysis::monitor::checklist;
use stock_analysis::monitor::prediction;
use stock_analysis::monitor::alert;

fn main() {
    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|buf, record| writeln!(buf, "[{} {}] {}", chrono::Local::now().format("%H:%M:%S"), record.level(), record.args()))
        .init();

    if !check_enabled() { return; }
    // 初始化数据库
    let db_path = std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/stock_analysis.db".into());
    let _ = stock_analysis::database::DatabaseManager::init(Some(std::path::PathBuf::from(&db_path)));
    // 加载热配置（toml 不可用时回退代码默认值）
    stock_analysis::config::load_all();
    let test_mode = std::env::args().any(|a| a == "--test");
    let review_mode = std::env::args().any(|a| a == "--review");

    log::info!("实盘监控启动 | {} | 当前: {} | 模式: {}",
        if calendar::today_is_trading_day() { "交易日" } else { "非交易日" },
        calendar::session_label(),
        if test_mode { "测试" } else if review_mode { "复盘" } else { "正常" },
    );

    let rt = tokio::runtime::Runtime::new().expect("创建 tokio runtime 失败");
    if test_mode {
        rt.block_on(run_test_scan());
    } else if review_mode {
        rt.block_on(run_review_only());
    } else {
        rt.block_on(async {
            // 两条独立扫描线：价格（仅交易时段）+ 消息（独立窗口）
            // 用 join! 而非 spawn（GeminiAnalyzer 含 RefCell 不满足 Send）
            tokio::join!(monitor_loop(), news_monitor_loop());
        });
    }
}

fn check_enabled() -> bool {
    std::env::var("MONITOR_ENABLED").unwrap_or_default().to_lowercase() == "true"
}

async fn push_wechat(text: &str) {
    // 通过 aiclaw send 发送微信消息。
    // aiclaw 自动读取 ~/.claude/channels/wechat/account.json + context_tokens.json，
    // 优先走 daemon HTTP API（127.0.0.1:18011），daemon 不可达时直连 ilink 协议。
    //
    // 环境变量（均可选）：
    //   AICLAW_BIN          aiclaw 二进制路径，默认 ~/Desktop/aiclaw/target/release/aiclaw
    //   WECHAT_CHANNEL_DIR  覆盖 ~/.claude/channels/wechat 目录
    //   AICLAW_API_ADDR     daemon 地址，默认 127.0.0.1:18011
    let aiclaw_bin = std::env::var("AICLAW_BIN").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{}/Desktop/aiclaw/target/release/aiclaw", home)
    });

    log::info!("[微信] 开始推送 ({}字)...", text.chars().count());

    let mut cmd = tokio::process::Command::new(&aiclaw_bin);
    cmd.arg("send").arg("--message").arg(text);

    // 透传 aiclaw 所需的可选环境变量
    if let Ok(dir) = std::env::var("WECHAT_CHANNEL_DIR") {
        cmd.arg("--data-dir").arg(&dir);
    }
    if let Ok(addr) = std::env::var("AICLAW_API_ADDR") {
        cmd.env("AICLAW_API_ADDR", addr);
    }

    match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        cmd.output(),
    ).await {
        Err(_) => log::error!("[微信] 推送超时(>30s)"),
        Ok(Ok(o)) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            if o.status.success() {
                log::info!("[微信] 推送成功 | {}", stdout.trim());
                if !stderr.trim().is_empty() {
                    log::debug!("[微信] stderr: {}", stderr.trim());
                }
            } else {
                log::error!("[微信] 推送失败 | exit={} | stdout: {} | stderr: {}",
                    o.status, stdout.trim(), stderr.trim());
            }
        }
        Ok(Err(e)) => log::error!("[微信] 推送异常 (aiclaw: {}): {}", aiclaw_bin, e),
    }
}


async fn run_test_scan() {
    log::info!("[测试] 跳过交易日历，立即执行连通性检查...");

    // 1. 扫描器初始化
    let mut targets = Vec::new();
    TieredScanner::load_positions(&mut targets);
    TieredScanner::load_watchlist(&mut targets);
    let scanner = TieredScanner::new(targets);
    log::info!("[测试] Scanner: {} 个目标", scanner.dq_summary());

    // 2. 检测器 + 状态机
    let detector = Detector::new(DetectorConfig::default());
    let mut sm = SignalStateMachine::default();

    // 3. 模拟一条数据跑全链路
    let snap = StockSnapshot {
        code: "000001".into(), name: "平安银行".into(),
        price: 10.0, change_pct: 9.8, volume_ratio: 4.0, main_net_yi: 0.6,
        limit_up_price: Some(11.0), was_limit_up: false, t1_locked: false,
    };
    let events = detector.scan_stock(&snap);
    log::info!("[测试] Detector: {} 条信号", events.len());
    let mut alerts = Vec::new();
    for e in events {
        stock_analysis::monitor::alert_log::append_jsonl(&e);
        stock_analysis::monitor::alert_log::append_md(&e);
        if let Some(ev) = sm.process(e) { alerts.push(ev); }
    }
    log::info!("[测试] 状态机: 过滤后 {} 条告警，已归档到 reports/alerts/", alerts.len());

    // 5. 风控
    use stock_analysis::monitor::risk::{PositionSizer, StopLoss, classify_market, MarketRegime};
    let regime = classify_market(0.5, 0.8);
    let sizer = PositionSizer::default();
    let sl = StopLoss::new(10.0, 3.0, Some(9.5));
    log::info!("[测试] 风控: 市场={:?} 止损={:.2} 仓位上限={:.0}",
        regime, sl.effective(), sizer.max_position(MarketRegime::Structural, 3.0, 0, 0, false));

    // 6. 信号融合
    use stock_analysis::monitor::signal_fusion::{SignalFusion, Signal, SignalSource};
    let fusion = SignalFusion::default();
    let signals = vec![
        Signal::new(SignalSource::Technical, 1.0, 80.0, 0.0),
        Signal::new(SignalSource::FundFlow, 1.0, 70.0, 0.0),
        Signal::new(SignalSource::Chain, 0.5, 60.0, 0.0),
    ];
    let resonance = fusion.resonance(&signals);
    log::info!("[测试] 信号融合: 共振={:.0} 建议={}", resonance, fusion.recommend(resonance));

    // 7. Checklist
    let positions = stock_analysis::portfolio::get_positions().unwrap_or_default();
    let _pre = checklist::build_pre_market_checklist(&positions, &[], &[]);
    log::info!("[测试] 盘前 Checklist 生成完成 ({} 只持仓)", positions.len());

    // 8. 预测
    log::info!("[测试] {}", prediction::hit_rate_summary(7));

    // 9. 自适应权重
    use stock_analysis::monitor::adaptive::AdaptiveWeightManager;
    let mut awm = AdaptiveWeightManager::default();
    awm.register_rule("test_vol_burst");
    awm.record_shadow("test_vol_burst", true);
    log::info!("[测试] 自适应权重: {} | Shadow: {}", awm.weight_summary(), awm.shadow_summary());

    // 10. 微信推送
    if !alerts.is_empty() {
        let summary = alert::aggregate_alerts(&alerts).unwrap_or_default();
        push_wechat(&summary).await;
    }

    // 11. 复盘报告（v3 新增）
    log::info!("[测试] 生成复盘报告...");
    let holdings = stock_analysis::portfolio::get_positions().unwrap_or_default();
    let report = tokio::task::spawn_blocking(move || {
        let quotes = fetch_position_quotes();
        let trades = stock_analysis::portfolio::get_trade_history(90).unwrap_or_default();
        let reviews = stock_analysis::review::journal::review_closed_trades(&trades);
        let equity = stock_analysis::portfolio::get_equity_curve(365).unwrap_or_default();
        let mut stats = stock_analysis::review::equity::compute_stats(&equity);
        stock_analysis::review::equity::enrich_with_trades(&mut stats, &reviews);
        let prices = build_price_map(&quotes);
        (stock_analysis::review::report::generate_daily_report(&reviews, &stats, &holdings, &prices), holdings)
    }).await.unwrap_or_default();
    log::info!("[测试] 复盘报告:\n{}", report.0);
    push_wechat(&report.0).await;
    let holdings = report.1;

    // 12. 净值快照（v3 新增）
    let _ = tokio::task::spawn_blocking(snapshot_portfolio_value).await;

    // 13. 产业链扫描（v3 新增）
    let opp_text = stock_analysis::opportunity::run_opportunity_scan().await;
    log::info!("[测试] 产业链扫描:\n{}", opp_text);
    push_wechat(&opp_text).await;

    // 14. v4 决策层：排除引擎 + 风控（含 HTTP 调用，走 spawn_blocking）
    let h = holdings.clone();
    let (excl_hits, violations) = tokio::task::spawn_blocking(move || {
        let watchlist = stock_analysis::portfolio::get_watchlist().unwrap_or_default();
        let excl = stock_analysis::decision::exclusion::scan_exclusions(&h, &watchlist);
        let limits = stock_analysis::risk::limits::HardLimits::default();
        let quotes = fetch_position_quotes();
        let price_map: std::collections::HashMap<String, f64> =
            quotes.iter().map(|q| (q.code.clone(), q.price)).collect();
        let viol = stock_analysis::risk::limits::check_position_limits(&h, &price_map, &limits);
        (excl, viol)
    }).await.unwrap_or_else(|_| (vec![], vec![]));
    log::info!("[测试] 排除检查: {} 项命中", excl_hits.len());
    log::info!("[测试] 风控检查: {} 项超标", violations.len());
    if !excl_hits.is_empty() {
        push_wechat(&stock_analysis::decision::exclusion::format_exclusion_alert(&excl_hits)).await;
    }
    if !violations.is_empty() {
        push_wechat(&stock_analysis::risk::limits::format_limit_alert(&violations)).await;
    }

    // 16. v4 赛道分档
    let tier_text = tokio::task::spawn_blocking(|| {
        let boards = stock_analysis::market_analyzer::sector_monitor::fetch_board_ranking("f3", 30).unwrap_or_default();
        let graded = stock_analysis::decision::sector_score::grade_sectors(&boards);
        stock_analysis::decision::sector_score::format_tier_list(&graded)
    }).await.unwrap_or_default();
    log::info!("[测试] 赛道分档:\n{}", tier_text);
    push_wechat(&tier_text).await;

    // 16.1 v4 资金验证 + v6 放量分析（复用 K 线数据，走 spawn_blocking）
    let h2 = holdings.clone();
    let (capital_text, breakout_text) = tokio::task::spawn_blocking(move || {
        let fetcher = stock_analysis::data_provider::DataFetcherManager::new().ok()?;
        let index_data = fetcher.get_daily_data("000001", 30).ok()?.0;
        let mut klines = std::collections::HashMap::new();
        for p in &h2 {
            if let Ok((data, _)) = fetcher.get_daily_data(&p.code, 60) {
                klines.insert(p.code.clone(), data);
            }
        }
        let signals = stock_analysis::decision::capital_verify::verify_holdings(&h2, &klines, &index_data);
        let cap = stock_analysis::decision::capital_verify::format_capital_signals(&signals);

        // v6 放量分析
        let mut lines = vec!["📊 放量分析（盘后·算法研判仅供参考）".to_string()];
        for p in &h2 {
            if let Some(kline) = klines.get(&p.code) {
                let sig = stock_analysis::breakout::engine::analyze_postmarket(&p.code, &p.name, kline);
                lines.push(format!(
                    "  {} {}({}) — {} 置信{}% [{}]",
                    sig.breakout_type.emoji(), sig.name, sig.code,
                    sig.breakout_type.label(), sig.confidence, sig.description,
                ));
            }
        }
        let brk = if lines.len() > 1 { Some(lines.join("\n")) } else { None };
        Some((cap, brk))
    }).await.unwrap_or_default().unwrap_or_default();

    if !capital_text.is_empty() {
        log::info!("[测试] 资金验证:\n{}", capital_text);
        push_wechat(&capital_text).await;
    }
    if let Some(ref text) = breakout_text {
        log::info!("[测试] 放量分析:\n{}", text);
        push_wechat(text).await;
    }

    // 17. v4 证伪提醒 + 周度 SOP
    let falsify_text = stock_analysis::review::falsify::daily_falsify();
    log::info!("[测试] 证伪提醒:\n{}", falsify_text);
    push_wechat(&falsify_text).await;

    if stock_analysis::review::sop::is_friday() {
        let sop_text = stock_analysis::review::sop::weekly_sop(
            holdings.len(), excl_hits.len(), violations.len(),
        );
        log::info!("[测试] 周度SOP:\n{}", sop_text);
        push_wechat(&sop_text).await;
    }

    log::info!("[测试] ======== 全链路连通性检查完成 ========");
}

/// 手动复盘：`cargo run --bin monitor -- --review`
async fn run_review_only() {
    log::info!("[复盘] 手动触发盘后分析...");

    let (report, breakout_text, risk_text) = tokio::task::spawn_blocking(|| {
        let holdings = stock_analysis::portfolio::get_positions().unwrap_or_default();
        let quotes = fetch_position_quotes();
        let prices = build_price_map(&quotes);
        let trades = stock_analysis::portfolio::get_trade_history(90).unwrap_or_default();
        let reviews = stock_analysis::review::journal::review_closed_trades(&trades);
        let equity = stock_analysis::portfolio::get_equity_curve(365).unwrap_or_default();
        let mut stats = stock_analysis::review::equity::compute_stats(&equity);
        stock_analysis::review::equity::enrich_with_trades(&mut stats, &reviews);
        let r = stock_analysis::review::report::generate_daily_report(&reviews, &stats, &holdings, &prices);
        snapshot_portfolio_value();

        // 持仓代码集合：止损/轮动只对真实持仓有意义
        let holding_codes: std::collections::HashSet<String> =
            holdings.iter().map(|p| p.code.clone()).collect();
        // 持仓成本/硬止损索引（用于止损检查）
        let holding_map: std::collections::HashMap<String, &stock_analysis::portfolio::Position> =
            holdings.iter().map(|p| (p.code.clone(), p)).collect();

        // v6 放量分析（持仓 + 自选）
        let mut brk = String::new();
        // v7 风控：收盘止损 + 轮动研判（复用已拉 K 线，零额外 HTTP）
        let mut stop_signals: Vec<stock_analysis::risk::stop_loss::StopSignal> = Vec::new();
        let mut rotation_lines: Vec<String> = Vec::new();
        let watchlist = stock_analysis::portfolio::get_watchlist().unwrap_or_default();
        let all_stocks: Vec<_> = holdings.iter().chain(watchlist.iter()).collect();
        if let Ok(fetcher) = stock_analysis::data_provider::DataFetcherManager::new() {
            let mut lines = vec!["📊 放量分析（盘后·算法研判仅供参考）".to_string()];
            for p in &all_stocks {
                if let Ok((kline, _)) = fetcher.get_daily_data(&p.code, 60) {
                    let sig = stock_analysis::breakout::engine::analyze_postmarket(&p.code, &p.name, &kline);
                    lines.push(format!(
                        "  {} {}({}) — {} 置信{}% [{}]",
                        sig.breakout_type.emoji(), sig.name, sig.code,
                        sig.breakout_type.label(), sig.confidence, sig.description,
                    ));

                    // 仅对持仓做风控检查
                    if holding_codes.contains(&p.code) {
                        // 现价：缺失则跳过止损（不静默用 0 价触发假硬止损 — AGENTS.md 2.2）
                        match prices.get(&p.code) {
                            Some(&cur) if cur > 0.0 => {
                                let ma20 = compute_ma(&kline, 20);
                                let ma60 = compute_ma(&kline, 60);
                                if let Some(pos) = holding_map.get(&p.code) {
                                    let mut sigs = stock_analysis::risk::stop_loss::check_stops(
                                        &p.code, &p.name, cur, pos.cost_price, pos.hard_stop, ma20, ma60,
                                    );
                                    stop_signals.append(&mut sigs);
                                }
                            }
                            _ => log::warn!("[复盘] {}({}) 现价缺失，跳过止损检查", p.name, p.code),
                        }
                        // 轮动研判（健康回调 vs 趋势结束）
                        let rot = stock_analysis::decision::rotation::judge_trend(&kline);
                        rotation_lines.push(format!(
                            "  {} {}({}) — {} [{}]",
                            rot.status.emoji(), p.name, p.code,
                            rot.status.label(), rot.reasons.join("·"),
                        ));
                    }
                }
            }
            if lines.len() > 1 { brk = lines.join("\n"); }
        }

        // 组装风控文本：止损告警 + 轮动研判
        let mut risk = String::new();
        let stop_text = stock_analysis::risk::stop_loss::format_stop_alerts(&stop_signals);
        if !stop_text.is_empty() {
            risk.push_str(&stop_text);
        }
        if !rotation_lines.is_empty() {
            if !risk.is_empty() { risk.push_str("\n\n"); }
            risk.push_str("🔄 持仓轮动研判（算法·仅供参考）\n");
            risk.push_str(&rotation_lines.join("\n"));
        }
        (r, brk, risk)
    }).await.unwrap_or_default();

    log::info!("[复盘] 复盘报告:\n{}", report);
    push_wechat(&report).await;

    if !breakout_text.is_empty() {
        log::info!("[复盘] 放量分析:\n{}", breakout_text);
        push_wechat(&breakout_text).await;
    }

    if !risk_text.is_empty() {
        log::info!("[复盘] 风控研判:\n{}", risk_text);
        push_wechat(&risk_text).await;
    }

    let falsify_text = stock_analysis::review::falsify::daily_falsify();
    log::info!("[复盘] 证伪提醒:\n{}", falsify_text);
    push_wechat(&falsify_text).await;

    log::info!("[复盘] ======== 盘后分析完成 ========");
}

/// 消息监控独立循环 —— 不受交易日/交易时段限制。
/// 窗口：盘前08:00-09:30、盘中09:30-15:00、盘后15:00-22:00。
async fn news_monitor_loop() {
    use stock_analysis::monitor::detector::{AlertEvent, AlertLevel};
    use stock_analysis::monitor::news_monitor::NewsMonitor;
    use stock_analysis::monitor::news_ai::NewsAIAnalyzer;
    use stock_analysis::monitor::signal_state::SignalStateMachine;

    let poll_secs: u64 = std::env::var("NEWS_POLL_INTERVAL")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(120);

    log::info!("[NewsMonitor] 启动（独立窗口，不随价格扫描器静默）");
    let mut nm = NewsMonitor::new();
    nm.restore_dedup();
    let mut ai = NewsAIAnalyzer::new();
    let mut sm = SignalStateMachine::default();
    sm.restore_state();
    let mut last_concept_refresh = std::time::Instant::now();
    let mut last_flush = std::time::Instant::now();
    // 产业链机会发现调度：None=启动后首轮立即跑，之后按 opportunity_scan_interval_min 间隔
    // 统一在本 8:00-22:00 窗口内调度（覆盖盘前/盘中/盘后），消除「收盘即停」盲区。
    let mut last_opp_scan: Option<std::time::Instant> = None;

    // 收集我们的标的代码（供L2概念匹配）
    let our_codes: std::collections::HashSet<String> = {
        let mut set: std::collections::HashSet<String> = stock_analysis::portfolio::get_all_codes()
            .unwrap_or_default().into_iter().collect();
        for code in nm.linker_ref().registered_codes() {
            set.insert(code.to_string());
        }
        set
    };
    log::info!("[NewsMonitor] L2 标的池: {} 只", our_codes.len());

    loop {
        if !NewsMonitor::should_run() {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            continue;
        }

        // L2 概念索引刷新（每5分钟一次）
        if last_concept_refresh.elapsed().as_secs() >= 300 {
            last_concept_refresh = std::time::Instant::now();
            let codes = our_codes.clone();
            match tokio::task::spawn_blocking(move || {
                // 同步HTTP在独立线程执行，不触发 runtime 冲突
                stock_analysis::monitor::news_monitor::refresh_concept_index_blocking(&codes)
            }).await {
                Ok(Some(index)) => {
                    nm.linker_mut().replace_concept_index(index);
                    log::info!("[NewsMonitor] L2 概念索引已更新（{}个板块关联）", nm.linker_ref().concept_count());
                }
                Ok(None) => log::warn!("[NewsMonitor] L2 概念索引刷新跳过（无板块数据）"),
                Err(_) => log::warn!("[NewsMonitor] L2 概念索引刷新 panic"),
            }
        }

        // 公告扫描（仅网络拉取在 spawn_blocking，处理在主线程）
        let anns = tokio::task::spawn_blocking(|| {
            stock_analysis::data_provider::announcement::fetch_announcements(None)
                .unwrap_or_default()
        }).await.unwrap_or_else(|_| vec![]);

        // 异步预解析：公告API缺失code时，通过东方财富搜索反查
        let mut resolved_codes: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        {
            let http = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build().unwrap_or_default();
            for ann in &anns {
                if ann.code.is_empty() && !ann.name.is_empty() {
                    // 先查本地缓存
                    if let Some(code) = nm.linker_ref().lookup_code_by_name(&ann.name) {
                        resolved_codes.insert(ann.name.clone(), code.to_string());
                    } else if let Some(code) = stock_analysis::monitor::news_monitor::resolve_code_by_name(&ann.name, &http).await {
                        log::info!("[NewsMonitor] 反查 {} → {}", ann.name, code);
                        resolved_codes.insert(ann.name.clone(), code);
                    }
                }
            }
        }
        let events = nm.process_announcements(&anns, &resolved_codes);
        let mut pushed: Vec<AlertEvent> = Vec::new();
        for e in events {
            stock_analysis::monitor::alert_log::append_jsonl(&e);
            stock_analysis::monitor::alert_log::append_md(&e);
            if let Some(ev) = sm.process(e) {
                push(&ev).await;
                pushed.push(ev);
            }
        }
        // 🚀 实时层：对重要公告，AI 追推一句话决策
        for ev in &pushed {
            if ev.level <= AlertLevel::Important
                && !ev.name.is_empty()
                && ev.name != "RISK"
            {
                let title = ev.detail.news_title.as_deref().unwrap_or(&ev.message);
                let code = if ev.code.is_empty() { ev.name.as_str() } else { &ev.code };
                log::info!("[NewsAI] 🚀实时层 开始为 {} 生成决策...", ev.name);
                match ai.quick_decision(title, code, &ev.name).await {
                    Some(decision) => {
                        let follow = format!(
                            "🧠 {} AI研判：{}【AI研判-仅供参考】",
                            ev.name, decision
                        );
                        push_wechat(&follow).await;
                        log::info!("[NewsAI] {} 实时决策已推送", ev.name);
                    }
                    None => {
                        log::warn!("[NewsAI] {} 实时决策生成失败（超时/AI不可用）", ev.name);
                    }
                }
            }
        }

        // ⚡ 快研层：Important+ 事件，顺序深度分析（每只~5s，120s轮询间隔足够）
        for ev in &pushed {
            if ev.level <= AlertLevel::Important
                && !ev.code.is_empty()
                && ev.code != "RISK"
            {
                let news_text = ev.detail.news_summary
                    .clone()
                    .unwrap_or_else(|| ev.message.clone());
                log::info!("[NewsAI] ⚡快研层 开始分析 {}({})...", ev.name, ev.code);
                match ai.analyze_position_news(
                    &ev.code, &ev.name, &news_text,
                    0.0, 0.0, 0.0, 0.0,  // 默认值（快研层侧重消息面）
                    "未知", 0.0, "未知", "未知", 0.0,
                ).await {
                    Some(deep) => {
                        let prefix = if ev.level == AlertLevel::Emergency { "🔬" } else { "🔍" };
                        let follow = format!(
                            "{} {}({}) 快研补充：\n{}",
                            prefix, ev.name, ev.code,
                            deep.message
                        );
                        push_wechat(&follow).await;
                        log::info!("[NewsAI] {} 快研已推送", ev.name);
                    }
                    None => {
                        log::warn!("[NewsAI] {} 快研失败（超时/AI不可用）", ev.name);
                    }
                }
            }
        }

        // 路径A 机会发现已统一到 opportunity::run_opportunity_scan（monitor_loop 内调度），
        // 此处不再重复跑 news_ai::discover_opportunities（v8 单一发现器，消除重复路径）。

        // 产业链机会扫描：统一在 8:00-22:00 窗口内按间隔调度（覆盖盘前/盘中/盘后）。
        // spawn 异步执行，不阻塞新闻轮询。
        let opp_interval_secs = stock_analysis::config::get_monitor_config()
            .opportunity_scan_interval_min * 60;
        let opp_due = last_opp_scan
            .map(|t| t.elapsed().as_secs() >= opp_interval_secs)
            .unwrap_or(true);
        if opp_due {
            last_opp_scan = Some(std::time::Instant::now());
            tokio::spawn(async move {
                let opp_text = stock_analysis::opportunity::run_opportunity_scan().await;
                // 仅在有实际机会时推送；空结果（暂无快讯/未命中/无可用标的）只记日志不刷屏。
                if !opp_text.contains("暂无") && !opp_text.contains("无可用标的") && !opp_text.contains("未命中") {
                    log::info!("[产业链] {}", opp_text);
                    push_wechat(&opp_text).await;
                }
            });
        }

        // 每日重置
        let today = chrono::Local::now().format("%Y%m%d").to_string();
        {
            use std::sync::Mutex;
            static LAST_DATE: Mutex<Option<String>> = Mutex::new(None);
            let mut last = LAST_DATE.lock().unwrap();
            if last.as_deref() != Some(&today) {
                sm.daily_reset();
                *last = Some(today);
            }
        }

        // v5: 每 5 分钟刷盘
        if last_flush.elapsed().as_secs() >= 300 {
            last_flush = std::time::Instant::now();
            nm.flush_dedup();
            sm.flush_state();
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(poll_secs)).await;
    }
}

async fn monitor_loop() {
    // 全天候循环：非交易日等待，交易日自动进入扫描
    loop {
        if !calendar::today_is_trading_day() {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            continue;
        }

        while !is_market_active() {
            let session = calendar::session_label();
            if session.contains("休市") || session.contains("盘后") {
                // 还在盘前等待窗口
            }
            log::info!("等待交易时段... 当前: {}", session);
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            if !calendar::today_is_trading_day() { break; }
        }

        if !calendar::today_is_trading_day() { continue; }

        log::info!("进入交易时段，开始监控");

        let positions = stock_analysis::portfolio::get_positions().unwrap_or_default();
        let t1_unlocks: Vec<_> = positions.iter()
            .filter(|p| stock_analysis::portfolio::is_t1_locked(&p.code))
            .cloned().collect();
        let pre_market = checklist::build_pre_market_checklist(&positions, &t1_unlocks, &[]);
        log::info!("[盘前] {} 只持仓，{} 只解禁", positions.len(), t1_unlocks.len());

        push_wechat(&pre_market).await;

        prediction::verify_predictions();
        let hit_rate = prediction::recent_hit_rate(7);
        if hit_rate > 0.0 { log::info!("[预测] 近7天命中率: {:.0}%", hit_rate * 100.0); }

        let mut targets = Vec::new();
        TieredScanner::load_positions(&mut targets);
        TieredScanner::load_watchlist(&mut targets);
        // 构建实体过滤集合（只关注9只标的）
        let our_codes: std::collections::HashSet<String> = targets.iter().map(|t| t.code.clone()).collect();
        let scanner = TieredScanner::new(targets);

        let detector = Detector::new(DetectorConfig::default());
        let mut state_machine = SignalStateMachine::default();
        state_machine.restore_state();
        let mut signal_count = 0u32;
        let mut alert_count = 0u32;
        let mut total_limit_ups: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut total_limit_downs: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut total_board_breaks = 0u32;
        let poll_secs: u64 = std::env::var("MONITOR_HOLDING_INTERVAL")
            .ok().and_then(|s| s.parse().ok()).unwrap_or(30);
        // Phase 1.1 量化标准：信号融合 + 风险叠加 + 状态驱动
        use stock_analysis::monitor::signal_fusion::{Signal, SignalFusion, SignalSource};
        let fusion = SignalFusion::default();
        // 三个独立计时器
        let mut last_sector_push = std::time::Instant::now();    // 领涨板块（5分钟）
        let mut last_health_summary = std::time::Instant::now(); // 持仓健康度（5分钟）
        let mut last_screener_run = std::time::Instant::now();   // 选股推荐（30分钟）
        // 产业链扫描已移至 news_monitor_loop 的 8:00-22:00 窗口统一调度。
        let mut was_limit_up: std::collections::HashSet<String> = std::collections::HashSet::new();

        loop {
            let session = current_session();

            if session == MarketSession::Auction {
                log::info!("[竞价] 09:25 扫描...");
                let stocks = tokio::task::spawn_blocking(|| {
                    let analyzer = stock_analysis::market_analyzer::MarketAnalyzer::new(None).ok()?;
                    analyzer.get_limit_up_stocks().ok()
                }).await.unwrap_or(None);
                if let Some(stocks) = stocks {
                    for s in stocks.iter().take(10) {
                        if !our_codes.contains(&s.code) { continue; }
                        let snap = StockSnapshot {
                            code: s.code.clone(), name: s.name.clone(),
                            price: s.price, change_pct: s.change_pct,
                            volume_ratio: 0.0, main_net_yi: 0.0,
                            limit_up_price: None, was_limit_up: false, t1_locked: false,
                        };
                        for e in detector.scan_stock(&snap) {
                            signal_count += 1;
                            if let Some(event) = state_machine.process(e) {
                                alert_count += 1;
                                push(&event).await;
                            }
                        }
                    }
                }
            }

            if session == MarketSession::Morning || session == MarketSession::Afternoon {
                let result = tokio::task::spawn_blocking(|| {
                    let analyzer = stock_analysis::market_analyzer::MarketAnalyzer::new(None).ok()?;
                    let limit_stocks = analyzer.get_limit_up_stocks().ok().unwrap_or_default();
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    let position_quotes = fetch_position_quotes();
                    Some((limit_stocks, position_quotes))
                }).await.unwrap_or(None);

                if let Some((limit_stocks, position_quotes)) = result {
                    // 合并两路数据：涨停列表中的持仓 + 持仓单独查询
                    let mut stock_map: std::collections::HashMap<String, &stock_analysis::market_data::TopStock> = std::collections::HashMap::new();
                    for s in &limit_stocks { if our_codes.contains(&s.code) { stock_map.insert(s.code.clone(), s); } }
                    for q in &position_quotes { if !stock_map.contains_key(&q.code) { stock_map.insert(q.code.clone(), q); } }

                    // 主力排名（仅涨停股中排序）
                    let mut ranked: Vec<&stock_analysis::market_data::TopStock> = limit_stocks.iter().collect();
                    ranked.sort_by(|a, b| b.main_net_yi.partial_cmp(&a.main_net_yi).unwrap_or(std::cmp::Ordering::Equal));
                    let total_ranked = ranked.len();

                    // 持仓遍历：信号融合（不再单独推送每条事件）
                    let mut health_lines: Vec<String> = Vec::new();
                    for (code, s) in &stock_map {
                        let t1_locked = stock_analysis::portfolio::is_t1_locked(code);
                        let rank = ranked.iter().position(|r| r.code == *code).map(|p| p + 1);
                        let is_limit_up = s.change_pct >= 9.5;
                        let prev_was_limit = was_limit_up.contains(code);

                        // 状态追踪
                        if is_limit_up { was_limit_up.insert(code.clone()); }
                        else { was_limit_up.remove(code); }

                        let snap = StockSnapshot {
                            code: s.code.clone(), name: s.name.clone(),
                            price: s.price, change_pct: s.change_pct,
                            volume_ratio: s.volume_ratio, main_net_yi: s.main_net_yi,
                            limit_up_price: Some(s.price * 1.1),
                            was_limit_up: prev_was_limit, t1_locked,
                        };

                        // 信号收集 + 突变检测
                        let mut signals: Vec<Signal> = Vec::new();
                        let mut emergency_note = String::new();
                        for e in detector.scan_stock(&snap) {
                            signal_count += 1;
                            let (dir, strength) = match e.category {
                                AlertCategory::LimitUp | AlertCategory::MainInflow => (1.0, 80.0),
                                AlertCategory::LimitDown | AlertCategory::MainOutflow => (-1.0, 80.0),
                                AlertCategory::VolBurst => (1.0, 60.0),
                                AlertCategory::BoardBreak => (-1.0, 90.0),
                                _ => (0.0, 40.0),
                            };
                            signals.push(Signal::new(
                                match e.category {
                                    AlertCategory::MainInflow | AlertCategory::MainOutflow => SignalSource::FundFlow,
                                    _ => SignalSource::Technical,
                                },
                                dir, strength, 0.0,
                            ));
                            // 突变检测：仅记录状态，不单独推送
                            if matches!(e.category, AlertCategory::BoardBreak) {
                                emergency_note = "⚠️ 炸板！".to_string();
                            }
                        }

                        // 信号融合
                        let resonance = if signals.is_empty() { 0.0 } else { fusion.resonance(&signals) };
                        let recommend = fusion.recommend(resonance);

                        // 累计当日数据（供收盘总结）
                        if is_limit_up { total_limit_ups.insert(code.clone()); }
                        if s.change_pct <= -9.5 { total_limit_downs.insert(code.clone()); }
                        if prev_was_limit && !is_limit_up { total_board_breaks += 1; }

                        // 涨停/跌停突变一次推送（走状态机防重复）
                        if is_limit_up || s.change_pct <= -9.5 {
                            let event = AlertEvent {
                                level: if s.change_pct <= -9.5 { AlertLevel::Emergency } else { AlertLevel::Important },
                                category: if s.change_pct <= -9.5 { AlertCategory::LimitDown } else { AlertCategory::LimitUp },
                                code: code.clone(), name: s.name.clone(),
                                message: if s.change_pct <= -9.5 {
                                    format!("{} 跌停 {:.1}%", s.name, s.change_pct)
                                } else {
                                    format!("{} 涨停 {:.1}%", s.name, s.change_pct)
                                },
                                detail: AlertDetail {
                                    price: Some(s.price), change_pct: Some(s.change_pct),
                                    volume_ratio: Some(s.volume_ratio),
                                    main_flow_yi: Some(s.main_net_yi),
                                    threshold: None, news_title: None,
                                    news_summary: None, ai_decision: None,
                                    t1_locked,
                                    extra: rank.map(|r| format!("主力排名 {}/{} | 共振{:.0} {}", r, total_ranked, resonance, recommend)),
                                },
                                triggered_at: chrono::Local::now(),
                            };
                            if let Some(ev) = state_machine.process(event) {
                                alert_count += 1;
                                push(&ev).await;
                            }
                        }
                        // 炸板立即推送（Emergency，无限冷却）
                        if !emergency_note.is_empty() {
                            push_wechat(&format!("🔴 {}({}) {}", s.name, code, emergency_note)).await;
                        }

                        // 健康度记录（每5分钟推送汇总）
                        let note = if t1_locked { "🔒锁仓" }
                            else if is_limit_up { "🔺涨停" }
                            else if s.change_pct <= -5.0 { "🔻" }
                            else if resonance > 60.0 { "📈" }
                            else if resonance < -30.0 { "📉" }
                            else { "→" };
                        health_lines.push(format!(
                            "  {:<6} {}({}) {:>+.1}% ¥{:2} {}",
                            note, s.name, code, s.change_pct, s.price,
                            if resonance.abs() > 5.0 { format!("共振{:0}", resonance) } else { String::new() }
                        ));
                        if resonance.abs() > 30.0 {
                            log::info!("[信号融合] {}({}) 共振={:0} 建议={}", s.name, code, resonance, recommend);
                        }
                    }

                    // 每5分钟推送持仓健康度汇总
                    if last_health_summary.elapsed().as_secs() >= 300 && !health_lines.is_empty() {
                        last_health_summary = std::time::Instant::now();
                        let mut summary = vec![format!("📊 持仓健康度 ({})", chrono::Local::now().format("%H:%M"))];
                        summary.append(&mut health_lines);
                        push_wechat(&summary.join("\n")).await;
                    }

                    // 选股推荐（独立计时器，每30分钟）
                    let cfg = stock_analysis::config::get_monitor_config();
                    if last_screener_run.elapsed().as_secs() >= cfg.screener_interval_min * 60 {
                        last_screener_run = std::time::Instant::now();
                        log::info!("[选股] 开始盘中选股扫描...");
                        let recs = tokio::task::spawn_blocking(run_stock_screener).await.unwrap_or(None);
                        if let Some(ref recs) = recs {
                            for rec in recs { log::info!("[选股] {}", rec); push_wechat(rec).await; }
                        }
                    }

                    // 产业链扫描已统一到 news_monitor_loop 的 8:00-22:00 窗口调度，
                    // 此处不再重复（避免盘中 monitor_loop 与 news_monitor_loop 双跑双推）。

                    // 领涨板块（独立计时器，每5分钟）
                    if last_sector_push.elapsed().as_secs() >= 300 {
                        last_sector_push = std::time::Instant::now();
                        push_sector_leaders().await;
                    }
                }
            }

            if session == MarketSession::AfterHours { break; }
            if session == MarketSession::LunchBreak {
                log::info!("[午休] 暂停扫描");
                tokio::time::sleep(tokio::time::Duration::from_secs(90 * 60)).await;
                continue;
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(poll_secs)).await;
        }

        // 拉上证指数（新浪 API）：阻塞 I/O 放到 blocking 线程，避免在 async 上下文创建/销毁 blocking runtime。
        let index_change = tokio::task::spawn_blocking(fetch_sh_index_change)
            .await
            .unwrap_or(0.0);
        let up_count = total_limit_ups.len();
        let down_count = total_limit_downs.len();
        let board_break_rate = if up_count > 0 { total_board_breaks as f64 / up_count as f64 * 100.0 } else { 0.0 };
        let summary = checklist::build_close_summary(
            index_change, up_count, down_count, board_break_rate,
            signal_count as usize, alert_count as usize, &t1_unlocks,
        );
        push_wechat(&summary).await;

        // v3 复盘报告
        let trades = stock_analysis::portfolio::get_trade_history(90).unwrap_or_default();
        let reviews = stock_analysis::review::journal::review_closed_trades(&trades);
        let equity = stock_analysis::portfolio::get_equity_curve(365).unwrap_or_default();
        let mut stats = stock_analysis::review::equity::compute_stats(&equity);
        stock_analysis::review::equity::enrich_with_trades(&mut stats, &reviews);
        let holdings = stock_analysis::portfolio::get_positions().unwrap_or_default();
        let prices = tokio::task::spawn_blocking(|| {
            let quotes = fetch_position_quotes();
            build_price_map(&quotes)
        })
        .await
        .unwrap_or_default();
        let review_report = stock_analysis::review::report::generate_daily_report(&reviews, &stats, &holdings, &prices);
        push_wechat(&review_report).await;

        // v3 每日净值快照
        let _ = tokio::task::spawn_blocking(snapshot_portfolio_value).await;

        log::info!("[收盘] 信号{}条 告警{}条 | DQ: {} | {}",
            signal_count, alert_count, scanner.dq_summary(), prediction::hit_rate_summary(7));
        // 收盘后继续循环，等待下一个交易日
    }
}

/// Phase 4.1 选股推荐：点火广度排序 + 成份股过滤
fn run_stock_screener() -> Option<Vec<String>> {
    use stock_analysis::market_analyzer::sector_monitor;
    use stock_analysis::breakout::engine::screen_intraday;

    let our_codes: std::collections::HashSet<String> = stock_analysis::portfolio::get_all_codes()
        .unwrap_or_default().into_iter().collect();

    // 1. 拉涨幅前 30 板块（失败→本轮无推荐，不刷屏）
    let boards = sector_monitor::fetch_board_ranking("f3", 30).ok()?;

    // 2. 收集候选标的（逐板块拉成份股，命中足够候选即提前停止，避免预拉全部 30 板块）
    //    候选携带其所属板块名 + 板块点火广度，供 breakout 盘中模式打分。
    const MAX_CANDIDATES: usize = 20; // 限制批量报价规模，控制 HTTP 成本
    struct Candidate { code: String, name: String, board: String, near_limit: usize }
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for b in boards.iter() {
        let comps = match sector_monitor::fetch_board_components(&b.code, 30) {
            Ok(c) => c,
            Err(_) => continue, // 该板块拉取失败→跳过，不中断
        };
        let ignition = sector_monitor::compute_ignition(&comps);
        for s in comps.iter() {
            if our_codes.contains(&s.code) { continue; }
            if s.code.starts_with('8') || s.code.starts_with('4') || s.code.starts_with("688") { continue; }
            if s.name.contains("ST") || s.name.contains("退") { continue; }
            if s.change_pct > 9.5 { continue; } // 已涨停不追
            if !seen.insert(s.code.clone()) { continue; }
            candidates.push(Candidate {
                code: s.code.clone(), name: s.name.clone(),
                board: b.name.clone(), near_limit: ignition.near_limit_count,
            });
            if candidates.len() >= MAX_CANDIDATES { break; }
        }
        if candidates.len() >= MAX_CANDIDATES { break; }
    }
    if candidates.is_empty() { return None; }

    // 3. 批量拉候选资金面（一次 HTTP）。失败→资金面留空，breakout 标记数据降级（不伪造）。
    let codes: Vec<String> = candidates.iter().map(|c| c.code.clone()).collect();
    let quote_map: std::collections::HashMap<String, stock_analysis::market_data::TopStock> =
        match fetch_eastmoney_quotes(&codes) {
            Ok(qs) => qs.into_iter().map(|q| (q.code.clone(), q)).collect(),
            Err(e) => { log::warn!("[选股] 候选资金面拉取失败，按数据降级处理: {}", e); std::collections::HashMap::new() }
        };

    // 4. breakout 盘中模式逐个打分
    let mut signals: Vec<(stock_analysis::breakout::signal::BreakoutSignal, String)> = Vec::new();
    for c in &candidates {
        let (vol_ratio, change_pct, main_net_yi) = match quote_map.get(&c.code) {
            Some(q) => (q.volume_ratio, q.change_pct, q.main_net_yi),
            None => (0.0, 0.0, 0.0), // 数据降级：screen_intraday 内部会置 data_degraded
        };
        let sig = screen_intraday(&c.code, &c.name, vol_ratio, change_pct, main_net_yi, c.near_limit);
        signals.push((sig, c.board.clone()));
    }

    // 5. 按置信度降序，取置信度达阈值（≥20）的 Top 3
    signals.sort_by(|a, b| b.0.confidence.cmp(&a.0.confidence));
    let recs: Vec<String> = signals.iter()
        .filter(|(s, _)| s.confidence >= 20)
        .take(3)
        .map(|(s, board)| {
            format!(
                "{} 选股推荐 | {}({}) | 板块:{} | 涨幅:{:.1}% | 置信度:{} | {}",
                s.breakout_type.emoji(), s.name, s.code, board, s.change_pct,
                s.confidence, s.description
            )
        }).collect();

    if recs.is_empty() { None } else { Some(recs) }
}

/// 持仓实时行情：东财 push2 为主（多主机轮询），新浪兜底
fn fetch_position_quotes() -> Vec<stock_analysis::market_data::TopStock> {
    let codes: Vec<String> = stock_analysis::portfolio::get_all_codes().unwrap_or_default();
    if codes.is_empty() { return vec![]; }

    match fetch_eastmoney_quotes(&codes) {
        Ok(q) if !q.is_empty() => q,
        _ => fetch_sina_quotes(&codes),
    }
}

/// 东方财富 push2 实时行情（多主机轮询，含 volume_ratio + main_net_yi）
fn fetch_eastmoney_quotes(codes: &[String]) -> Result<Vec<stock_analysis::market_data::TopStock>, String> {
    use stock_analysis::market_data::TopStock;
    // secids: 0.000547,1.603618 (0=深交所,1=上交所)
    let secids: Vec<String> = codes.iter().map(|c| {
        if c.starts_with('6') || c.starts_with('5') { format!("1.{}", c) }
        else { format!("0.{}", c) }
    }).collect();
    let url_path = format!("/api/qt/ulist.np/get?secids={}&fields=f2,f3,f10,f12,f14,f62&fltt=2&invt=2",
        secids.join(","));

    const HOSTS: &[&str] = &["push2delay.eastmoney.com", "push2.eastmoney.com", "82.push2.eastmoney.com"];
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build().map_err(|e| e.to_string())?;

    for host in HOSTS {
        let url = format!("https://{}{}", host, url_path);
        let resp = client.get(&url)
            .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .header("Referer", "https://quote.eastmoney.com/")
            .send();
        match resp.and_then(|r| r.json::<serde_json::Value>()) {
            Ok(json) => {
                if let Some(arr) = json.get("data").and_then(|d| d.get("diff")).and_then(|d| d.as_array()) {
                    let stocks: Vec<TopStock> = arr.iter().filter_map(|item| {
                        Some(TopStock {
                            code: item.get("f12")?.as_str()?.to_string(),
                            name: item.get("f14")?.as_str()?.to_string(),
                            price: item.get("f2").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            change_pct: item.get("f3").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            volume_ratio: item.get("f10").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            main_net_yi: item.get("f62").and_then(|v| v.as_f64()).unwrap_or(0.0) / 1e8,
                        })
                    }).collect();
                    if !stocks.is_empty() { return Ok(stocks); }
                }
            }
            Err(_) => continue,
        }
    }
    Err("所有东财主机请求失败".into())
}

/// 新浪行情 API：免费、稳定、无频率限制、无需 Referer/Cookie/Token。
/// URL: http://hq.sinajs.cn/list=sz000547,sh603618
/// 返回: var hq_str_sz000547="名称,今开,昨收,现价,最高,最低,..."
fn fetch_sina_quotes(codes: &[String]) -> Vec<stock_analysis::market_data::TopStock> {
    use stock_analysis::market_data::TopStock;
    // 新浪 A 股符号映射：深交所 sz，上交所(6/5开头) sh
    let symbols: Vec<String> = codes.iter().map(|c| {
        if c.starts_with('6') || c.starts_with('5') { format!("sh{}", c) }
        else { format!("sz{}", c) }
    }).collect();
    let url = format!("http://hq.sinajs.cn/list={}", symbols.join(","));

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build() { Ok(c) => c, Err(_) => return vec![] };

    let text = match client.get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .header("Referer", "https://finance.sina.com.cn/")
        .send().and_then(|r| r.text()) // 新浪返回 GBK 文本，reqwest 自动解码
    { Ok(t) => t, Err(e) => { log::warn!("[新浪行情] 请求失败: {}", e); return vec![]; } };

    // 逐行解析：var hq_str_sz000547="名称,今开,昨收,...";
    let mut results = Vec::new();
    for (symbol, code) in symbols.iter().zip(codes.iter()) {
        // 从文本中提取该股票的数据行
        let prefix = format!("var hq_str_{}=\"", symbol);
        let start = match text.find(&prefix) { Some(p) => p + prefix.len(), None => continue };
        let end = match text[start..].find('"') { Some(p) => start + p, None => continue };
        let data = &text[start..end];
        let fields: Vec<&str> = data.split(',').collect();
        if fields.len() < 4 { continue; }

        let name = fields[0].to_string();
        let prev_close: f64 = fields.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let price: f64 = fields.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let change_pct = if prev_close > 0.0 { (price - prev_close) / prev_close * 100.0 } else { 0.0 };

        results.push(TopStock {
            code: code.clone(), name,
            price, change_pct,
            volume_ratio: 0.0,   // 新浪不提供量比
            main_net_yi: 0.0,    // 新浪不提供主力净流入
        });
    }
    results
}

/// 拉取上证指数涨跌幅（新浪 API）
fn fetch_sh_index_change() -> f64 {
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build() { Ok(c) => c, Err(_) => return 0.0 };
    let text = match client.get("http://hq.sinajs.cn/list=s_sh000001")
        .header("User-Agent", "Mozilla/5.0")
        .header("Referer", "https://finance.sina.com.cn/")
        .send().and_then(|r| r.text())
    { Ok(t) => t, Err(_) => return 0.0 };
    // 格式：var hq_str_s_sh000001="上证指数,3267.19,3258.86,..."
    if let Some(start) = text.find('"') {
        if let Some(end) = text[start+1..].find('"') {
            let data = &text[start+1..start+1+end];
            let fields: Vec<&str> = data.split(',').collect();
            // fields[1]=当前价, fields[2]=昨收
            if fields.len() >= 3 {
                let price: f64 = fields[1].parse().unwrap_or(0.0);
                let prev: f64 = fields[2].parse().unwrap_or(0.0);
                if prev > 0.0 { return (price - prev) / prev * 100.0; }
            }
        }
    }
    0.0
}

/// 领涨板块推送
async fn push_sector_leaders() {
    let boards = tokio::task::spawn_blocking(|| {
        stock_analysis::market_analyzer::sector_monitor::fetch_board_ranking("f3", 5)
    }).await.unwrap_or(Ok(vec![])).unwrap_or_default();

    if boards.is_empty() { return; }
    let mut lines = vec!["📊 领涨板块 Top 5".to_string()];
    let medals = ["🥇", "🥈", "🥉", "4️⃣", "5️⃣"];
    for (i, b) in boards.iter().enumerate() {
        let inflow_yi = b.main_inflow / 1e8;
        lines.push(format!("  {} {} {:+.1}% 主力{:.1}亿",
            medals[i.min(4)], b.name, b.change_pct, inflow_yi));
    }
    push_wechat(&lines.join("\n")).await;
}

async fn push(event: &AlertEvent) {
    let text = alert::format_alert(event);
    log::info!("[告警] {} {} → {}", event.level.emoji(), event.code, event.message);
    stock_analysis::monitor::alert_log::append_jsonl(event);
    stock_analysis::monitor::alert_log::append_md(event);
    push_wechat(&text).await;
}

fn build_price_map(quotes: &[stock_analysis::market_data::TopStock]) -> std::collections::HashMap<String, f64> {
    quotes.iter().map(|q| (q.code.clone(), q.price)).collect()
}

/// 计算最近 n 日收盘均价；K 线不足 n 条返回 None（避免误导结构止损 / 轮动判断）
fn compute_ma(kline: &[stock_analysis::data_provider::KlineData], n: usize) -> Option<f64> {
    if n == 0 || kline.len() < n { return None; }
    let sum: f64 = kline.iter().rev().take(n).map(|k| k.close).sum();
    Some(sum / n as f64)
}

/// v3: 收盘时记录净值快照到 ledger 表
fn snapshot_portfolio_value() {
    let positions = match stock_analysis::portfolio::get_positions() {
        Ok(p) => p,
        Err(e) => { log::warn!("[净值快照] 获取持仓失败: {}", e); return; }
    };
    if positions.is_empty() { return; }

    let quotes = fetch_position_quotes();
    let mut quote_map: std::collections::HashMap<&str, f64> = std::collections::HashMap::new();
    for q in &quotes { quote_map.insert(q.code.as_str(), q.price); }

    let mut total_value = 0.0_f64;
    let mut counted = 0;
    for p in &positions {
        let price = quote_map.get(p.code.as_str()).copied().unwrap_or(p.cost_price);
        total_value += p.shares as f64 * price;
        counted += 1;
    }

    let prev_value = stock_analysis::portfolio::get_equity_curve(2).ok()
        .and_then(|c| c.last().map(|e| e.total_value))
        .unwrap_or(total_value);
    let daily_pnl = total_value - prev_value;

    let entry = stock_analysis::portfolio::LedgerEntry {
        date: chrono::Local::now().date_naive(),
        total_value,
        cash: 0.0,
        market_value: total_value,
        daily_pnl,
    };
    match stock_analysis::portfolio::snapshot_ledger(entry) {
        Ok(()) => log::info!("[净值快照] 总市值 ¥{:.0} ({}/{} 只) 日盈亏 {:+.0}",
            total_value, counted, positions.len(), daily_pnl),
        Err(e) => log::warn!("[净值快照] 保存失败: {}", e),
    }
}
