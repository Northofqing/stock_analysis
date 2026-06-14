//! 实盘监控模式入口。
//!
//! 用法：
//!   cargo run --bin monitor             # 正常监控（等交易日+交易时段）
//!   cargo run --bin monitor -- --test   # 测试模式（跳过日历，立即跑一次扫描验证）
//!
//! 依赖 .env 中 MONITOR_ENABLED=true

use std::io::Write;
use stock_analysis::calendar::{self, current_session, is_market_active, MarketSession};
use stock_analysis::monitor::detector::{AlertEvent, Detector, DetectorConfig, StockSnapshot};
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
    // 初始化数据库（后续持仓查询/预测追踪需要）
    let db_path = std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/stock_analysis.db".into());
    let _ = stock_analysis::database::DatabaseManager::init(Some(std::path::PathBuf::from(&db_path)));
    let test_mode = std::env::args().any(|a| a == "--test");

    log::info!("实盘监控启动 | {} | 当前: {} | 模式: {}",
        if calendar::today_is_trading_day() { "交易日" } else { "非交易日" },
        calendar::session_label(),
        if test_mode { "测试" } else { "正常" },
    );

    let rt = tokio::runtime::Runtime::new().expect("创建 tokio runtime 失败");
    if test_mode {
        rt.block_on(run_test_scan());
    } else {
        rt.block_on(monitor_loop());
    }
}

fn check_enabled() -> bool {
    std::env::var("MONITOR_ENABLED").unwrap_or_default().to_lowercase() == "true"
}

/// 测试模式：跳过交易日历等待，立即跑一次扫描验证所有模块连通性
async fn push_wechat(text: &str) {
    let script = std::env::var("WECHAT_SEND_SCRIPT")
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{}/Desktop/claude-code-wechat/send-wechat.ts", home)
        });
    match tokio::process::Command::new("bun")
        .arg(&script).arg(text)
        .output().await
    {
        Ok(o) if o.status.success() => log::info!("[微信] 推送成功"),
        Ok(o) => log::warn!("[微信] 推送失败: {}", String::from_utf8_lossy(&o.stderr)),
        Err(e) => log::warn!("[微信] 推送异常: {}", e),
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
    let positions = checklist::PositionSummary::from_db();
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

    log::info!("[测试] ======== 全链路连通性检查完成 ========");
}

async fn monitor_loop() {
    if !calendar::today_is_trading_day() {
        log::info!("今日非交易日，退出监控");
        return;
    }

    while !is_market_active() {
        log::info!("等待交易时段... 当前: {}", calendar::session_label());
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        if !calendar::today_is_trading_day() { return; }
    }

    log::info!("进入交易时段，开始监控");

    let positions = checklist::PositionSummary::from_db();
    let t1_unlocks: Vec<_> = positions.iter().filter(|p| p.t1_locked).cloned().collect();
    let pre_market = checklist::build_pre_market_checklist(&positions, &t1_unlocks, &[]);
    log::info!("[盘前] {} 只持仓，{} 只解禁", positions.len(), t1_unlocks.len());

    push_wechat(&pre_market).await;

    prediction::verify_predictions();
    let hit_rate = prediction::recent_hit_rate(7);
    if hit_rate > 0.0 { log::info!("[预测] 近7天命中率: {:.0}%", hit_rate * 100.0); }

    let mut targets = Vec::new();
    TieredScanner::load_positions(&mut targets);
    TieredScanner::load_watchlist(&mut targets);
    let scanner = TieredScanner::new(targets);

    let detector = Detector::new(DetectorConfig::default());
    let mut state_machine = SignalStateMachine::default();
    let mut signal_count = 0u32;
    let mut alert_count = 0u32;
    let poll_secs: u64 = std::env::var("MONITOR_HOLDING_INTERVAL")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(30);

    loop {
        let session = current_session();

        if session == MarketSession::Auction {
            log::info!("[竞价] 09:25 扫描...");
            if let Ok(analyzer) = stock_analysis::market_analyzer::MarketAnalyzer::new(None) {
                if let Ok(stocks) = analyzer.get_limit_up_stocks() {
                    for s in stocks.iter().take(10) {
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
        }

        if session == MarketSession::Morning || session == MarketSession::Afternoon {
            if let Ok(analyzer) = stock_analysis::market_analyzer::MarketAnalyzer::new(None) {
                if let Ok(stocks) = analyzer.get_limit_up_stocks() {
                    for s in &stocks {
                        let snap = StockSnapshot {
                            code: s.code.clone(), name: s.name.clone(),
                            price: s.price, change_pct: s.change_pct,
                            volume_ratio: 0.0, main_net_yi: 0.0,
                            limit_up_price: Some(s.price * 1.1), was_limit_up: false, t1_locked: false,
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
        }

        if session == MarketSession::AfterHours { break; }
        if session == MarketSession::LunchBreak {
            log::info!("[午休] 暂停扫描");
            tokio::time::sleep(tokio::time::Duration::from_secs(90 * 60)).await;
            continue;
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(poll_secs)).await;
    }

    let summary = checklist::build_close_summary(0.0, 0, 0, 0.0, signal_count as usize, alert_count as usize, &t1_unlocks);
    push_wechat(&summary).await;
    log::info!("[收盘] 信号{}条 告警{}条 | DQ: {} | {}",
        signal_count, alert_count, scanner.dq_summary(), prediction::hit_rate_summary(7));
}

async fn push(event: &AlertEvent) {
    let text = alert::format_alert(event);
    log::info!("[告警] {} {} → {}", event.level.emoji(), event.code, event.message);
    stock_analysis::monitor::alert_log::append_jsonl(event);
    stock_analysis::monitor::alert_log::append_md(event);
    push_wechat(&text).await;
}
