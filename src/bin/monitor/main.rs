//! 实盘监控模式入口。
//!
//! 用法：
//!   cargo run --bin monitor             # 正常监控（等交易日+交易时段）
//!   cargo run --bin monitor -- --test   # 测试模式（跳过日历，立即跑一次扫描验证）
//!
//! 依赖 .env 中 MONITOR_ENABLED=true

use std::io::Write;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use stock_analysis::calendar::{self, current_session, is_market_active, MarketSession};
use stock_analysis::monitor::detector::{AlertCategory, AlertDetail, AlertEvent, AlertLevel, Detector, DetectorConfig, StockSnapshot};
use stock_analysis::monitor::signal_state::SignalStateMachine;
use stock_analysis::monitor::scanner::TieredScanner;
use stock_analysis::monitor::checklist;
use stock_analysis::monitor::prediction;
use stock_analysis::monitor::alert;

pub const DEFAULT_MAGICLAW_API_ADDR: &str = "127.0.0.1:18011";
pub const DEFAULT_MAGICLAW_PROJECT_ID: &str = "stock_analysis";
pub const DEFAULT_MAGICLAW_CLIENT_NAME: &str = "monitor";
pub const DEFAULT_MAGICLAW_TOKEN_TTL_SECS: i64 = 7 * 24 * 3600;
pub const DEFAULT_MAGICLAW_TOKEN_REFRESH_AHEAD_SECS: i64 = 10 * 60;

pub static MAGICLAW_DAEMON_BOOT_LOCK: Lazy<tokio::sync::Mutex<()>> =
    Lazy::new(|| tokio::sync::Mutex::new(()));
pub static MAGICLAW_TOKEN_MEM_CACHE: Lazy<tokio::sync::RwLock<Option<CachedApiToken>>> =
    Lazy::new(|| tokio::sync::RwLock::new(None));
pub static MAGICLAW_TOKEN_ISSUE_LOCK: Lazy<tokio::sync::Mutex<()>> =
    Lazy::new(|| tokio::sync::Mutex::new(()));
pub static MAGICLAW_DISABLE_ENV_TOKEN: AtomicBool = AtomicBool::new(false);

mod notify;
use notify::{summarize_push_text, evaluate_opportunity_push_skip_reason};

mod market_data;
pub enum DaemonReadySource {
    Reused,
    StartedNow,
}

pub enum ApiTokenSource {
    Env,
    DynamicMemCache,
    DynamicFileCache,
    DynamicIssued,
}

#[derive(Clone, Copy)]
pub enum MessageSendType {
    Wechat,
    Feishu,
}

#[derive(Clone, Copy)]
pub enum MessageSendTransport {
    Http,
    Cli,
}

impl MessageSendType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Wechat => "wechat",
            Self::Feishu => "feishu",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Wechat => "微信",
            Self::Feishu => "飞书",
        }
    }
}

impl MessageSendTransport {
    fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Cli => "cli",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CachedApiToken {
    token: String,
    expires_at: Option<i64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AirRefuelEntryMode {
    Confirm,
    Pilot,
}

fn air_refuel_entry_mode() -> AirRefuelEntryMode {
    let mode = stock_analysis::config::get_monitor_config().air_refuel.entry_mode;
    if mode.trim().eq_ignore_ascii_case("pilot") {
        AirRefuelEntryMode::Pilot
    } else {
        AirRefuelEntryMode::Confirm
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualObservationRecord {
    entry_date: String,
    code: String,
    name: String,
    entry_price: f64,
    shares: u32,
    entry_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualObservationSnapshot {
    created_at: String,
    records: Vec<VirtualObservationRecord>,
}

fn virtual_observation_dir() -> std::path::PathBuf {
    std::path::PathBuf::from("data/virtual_observation")
}

fn persist_virtual_observation_snapshot(records: &[VirtualObservationRecord]) {
    if records.is_empty() {
        return;
    }
    let dir = virtual_observation_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("[虚拟观察仓] 创建目录失败: {}", e);
        return;
    }
    let snapshot = VirtualObservationSnapshot {
        created_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        records: records.to_vec(),
    };
    let json = match serde_json::to_string_pretty(&snapshot) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[虚拟观察仓] 序列化失败: {}", e);
            return;
        }
    };
    let today = chrono::Local::now().format("%Y%m%d").to_string();
    let daily = dir.join(format!("{}.json", today));
    let latest = dir.join("latest.json");
    if let Err(e) = std::fs::write(&daily, &json) {
        log::warn!("[虚拟观察仓] 写入日快照失败: {}", e);
        return;
    }
    if let Err(e) = std::fs::write(&latest, &json) {
        log::warn!("[虚拟观察仓] 写入 latest 失败: {}", e);
        return;
    }
    log::info!("[虚拟观察仓] 已落盘: {} ({}条)", daily.display(), records.len());
}

fn load_latest_prior_virtual_snapshot() -> Option<VirtualObservationSnapshot> {
    let dir = virtual_observation_dir();
    let entries = std::fs::read_dir(&dir).ok()?;
    let today = chrono::Local::now().format("%Y%m%d").to_string();
    let mut best: Option<std::path::PathBuf> = None;
    let mut best_day = String::new();
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let stem = match p.file_stem().and_then(|x| x.to_str()) {
            Some(s) => s,
            None => continue,
        };
        if stem == "latest" || stem.len() != 8 || stem >= today.as_str() {
            continue;
        }
        if best.is_none() || stem > best_day.as_str() {
            best_day = stem.to_string();
            best = Some(p);
        }
    }
    let path = best?;
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<VirtualObservationSnapshot>(&raw).ok()
}

fn fetch_latest_close_map(codes: &[String]) -> std::collections::HashMap<String, f64> {
    let mut out = std::collections::HashMap::new();
    let fetcher = match stock_analysis::data_provider::DataFetcherManager::new() {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[虚拟观察仓] 初始化数据抓取器失败: {:#}", e);
            return out;
        }
    };
    for code in codes {
        if let Ok((kline, _)) = fetcher.get_daily_data(code, 3) {
            if let Some(last) = kline.last() {
                if last.close > 0.0 {
                    out.insert(code.clone(), last.close);
                }
            }
        }
    }
    out
}

fn build_virtual_next_day_review_text(
    snapshot: &VirtualObservationSnapshot,
    close_map: &std::collections::HashMap<String, f64>,
) -> Option<String> {
    if snapshot.records.is_empty() {
        return None;
    }
    let mut lines = vec![
        format!("📘 虚拟观察仓次日表现（基于 {} 建仓）", snapshot.created_at),
        "━━━━━━━━━━━━━━━━━━━━━━━━".to_string(),
    ];
    let mut win = 0usize;
    let mut n = 0usize;
    let mut pnl_total = 0.0_f64;
    let mut capital_total = 0.0_f64;
    for r in &snapshot.records {
        if r.entry_price <= 0.0 || r.shares == 0 {
            continue;
        }
        let Some(close) = close_map.get(&r.code).copied() else {
            lines.push(format!("  {}({}) 数据不足", r.name, r.code));
            continue;
        };
        let ret = (close / r.entry_price - 1.0) * 100.0;
        let pnl = (close - r.entry_price) * r.shares as f64;
        if ret > 0.0 {
            win += 1;
        }
        n += 1;
        pnl_total += pnl;
        capital_total += r.entry_price * r.shares as f64;
        lines.push(format!(
            "  {}({}) {}股 入场¥{:.2} -> 收盘¥{:.2} | {:+.2}% | {:+.0}",
            r.name, r.code, r.shares, r.entry_price, close, ret, pnl
        ));
    }
    if n == 0 {
        return None;
    }
    let hit_rate = win as f64 / n as f64 * 100.0;
    let total_ret = if capital_total > 0.0 {
        pnl_total / capital_total * 100.0
    } else {
        0.0
    };
    lines.push(String::new());
    lines.push(format!(
        "命中率 {:.1}% ({}/{}) | 组合收益 {:+.2}% | 组合盈亏 {:+.0}",
        hit_rate, win, n, total_ret, pnl_total
    ));
    Some(lines.join("\n"))
}

async fn push_virtual_next_day_review_if_needed() {
    let cfg = stock_analysis::config::get_monitor_config();
    if !cfg.air_refuel.next_day_review_enabled {
        return;
    }
    let Some(snapshot) = load_latest_prior_virtual_snapshot() else {
        return;
    };
    let codes: Vec<String> = snapshot.records.iter().map(|r| r.code.clone()).collect();
    let close_map = tokio::task::spawn_blocking(move || fetch_latest_close_map(&codes))
        .await
        .unwrap_or_default();
    if let Some(text) = build_virtual_next_day_review_text(&snapshot, &close_map) {
        push_wechat(&text).await;
    }
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|buf, record| writeln!(buf, "[{} {}] {}", chrono::Local::now().format("%H:%M:%S"), record.level(), record.args()))
        .init();

    if !check_enabled() { return; }
    // 初始化数据库
    let db_path = std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/stock_analysis.db".into());
    if std::env::var("MAGICLAW_DB_PATH").ok().map(|s| s.trim().is_empty()).unwrap_or(true) {
        std::env::set_var("MAGICLAW_DB_PATH", &db_path);
    }
    let _ = stock_analysis::database::DatabaseManager::init(Some(std::path::PathBuf::from(&db_path)));
    // 加载热配置
    stock_analysis::config::load_all();
    let test_mode = std::env::args().any(|a| a == "--test");
    let review_mode = std::env::args().any(|a| a == "--review");

    // 显式标记交易环境，供底层写入守卫执行双向隔离。
    std::env::set_var("STOCK_ENV_MODE", if test_mode { "test" } else { "prod" });

    log::info!("实盘监控启动 | {} | 当前: {} | 模式: {}",
        if calendar::today_is_trading_day() { "交易日" } else { "非交易日" },
        calendar::session_label(),
        if test_mode { "测试" } else if review_mode { "复盘" } else { "正常" },
    );

    // 事件总线 — 允许多个消费者独立订阅监控事件
    let (event_tx, _event_rx) = tokio::sync::broadcast::channel::<String>(256);

    if test_mode {
        run_test_scan().await;
    } else if review_mode {
        run_review_only().await;
    } else {
        let main_loops = async {
            tokio::join!(monitor_loop(), news_monitor_loop());
        };

        let _ = event_tx; // TODO: wire into scanner→detector→alert pipeline

        tokio::select! {
            _ = main_loops => {},
            _ = tokio::signal::ctrl_c() => {
                log::warn!("收到 SIGINT，正在优雅关闭监控...");
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                log::info!("监控已安全关闭");
            }
        }
    }
}

fn check_enabled() -> bool {
    std::env::var("MONITOR_ENABLED").unwrap_or_default().to_lowercase() == "true"
}

// 通知层已提取到 notify.rs
async fn push_wechat(text: &str) -> bool {
    notify::push_wechat(text).await
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
        let quotes = market_data::fetch_position_quotes();
        let trades = stock_analysis::portfolio::get_trade_history(90).unwrap_or_default();
        let mut reviews = stock_analysis::review::journal::review_closed_trades(&trades);
        stock_analysis::review::journal::enrich_post_exit(&mut reviews);
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
    let scan = stock_analysis::opportunity::run_opportunity_scan().await;
    log::info!("[测试] 产业链扫描:\n{}", scan.chain_text);
    push_wechat(&scan.chain_text).await;
    if !scan.impact_text.is_empty() {
        log::info!("[测试] 持仓影响:\n{}", scan.impact_text);
        push_wechat(&scan.impact_text).await;
    }

    // 14. v4 决策层：排除引擎 + 风控（含 HTTP 调用，走 spawn_blocking）
    let h = holdings.clone();
    let (excl_hits, violations) = tokio::task::spawn_blocking(move || {
        let watchlist = stock_analysis::portfolio::get_watchlist().unwrap_or_default();
        let excl = stock_analysis::decision::exclusion::scan_exclusions(&h, &watchlist);
        let limits = stock_analysis::risk::limits::HardLimits::default();
        let quotes = market_data::fetch_position_quotes();
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

/// P0-3: AI 评分因子 IC 分析。读取已平仓交易 + 买入日评分，计算各因子的 IC/IR。
fn run_factor_ic_analysis() -> Option<String> {
    stock_analysis::review::factor_ic::run_diagnostic()
}

/// 手动复盘：`cargo run --bin monitor -- --review`
async fn run_review_only() {
    log::info!("[复盘] 手动触发盘后分析...");

    let (report, holding_breakout_text, watch_breakout_text, market_breakout_text, risk_text) = tokio::task::spawn_blocking(|| {
        let holdings = stock_analysis::portfolio::get_positions().unwrap_or_default();
        let quotes = market_data::fetch_position_quotes();
        let prices = build_price_map(&quotes);
        let trades = stock_analysis::portfolio::get_trade_history(90).unwrap_or_default();
        let mut reviews = stock_analysis::review::journal::review_closed_trades(&trades);
        stock_analysis::review::journal::enrich_post_exit(&mut reviews);
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

        // v6 放量分析（持仓 / 自选 分开发送）
        let mut holding_brk = String::new();
        let mut watch_brk = String::new();
        let mut market_brk = String::new();
        // v7 风控：收盘止损 + 轮动研判（复用已拉 K 线，零额外 HTTP）
        let mut stop_signals: Vec<stock_analysis::risk::stop_loss::StopSignal> = Vec::new();
        let mut rotation_lines: Vec<String> = Vec::new();
        let watchlist = stock_analysis::portfolio::get_watchlist().unwrap_or_default();
        let watch_codes: std::collections::HashSet<String> =
            watchlist.iter().map(|p| p.code.clone()).collect();
        if let Ok(fetcher) = stock_analysis::data_provider::DataFetcherManager::new() {
            // —— 持仓放量分析 + 止损 / 轮动 ——
            let mut holding_lines = vec!["📊 放量分析·持仓（盘后·算法研判仅供参考）".to_string()];
            for p in &holdings {
                if let Ok((kline, _)) = fetcher.get_daily_data(&p.code, 60) {
                    let sig = stock_analysis::breakout::engine::analyze_postmarket(&p.code, &p.name, &kline);
                    holding_lines.push(format!(
                        "  {} {}({}) — {} 置信{}% [{}]",
                        sig.breakout_type.emoji(), sig.name, sig.code,
                        sig.breakout_type.label(), sig.confidence, sig.description,
                    ));

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
            if holding_lines.len() > 1 { holding_brk = holding_lines.join("\n"); }

            // —— 自选（STOCK_LIST）放量分析（剔除已在持仓列出的标的）——
            let mut watch_lines = vec!["📊 放量分析·自选（盘后·算法研判仅供参考）".to_string()];
            for p in &watchlist {
                if holding_codes.contains(&p.code) { continue; }
                if let Ok((kline, _)) = fetcher.get_daily_data(&p.code, 60) {
                    let sig = stock_analysis::breakout::engine::analyze_postmarket(&p.code, &p.name, &kline);
                    watch_lines.push(format!(
                        "  {} {}({}) — {} 置信{}% [{}]",
                        sig.breakout_type.emoji(), sig.name, sig.code,
                        sig.breakout_type.label(), sig.confidence, sig.description,
                    ));
                }
            }
            if watch_lines.len() > 1 { watch_brk = watch_lines.join("\n"); }

            // —— 实盘量能优选：全市场量能前列 + 走势较好（盘后 Top5）——
            let mut market_lines = vec!["📊 放量分析·实盘优选（盘后·算法研判仅供参考）".to_string()];
            let market_candidates = market_data::fetch_market_volume_ratio_leaders(80).unwrap_or_default();
            let mut picked = 0usize;
            for s in &market_candidates {
                if picked >= 5 { break; }
                if holding_codes.contains(&s.code) || watch_codes.contains(&s.code) {
                    continue;
                }
                if let Ok((kline, _)) = fetcher.get_daily_data(&s.code, 60) {
                    let sig = stock_analysis::breakout::engine::analyze_postmarket(&s.code, &s.name, &kline);
                    if sig.breakout_type != stock_analysis::breakout::signal::BreakoutType::Launch || sig.confidence < 50 {
                        continue;
                    }
                    market_lines.push(format!(
                        "  {} {}({}) — {} 置信{}% [量比{:.1} 主力{:+.2}亿 | {}]",
                        sig.breakout_type.emoji(), sig.name, sig.code,
                        sig.breakout_type.label(), sig.confidence,
                        s.volume_ratio, s.main_net_yi, sig.description,
                    ));
                    picked += 1;
                }
            }
            if market_lines.len() > 1 { market_brk = market_lines.join("\n"); }
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
        (r, holding_brk, watch_brk, market_brk, risk)
    }).await.unwrap_or_default();

    log::info!("[复盘] 复盘报告:\n{}", report);
    push_wechat(&report).await;

    // 与正常收盘路径保持一致：推送优选候选（最多5只，阈值过滤后可少推/不推）
    let post_close_candidates = stock_analysis::opportunity::run_post_close_candidates(5).await;
    log::info!("[复盘] 优选候选:\n{}", post_close_candidates);
    push_wechat(&post_close_candidates).await;

    // 盘后统计上一交易日虚拟观察仓表现（可配置开关）
    push_virtual_next_day_review_if_needed().await;

    if !holding_breakout_text.is_empty() {
        log::info!("[复盘] 放量分析·持仓:\n{}", holding_breakout_text);
        push_wechat(&holding_breakout_text).await;
    }

    if !watch_breakout_text.is_empty() {
        log::info!("[复盘] 放量分析·自选:\n{}", watch_breakout_text);
        push_wechat(&watch_breakout_text).await;
    }

    if !market_breakout_text.is_empty() {
        log::info!("[复盘] 放量分析·实盘优选:\n{}", market_breakout_text);
        push_wechat(&market_breakout_text).await;
    }

    if !risk_text.is_empty() {
        log::info!("[复盘] 风控研判:\n{}", risk_text);
        push_wechat(&risk_text).await;
    }

    // 盘后持仓多 Agent 深度研判（6 分析师 + 多空辩论 + 仲裁），逐只推送飞书
    run_review_deep_analysis().await;

    let falsify_text = stock_analysis::review::falsify::daily_falsify();
    log::info!("[复盘] 证伪提醒:\n{}", falsify_text);
    push_wechat(&falsify_text).await;

    // P0-3: AI 评分因子 IC 分析
    if let Some(ic_report) = run_factor_ic_analysis() {
        log::info!("[复盘] 因子IC分析:\n{}", ic_report);
    }

    log::info!("[复盘] ======== 盘后分析完成 ========");
}

/// 盘后持仓多 Agent 深度研判：对每只真实持仓跑「6 分析师 + 多空辩论 + 仲裁」流水线，
/// 结果逐只推送飞书。受 `AI_AGENT_PIPELINE`（默认开启）控制；关闭则整体跳过。
async fn run_review_deep_analysis() {
    use futures::stream::{self, StreamExt};

    // 开关：与主流程一致，AI_AGENT_PIPELINE=false 时不跑多 Agent
    let enabled = std::env::var("AI_AGENT_PIPELINE")
        .map(|v| v.trim().to_lowercase() != "false")
        .unwrap_or(true);
    if !enabled {
        log::info!("[复盘] AI_AGENT_PIPELINE=false，跳过持仓多 Agent 深度研判");
        return;
    }

    let holdings = stock_analysis::portfolio::get_positions().unwrap_or_default();
    if holdings.is_empty() {
        log::info!("[复盘] 无持仓，跳过多 Agent 深度研判");
        return;
    }

    // 深度研判并发度（LLM 密集，默认 3）
    let concurrency = std::env::var("DEEP_ANALYSIS_CONCURRENCY")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&c| c > 0)
        .unwrap_or(3);

    log::info!("[复盘] 持仓多 Agent 深度研判开始（{} 只，并发 {}）", holdings.len(), concurrency);

    // 并发跑多 Agent，结果回收后按持仓顺序推送
    let codes: Vec<(String, String)> =
        holdings.iter().map(|p| (p.code.clone(), p.name.clone())).collect();

    let results: Vec<(String, String, Option<String>)> = stream::iter(codes)
        .map(|(code, name)| async move {
            log::info!("[复盘] ▶ 多 Agent 研判 {} {}", code, name);
            let deep = tokio::time::timeout(
                std::time::Duration::from_secs(300),
                stock_analysis::deep_analyzer::run_multi_agent_analysis(&code),
            )
            .await;
            let md = match deep {
                Ok(Ok(md)) if !md.trim().is_empty() => Some(md),
                Ok(Ok(_)) => {
                    log::warn!("[复盘] {} 多 Agent 返回空", code);
                    None
                }
                Ok(Err(e)) => {
                    log::warn!("[复盘] {} 多 Agent 失败: {:#}", code, e);
                    None
                }
                Err(_) => {
                    log::warn!("[复盘] {} 多 Agent 超时(300s)", code);
                    None
                }
            };
            (code, name, md)
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    // 按持仓原顺序推送（buffer_unordered 完成顺序不确定，重排回固定顺序）
    let mut by_code: std::collections::HashMap<String, (String, Option<String>)> =
        results.into_iter().map(|(c, n, m)| (c, (n, m))).collect();
    for p in &holdings {
        let Some((name, md)) = by_code.remove(&p.code) else { continue };
        let Some(md) = md else { continue };
        let header = format!("🧠 持仓深度研判 · {}({})\n", name, p.code);
        let text = format!("{}{}", header, md);
        log::info!("[复盘] 持仓深度研判 {}({}):\n{}", name, p.code, md);
        push_wechat(&text).await;
    }

    log::info!("[复盘] 持仓多 Agent 深度研判完成");
}

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
                let scan = stock_analysis::opportunity::run_opportunity_scan().await;
                // 仅在有实际机会时推送；空结果（暂无快讯/未命中/无可用标的）只记日志不刷屏。
                let opp_text = &scan.chain_text;
                if let Some(reason) = evaluate_opportunity_push_skip_reason(opp_text) {
                    log::info!(
                        "[产业链] 跳过推送 | reason={} | preview={}",
                        reason,
                        summarize_push_text(opp_text, 120)
                    );
                } else {
                    log::info!("[产业链] {}", opp_text);
                    let ok = push_wechat(opp_text).await;
                    log::info!(
                        "[产业链] 推送结果 | ok={} | preview={}",
                        ok,
                        summarize_push_text(opp_text, 120)
                    );
                }
                // 持仓影响分开推送
                if !scan.impact_text.is_empty() {
                    log::info!("[持仓影响] {}", scan.impact_text);
                    let ok = push_wechat(&scan.impact_text).await;
                    log::info!(
                        "[持仓影响] 推送结果 | ok={} | preview={}",
                        ok,
                        summarize_push_text(&scan.impact_text, 120)
                    );
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
        let mut last_fund_top_push = std::time::Instant::now();  // 全市场主力净流入Top10（5分钟）
        // 产业链扫描已移至 news_monitor_loop 的 8:00-22:00 窗口统一调度。
        let mut was_limit_up: std::collections::HashSet<String> = std::collections::HashSet::new();
        // 连板追踪：已推送过的标的不重复推送；board_level_cache 存 1=首板/2=二板/3+=三板
        let mut board_notified: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut board_level_cache: std::collections::HashMap<String, u8> = std::collections::HashMap::new();
        // 竞价量能扫描：9:20-9:25 每30秒推送一次全市场涨停量能榜
        let mut auction_vol_notified: std::collections::HashSet<String> = std::collections::HashSet::new();
        // 优选候选虚拟仓位记录：从集合竞价推送的候选+开盘价记录
        let mut virtual_observation: Vec<(String, String, f64)> = Vec::new(); // (code, name, open_price)
        let mut post_close_candidates_notified = false;
        let mut virtual_snapshot_persisted = false;
        let entry_mode = air_refuel_entry_mode();
        let monitor_cfg = stock_analysis::config::get_monitor_config();
        let confirm_shares = monitor_cfg.air_refuel.confirm_lots.saturating_mul(100);
        let pilot_shares = monitor_cfg.air_refuel.pilot_lots.saturating_mul(100);

        loop {
            let session = current_session();

            // ── 9:20-9:25 竞价高量能扫描（30秒一次）+ 盘后优选重推 ──
            if session == MarketSession::Auction {
                let now_time = chrono::Local::now().time();
                // 9:20 之前只做持仓告警，不推全市场量能（数据不稳定）
                if now_time >= chrono::NaiveTime::from_hms_opt(9, 20, 0).unwrap() {
                    log::info!("[竞价] 9:20-9:25 量能扫描...");
                    let limit_stocks = tokio::task::spawn_blocking(|| {
                        let analyzer = stock_analysis::market_analyzer::MarketAnalyzer::new(None).ok()?;
                        analyzer.get_limit_up_stocks().ok()
                    }).await.unwrap_or(None).unwrap_or_default();

                    if !limit_stocks.is_empty() {
                        // 按量比降序，取量比最高的前10（量能高代表竞价封板意愿强）
                        let mut sorted = limit_stocks.clone();
                        sorted.sort_by(|a, b| {
                            b.volume_ratio.partial_cmp(&a.volume_ratio).unwrap_or(std::cmp::Ordering::Equal)
                        });
                        let new_items: Vec<_> = sorted.iter()
                            .filter(|s| !auction_vol_notified.contains(&s.code))
                            .take(10)
                            .collect();
                        if !new_items.is_empty() {
                            let ts = chrono::Local::now().format("%H:%M:%S");
                            let mut lines = vec![format!("⚡ 竞价涨停·量能 Top{}（{}）", new_items.len(), ts)];
                            for s in &new_items {
                                auction_vol_notified.insert(s.code.clone());
                                lines.push(format!(
                                    "  {}({}) 量比{:.1} 主力{:+.2}亿 {:+.1}%",
                                    s.name, s.code, s.volume_ratio, s.main_net_yi, s.change_pct,
                                ));
                            }
                            push_wechat(&lines.join("\n")).await;
                        }
                    }

                    // ▶ 新增：9:20-9:25 集合竞价阶段重推优选候选（仅一次）
                    if !post_close_candidates_notified {
                        post_close_candidates_notified = true;
                        log::info!("[竞价] 9:20-9:25 更新推送优选候选（2.1版本）...");
                        let post_close = stock_analysis::opportunity::run_post_close_candidates(5).await;
                        push_wechat(&post_close).await;
                        
                        // 提取候选的code和name以便后续虚拟记录（简单方式：从推送文案中正则提取）
                        // 格式: "N. 名称(代码)" → 收集前5个作为虚拟观察对象
                        let mut seen_codes: std::collections::HashSet<String> = std::collections::HashSet::new();
                        for line in post_close.lines() {
                            if let Some(paren_start) = line.find('(') {
                                if let Some(paren_end) = line.find(')') {
                                    if paren_start < paren_end {
                                        let code_str = &line[paren_start+1..paren_end];
                                        if code_str.len() == 6 && code_str.chars().all(|c| c.is_numeric()) {
                                            if !seen_codes.insert(code_str.to_string()) {
                                                continue;
                                            }
                                            // 从该行"  "后提取name
                                            let name_part = line.trim_start();
                                            if let Some(name_end) = name_part.find('(') {
                                                let name = name_part[..name_end].trim_end();
                                                // 移除序号 "N. "
                                                let name = if let Some(dot_pos) = name.find('.') {
                                                    name[dot_pos+1..].trim()
                                                } else {
                                                    name
                                                };
                                                virtual_observation.push((code_str.to_string(), name.to_string(), 0.0));
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // pilot 模式：竞价阶段先按当前价格虚拟潜伏记录（仅一次）
                        if entry_mode == AirRefuelEntryMode::Pilot && !virtual_observation.is_empty() {
                            let codes: Vec<String> = virtual_observation.iter().map(|(c, _, _)| c.clone()).collect();
                            let quote_map = tokio::task::spawn_blocking(move || {
                                let quotes = market_data::fetch_eastmoney_quotes(&codes).unwrap_or_default();
                                quotes.into_iter().map(|q| (q.code, q.price)).collect::<std::collections::HashMap<_, _>>()
                            }).await.unwrap_or_default();

                            for v in &mut virtual_observation {
                                if let Some(px) = quote_map.get(&v.0) {
                                    if *px > 0.0 {
                                        v.2 = *px;
                                    }
                                }
                            }

                            let mut lines = vec![
                                "🟠 虚拟观察仓位（尾盘/竞价潜伏模式）".to_string(),
                                String::new(),
                            ];
                            let mut records: Vec<VirtualObservationRecord> = Vec::new();
                            let mut total_amount = 0.0_f64;
                            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                            for (code, name, price) in &virtual_observation {
                                if *price <= 0.0 {
                                    continue;
                                }
                                let amount = *price * pilot_shares as f64;
                                total_amount += amount;
                                lines.push(format!(
                                    "  {}({}) @ ¥{:.2} | {}股 预计 ¥{:.0}",
                                    name, code, price, pilot_shares, amount
                                ));
                                records.push(VirtualObservationRecord {
                                    entry_date: today.clone(),
                                    code: code.clone(),
                                    name: name.clone(),
                                    entry_price: *price,
                                    shares: pilot_shares,
                                    entry_mode: "pilot".to_string(),
                                });
                            }
                            lines.push(format!(
                                "\n合计虚拟敞口: ¥{:.0} ({}股×{}只)",
                                total_amount,
                                pilot_shares,
                                records.len()
                            ));
                            lines.push("\n⚠️ 仅做观察、研究用途，未实际下单".to_string());
                            if !records.is_empty() {
                                persist_virtual_observation_snapshot(&records);
                                virtual_snapshot_persisted = true;
                                push_wechat(&lines.join("\n")).await;
                            }
                        }
                    }

                    // 持仓信号（原有逻辑保留）
                    for s in limit_stocks.iter().take(10) {
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

                    tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                    continue;
                } else {
                    // 9:15-9:20 等待即可
                    tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                    continue;
                }
            }

            if session == MarketSession::Morning || session == MarketSession::Afternoon {
                let result = tokio::task::spawn_blocking(|| {
                    let analyzer = stock_analysis::market_analyzer::MarketAnalyzer::new(None).ok()?;
                    let limit_stocks = analyzer.get_limit_up_stocks().ok().unwrap_or_default();
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    let position_quotes = market_data::fetch_position_quotes();
                    Some((limit_stocks, position_quotes))
                }).await.unwrap_or(None);

                if let Some((limit_stocks, position_quotes)) = result {
                    // ▶ 新增：开盘后虚拟记录观察仓位（仅一次）
                    if entry_mode == AirRefuelEntryMode::Confirm
                        && session == MarketSession::Morning
                        && !virtual_observation.is_empty()
                        && virtual_observation.iter().all(|(_, _, p)| *p == 0.0)
                    {
                        log::info!(
                            "[开盘] 虚拟观察仓位初始化（{}手 × {}只）",
                            confirm_shares / 100,
                            virtual_observation.len()
                        );
                        
                        // 从当前行情中获取这些候选的开盘价/实时价
                        for pos_quote in &position_quotes {
                            for virtual_pos in &mut virtual_observation {
                                if virtual_pos.0 == pos_quote.code && virtual_pos.2 == 0.0 {
                                    virtual_pos.2 = pos_quote.price;
                                }
                            }
                        }
                        
                        // 补充从limit_stocks中没获取到的价格
                        for limit_stock in &limit_stocks {
                            for virtual_pos in &mut virtual_observation {
                                if virtual_pos.0 == limit_stock.code && virtual_pos.2 == 0.0 {
                                    virtual_pos.2 = limit_stock.price;
                                }
                            }
                        }
                        
                        // 推送虚拟观察仓位摘要
                        let mut virtual_lines = vec![
                            format!("🔍 虚拟观察仓位（盘后优选·开盘价·{}手/只）", confirm_shares / 100),
                            "".to_string(),
                        ];
                        let mut total_amount = 0.0;
                        let mut records: Vec<VirtualObservationRecord> = Vec::new();
                        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                        for (code, name, price) in &virtual_observation {
                            if *price > 0.0 {
                                let amount = price * confirm_shares as f64;
                                total_amount += amount;
                                virtual_lines.push(format!(
                                    "  {}({}) @ ¥{:.2} | {}股 预计 ¥{:.0}",
                                    name, code, price, confirm_shares, amount
                                ));
                                records.push(VirtualObservationRecord {
                                    entry_date: today.clone(),
                                    code: code.clone(),
                                    name: name.clone(),
                                    entry_price: *price,
                                    shares: confirm_shares,
                                    entry_mode: "confirm".to_string(),
                                });
                            }
                        }
                        virtual_lines.push(format!(
                            "\n合计虚拟敞口: ¥{:.0} ({}股×{}只)",
                            total_amount, confirm_shares, virtual_observation.len()
                        ));
                        virtual_lines.push("\n⚠️ 仅做观察、研究用途，未实际下单".to_string());

                        if !virtual_snapshot_persisted && !records.is_empty() {
                            persist_virtual_observation_snapshot(&records);
                            virtual_snapshot_persisted = true;
                        }
                        
                        push_wechat(&virtual_lines.join("\n")).await;
                        log::info!("[开盘] 虚拟观察仓位已推送（合计 ¥{:.0}）", total_amount);
                    }

                    // 首板/二板/三板识别：全市场涨停池，各自独立消息，每只仅推一次
                    if !limit_stocks.is_empty() {
                        let mut need_lookup: Vec<(String, String)> = Vec::new();
                        for s in &limit_stocks {
                            if board_notified.contains(&s.code) { continue; }
                            if !board_level_cache.contains_key(&s.code) {
                                need_lookup.push((s.code.clone(), s.name.clone()));
                            }
                        }
                        if !need_lookup.is_empty() {
                            let need_lookup: Vec<(String, String)> = need_lookup.into_iter().take(40).collect();
                            let looked_up = tokio::task::spawn_blocking(move || {
                                market_data::lookup_board_level_batch(&need_lookup)
                            }).await.unwrap_or_default();
                            board_level_cache.extend(looked_up);
                        }

                        let mut first_lines: Vec<String> = Vec::new();
                        let mut second_lines: Vec<String> = Vec::new();
                        let mut third_lines: Vec<String> = Vec::new();
                        let mut sorted_limits = limit_stocks.clone();
                        sorted_limits.sort_by(|a, b| {
                            b.main_net_yi.partial_cmp(&a.main_net_yi).unwrap_or(std::cmp::Ordering::Equal)
                        });
                        for s in sorted_limits.iter().take(50) {
                            let level = match board_level_cache.get(&s.code) {
                                Some(v) => *v,
                                None => continue,
                            };
                            if !board_notified.insert(s.code.clone()) { continue; }
                            let row = format!(
                                "  {}({}) 主力{:+.2}亿 量比{:.1} {:+.1}%",
                                s.name, s.code, s.main_net_yi, s.volume_ratio, s.change_pct,
                            );
                            match level {
                                1 => first_lines.push(row),
                                2 => second_lines.push(row),
                                _ => third_lines.push(row),
                            }
                        }

                        let ts = chrono::Local::now().format("%H:%M");
                        if !first_lines.is_empty() {
                            let mut lines = vec![format!("🟢 首板涨停 Top{}（{}）", first_lines.len().min(10), ts)];
                            lines.extend(first_lines.into_iter().take(10));
                            push_wechat(&lines.join("\n")).await;
                        }
                        if !second_lines.is_empty() {
                            let mut lines = vec![format!("🟡 二板涨停 Top{}（{}）", second_lines.len().min(10), ts)];
                            lines.extend(second_lines.into_iter().take(10));
                            push_wechat(&lines.join("\n")).await;
                        }
                        if !third_lines.is_empty() {
                            let mut lines = vec![format!("🔴 三板+ 涨停 Top{}（{}）", third_lines.len().min(10), ts)];
                            lines.extend(third_lines.into_iter().take(10));
                            push_wechat(&lines.join("\n")).await;
                        }
                    }

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

                    // 全市场主力净流入 Top10（独立计时器，每5分钟）
                    if last_fund_top_push.elapsed().as_secs() >= 300 {
                        last_fund_top_push = std::time::Instant::now();
                        push_market_fund_top10().await;
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
        let index_change = tokio::task::spawn_blocking(market_data::fetch_sh_index_change)
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
        let mut reviews = stock_analysis::review::journal::review_closed_trades(&trades);
        stock_analysis::review::journal::enrich_post_exit(&mut reviews);
        let equity = stock_analysis::portfolio::get_equity_curve(365).unwrap_or_default();
        let mut stats = stock_analysis::review::equity::compute_stats(&equity);
        stock_analysis::review::equity::enrich_with_trades(&mut stats, &reviews);
        let holdings = stock_analysis::portfolio::get_positions().unwrap_or_default();
        let prices = tokio::task::spawn_blocking(|| {
            let quotes = market_data::fetch_position_quotes();
            build_price_map(&quotes)
        })
        .await
        .unwrap_or_default();
        let review_report = stock_analysis::review::report::generate_daily_report(&reviews, &stats, &holdings, &prices);
        push_wechat(&review_report).await;

        // 盘后独立维度：优选次日候选（最多 5 只，达不到阈值可少推/不推），强调可解释性，不复用盘中量能信号口径。
        let post_close_candidates = stock_analysis::opportunity::run_post_close_candidates(5).await;
        push_wechat(&post_close_candidates).await;

        // 盘后统计上一交易日虚拟观察仓表现（可配置开关）
        push_virtual_next_day_review_if_needed().await;

        // v3 每日净值快照
        let _ = tokio::task::spawn_blocking(snapshot_portfolio_value).await;

        // 盘后持仓多 Agent 深度研判（6 分析师 + 多空辩论 + 仲裁），逐只推送飞书
        run_review_deep_analysis().await;

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
        match market_data::fetch_eastmoney_quotes(&codes) {
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

async fn push_market_fund_top10() {
    let top = tokio::task::spawn_blocking(|| market_data::fetch_market_main_inflow_top(10))
        .await
        .unwrap_or_else(|_| Err("spawn_blocking join error".to_string()))
        .unwrap_or_default();

    if top.is_empty() {
        return;
    }

    let mut lines = vec![format!(
        "💰 主力净流入 Top 10（{}）",
        chrono::Local::now().format("%H:%M")
    )];
    for (i, s) in top.iter().enumerate() {
        lines.push(format!(
            "  {:>2}. {}({}) 主力{:+.2}亿 量比{:.1} 涨幅{:+.1}%",
            i + 1,
            s.name,
            s.code,
            s.main_net_yi,
            s.volume_ratio,
            s.change_pct,
        ));
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

fn monitor_freshness_config() -> stock_analysis::monitor::data_quality::FreshnessConfig {
    let cfg = stock_analysis::config::get_monitor_config();
    stock_analysis::monitor::data_quality::FreshnessConfig {
        quote_max_age_secs: cfg.dq_quote_stale_sec,
        position_max_age_secs: cfg.dq_position_stale_sec,
        nav_max_age_secs: cfg.dq_nav_stale_sec,
        daily_max_age_secs: cfg.dq_daily_stale_sec,
    }
}

pub fn validate_position_freshness(fetch_time: chrono::DateTime<chrono::Local>) -> bool {
    let stats = stock_analysis::monitor::data_quality::DqStats::new();
    let freshness = monitor_freshness_config();
    match stock_analysis::monitor::data_quality::validate_freshness(
        stock_analysis::monitor::data_quality::FreshnessDataType::Position,
        fetch_time,
        &freshness,
        &stats,
    ) {
        Ok(()) => true,
        Err(reason) => {
            log::warn!(
                "[DQ_FRESHNESS] rule_id=AGENTS-2.4 data_type=position action=reject reason={} timestamp={}",
                reason.label(),
                chrono::Utc::now().timestamp()
            );
            false
        }
    }
}

pub fn validate_quote_freshness(update_time: chrono::DateTime<chrono::Local>, source: &str, code: &str) -> bool {
    // AGENTS 2.4 指定为“实时行情(盘中) 5 秒”，非盘中时段不做 5 秒硬拦截。
    if !matches!(
        current_session(),
        MarketSession::Auction | MarketSession::Morning | MarketSession::Afternoon
    ) {
        return true;
    }

    let stats = stock_analysis::monitor::data_quality::DqStats::new();
    let freshness = monitor_freshness_config();
    match stock_analysis::monitor::data_quality::validate_freshness(
        stock_analysis::monitor::data_quality::FreshnessDataType::Quote,
        update_time,
        &freshness,
        &stats,
    ) {
        Ok(()) => true,
        Err(reason) => {
            log::warn!(
                "[DQ_FRESHNESS] rule_id=AGENTS-2.4 data_type=quote action=reject source={} code={} reason={} timestamp={}",
                source,
                code,
                reason.label(),
                chrono::Utc::now().timestamp()
            );
            false
        }
    }
}

fn validate_nav_freshness(nav_date: chrono::NaiveDate) -> bool {
    let stats = stock_analysis::monitor::data_quality::DqStats::new();
    let base = monitor_freshness_config();
    let freshness = stock_analysis::monitor::data_quality::FreshnessConfig {
        quote_max_age_secs: base.quote_max_age_secs,
        position_max_age_secs: base.position_max_age_secs,
        nav_max_age_secs: base.nav_max_age_secs,
        daily_max_age_secs: base.nav_max_age_secs,
    };
    match stock_analysis::monitor::data_quality::validate_daily_freshness(
        nav_date,
        chrono::Local::now(),
        &freshness,
        &stats,
    ) {
        Ok(()) => true,
        Err(reason) => {
            log::warn!(
                "[DQ_FRESHNESS] rule_id=AGENTS-2.4 data_type=nav action=reject reason={} nav_date={} timestamp={}",
                reason.label(),
                nav_date,
                chrono::Utc::now().timestamp()
            );
            false
        }
    }
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

    let quotes = market_data::fetch_position_quotes();
    let mut quote_map: std::collections::HashMap<&str, f64> = std::collections::HashMap::new();
    for q in &quotes { quote_map.insert(q.code.as_str(), q.price); }

    let mut total_value = 0.0_f64;
    let mut counted = 0;
    for p in &positions {
        let price = quote_map.get(p.code.as_str()).copied().unwrap_or(p.cost_price);
        total_value += p.shares as f64 * price;
        counted += 1;
    }

    let prev_curve = stock_analysis::portfolio::get_equity_curve(2).ok().unwrap_or_default();
    if let Some(last) = prev_curve.last() {
        if !validate_nav_freshness(last.date) {
            log::warn!("[净值快照] NAV 数据过期，跳过本次快照");
            return;
        }
    }
    let prev_value = prev_curve.last().map(|e| e.total_value).unwrap_or(total_value);
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
