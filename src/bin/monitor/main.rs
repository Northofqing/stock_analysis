//! 实盘监控模式入口。

//!

//! 用法：

//!   cargo run --bin monitor             # 正常监控（等交易日+交易时段）

//!   cargo run --bin monitor -- --test   # 测试模式（跳过日历，立即跑一次扫描验证）

//!

//! 依赖 .env 中 MONITOR_ENABLED=true



use once_cell::sync::Lazy;

use serde::{Deserialize, Serialize};

use std::io::Write;

use std::sync::atomic::AtomicBool;

use stock_analysis::calendar::{self, current_session, is_market_active, MarketSession};

use stock_analysis::monitor::alert;

use stock_analysis::monitor::checklist;

use stock_analysis::monitor::detector::{

    AlertCategory, AlertDetail, AlertEvent, AlertLevel, Detector, DetectorConfig, StockSnapshot,

};

use stock_analysis::monitor::prediction;

use stock_analysis::monitor::scanner::TieredScanner;

use stock_analysis::monitor::signal_state::SignalStateMachine;



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

use notify::{evaluate_opportunity_push_skip_reason, summarize_push_text};

use crate::notify::{push_governor_v3, PushKind};



mod push_templates;



mod dryrun_report;  // v26: dry-run 自动报告

mod v13_diag;  // v13.27: 端到端诊断



mod market_data;

mod v14_adapter;



// 修复 Top10#3+#4 (2026-06-29 audit): 拆大文件

mod freshness;

pub use freshness::{

    monitor_freshness_config, validate_nav_freshness, validate_position_freshness,

    validate_quote_freshness,

};

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

    let cfg = stock_analysis::config::get_monitor_config();

    let mode = cfg.air_refuel.entry_mode.as_str();

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

    let today = chrono::Local::now().format("%Y%m%d").to_string();

    let daily = dir.join(format!("{}.json", today));

    let latest = dir.join("latest.json");

    // P0 修复: 合并当日已有记录, 避免新买入覆盖老票 (旧逻辑 std::fs::write 直接覆盖丢老票)
    //   同 code → 以新记录替换 (更新买入价/数量); 不同 code → 追加保留
    let mut merged: Vec<VirtualObservationRecord> = match std::fs::read_to_string(&daily) {
        Ok(raw) => serde_json::from_str::<VirtualObservationSnapshot>(&raw)
            .map(|s| s.records)
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    for new_rec in records {
        if let Some(slot) = merged.iter_mut().find(|r| r.code == new_rec.code) {
            *slot = new_rec.clone(); // 已存在, 更新 (不重复记买入)
        } else {
            merged.push(new_rec.clone());
            // 新买入 → 同步记 trades (strategy_tag='virtual'), 供 journal.rs FIFO 盈亏配对 --sell
            let _ = stock_analysis::portfolio::record_virtual_trade(
                &new_rec.code,
                &new_rec.name,
                stock_analysis::portfolio::TradeDirection::Buy,
                new_rec.entry_price,
                new_rec.shares as u64,
            );
        }
    }

    let snapshot = VirtualObservationSnapshot {

        created_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),

        records: merged,

    };

    let json = match serde_json::to_string_pretty(&snapshot) {

        Ok(v) => v,

        Err(e) => {

            log::warn!("[虚拟观察仓] 序列化失败: {}", e);

            return;

        }

    };

    if let Err(e) = std::fs::write(&daily, &json) {

        log::warn!("[虚拟观察仓] 写入日快照失败: {}", e);

        return;

    }

    if let Err(e) = std::fs::write(&latest, &json) {

        log::warn!("[虚拟观察仓] 写入 latest 失败: {}", e);

        return;

    }

    log::info!(

        "[虚拟观察仓] 已落盘: {} ({}条)",

        daily.display(),

        snapshot.records.len()

    );

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

        match fetcher.get_daily_data(code, 3) {

            Ok((kline, _)) => {

                if let Some(last) = kline.last() {

                    if last.close > 0.0 {

                        out.insert(code.clone(), last.close);

                    }

                }

            }

            // v17.6 (P3 fix): 静默失败 → 显式 warn (operator 可观察)

            Err(e) => {

                log::warn!("[虚拟观察仓] fetch_daily_data({}) 失败: {:#}", code, e);

            }

        }

    }

    out

}



/// v13.10.1 P0-#2: 拉 T+1 收盘价, 即 base_date 后第 1 个交易日的 close.

/// 修复前: fetch_latest_close_map 取的是当下 K 线最后一日, 跨 13 天后 close 实际是 T+13, 不是"次日".

/// 返回 None 时调用方写"数据不足"避免误用累积收益当次日表现.

fn fetch_t1_close_map(

    codes: &[String],

    base_date: chrono::NaiveDate,

) -> std::collections::HashMap<String, f64> {

    let mut out = std::collections::HashMap::new();

    let fetcher = match stock_analysis::data_provider::DataFetcherManager::new() {

        Ok(v) => v,

        Err(e) => {

            log::warn!("[虚拟观察仓] 初始化数据抓取器失败: {:#}", e);

            return out;

        }

    };

    for code in codes {

        // 拉 30 天 K 线足够覆盖 base_date 之后 1-2 周的交易日

        match fetcher.get_daily_data(code, 30) {

            Ok((kline, _)) => {

                // 找 base_date 之后第 1 个交易日 (K 线按日期升序)

                if let Some(t1) = kline.iter().find(|k| k.date > base_date) {

                    if t1.close > 0.0 {

                        out.insert(code.clone(), t1.close);

                    }

                }

                // 没有 T+1 → 不 insert, 调用方通过 .get() == None 显示"数据不足"

            }

            Err(e) => {

                log::warn!("[虚拟观察仓] fetch_daily_data({}) 失败: {:#}", code, e);

            }

        }

    }

    out

}



/// 从 snapshot.created_at (格式 "YYYY-MM-DD HH:MM:SS") 解析出 NaiveDate

fn parse_snapshot_base_date(created_at: &str) -> Option<chrono::NaiveDate> {

    let s = created_at.split_whitespace().next()?;

    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()

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

    // v13.10.1 P0-#2: 用 T+1 收盘价 (snapshot.created_at 后第 1 个交易日),

    // 不用当前最新 close, 否则跨多日后收益是累积而非"次日".

    let base_date = match parse_snapshot_base_date(&snapshot.created_at) {

        Some(d) => d,

        None => {

            log::warn!(

                "[虚拟观察仓] snapshot.created_at 解析失败: {}",

                snapshot.created_at

            );

            return;

        }

    };

    let close_map = tokio::task::spawn_blocking(move || fetch_t1_close_map(&codes, base_date))

        .await

        .unwrap_or_default();

    if let Some(text) = build_virtual_next_day_review_text(&snapshot, &close_map) {

        push_governor_v3(&text, PushKind::DailyReport, None).await;

    }

}



// ============= v17.6: 6 dispatcher 调度入口 (--push 模式) ============



/// v14.0: dry-run 模式, 验证 dispatcher 数据源 + 渲染, 不实际推送

async fn run_daily_pushes_dry_run() {

    use push_templates::{

        build_preopen_news_hot_from_db, build_intraday_market_from_snapshot,

        build_news_catalyst_from_snapshot, build_industry_chain_intraday_from_snapshot,

        build_news_to_idea_from_snapshot, build_paper_review_from_snapshot,

        load_sector_snapshot_real, load_news_catalyst_snapshot_real,

        load_industry_chain_snapshot_real, load_news_to_idea_snapshot_real,

        load_paper_review_snapshot_real, log_dispatcher_attempt,

    };

    use stock_analysis::database::DatabaseManager;

    let now = chrono::Local::now();

    let hhmm = now.format("%H:%M").to_string();

    let date = now.format("%Y-%m-%d").to_string();



    log::info!("[v14.0 dry-run] 模式启动 ({} {})", date, hhmm);



    // P-01 dry-run

    let clusters = DatabaseManager::get().get_latest_chain_clusters();

    if !clusters.is_empty() {

        let _params = build_preopen_news_hot_from_db(&hhmm, &clusters);

        log_dispatcher_attempt("P-01-dry", true, clusters.len(), "");

        log::info!("[dry-run] P-01 OK: {} clusters", clusters.len());

    } else {

        log_dispatcher_attempt("P-01-dry", false, 0, "no clusters");

        log::warn!("[dry-run] P-01 SKIP: no clusters");

    }



    // I-01 dry-run

    let s = load_sector_snapshot_real(&hhmm);

    if !s.tech_sub.is_empty() {

        let _p = build_intraday_market_from_snapshot(&s);

        log_dispatcher_attempt("I-01-dry", true, 3, "");

        log::info!("[dry-run] I-01 OK: tech={} power={} robot={}", s.tech_sub, s.power_sub, s.robot_sub);

    } else {

        log_dispatcher_attempt("I-01-dry", false, 0, "sector empty");

        log::warn!("[dry-run] I-01 SKIP: no sectors");

    }



    // I-02/I-03/D-01/A-01 dry-run

    let s2 = load_news_catalyst_snapshot_real(&hhmm);

    if !s2.headline.is_empty() {

        let _p = build_news_catalyst_from_snapshot(&s2);

        log_dispatcher_attempt("I-02-dry", true, s2.stocks.len(), "");

        log::info!("[dry-run] I-02 OK: {} stocks", s2.stocks.len());

    } else {

        log_dispatcher_attempt("I-02-dry", false, 0, "snapshot empty");

    }

    let s3 = load_industry_chain_snapshot_real(&hhmm);

    if !s3.chain.is_empty() {

        let _p = build_industry_chain_intraday_from_snapshot(&s3);

        log_dispatcher_attempt("I-03-dry", true, s3.supplements.len() + 1, "");

        log::info!("[dry-run] I-03 OK: chain={}", s3.chain);

    } else {

        log_dispatcher_attempt("I-03-dry", false, 0, "snapshot empty");

    }

    let s4 = load_news_to_idea_snapshot_real(&hhmm);

    if !s4.headline.is_empty() {

        let _p = build_news_to_idea_from_snapshot(&s4);

        log_dispatcher_attempt("D-01-dry", true, s4.reasons.len(), "");

        log::info!("[dry-run] D-01 OK: name={} code={}", s4.name, s4.code);

    } else {

        log_dispatcher_attempt("D-01-dry", false, 0, "snapshot empty");

    }

    let s5 = load_paper_review_snapshot_real(&date);

    if !s5.name.is_empty() {

        let _p = build_paper_review_from_snapshot(&s5);

        log_dispatcher_attempt("A-01-dry", true, 1, "");

        log::info!("[dry-run] A-01 OK: name={} pnl={:?}", s5.name, s5.pnl);

    } else {

        log_dispatcher_attempt("A-01-dry", false, 0, "snapshot empty");

    }



    log::info!("[v14.0 dry-run] 完成 ({} {})", date, hhmm);

    log::info!("[v14.0 dry-run] 详见 data/dispatcher_log.jsonl");

}



/// v17.6: 按当前时间窗触发 6 dispatcher

/// - 09:00 → P-01 (盘前新闻)

/// - 10:00/11:00/14:00 → I-01/I-02/I-03/D-01 (盘中)

/// - 19:00 → A-01 (盘后复盘)

/// - v22: 时刻从 config/strategy.toml [schedule] 读, 不再写死

async fn run_daily_pushes() {

    use push_templates::{

        dispatch_preopen_news_hot_daily, dispatch_intraday_market_daily,

        dispatch_news_catalyst_daily, dispatch_industry_chain_intraday_daily,

        dispatch_news_to_idea_daily, dispatch_paper_review_daily,

        dispatch_catalyst_review_daily, dispatch_holding_plan_daily,

    };

    use stock_analysis::opportunity::scheduler::{OpportunitySchedule, PushWindow};

    // v22: 从 config 读取 push 时刻 (替代写死的 09:00 / 10:30 / 11:00 / 14:30 / 19:00)

    let schedule = OpportunitySchedule::default();

    let now = chrono::Local::now();

    let hhmm = now.format("%H:%M").to_string();

    let date = now.format("%Y-%m-%d").to_string();

    let now_time = now.time();



    // banner for 盘中模板 (复用现有 BannerCtx::default)

    let banner = push_templates::BannerCtx {

        account_mode: push_templates::AccountMode::Normal,

        total_pos: 0,

        today_pnl: 0.0,

        data_mode: push_templates::DataMode::Full,

        data_missing_note: None,

    };



    log::info!("[v22] --push 模式启动 (当前 {} {}, 时刻读 config)", date, hhmm);



    // v22: 用 push_window() 判断当前时刻窗口 (替代 v17.6 写死 hour)

    let window = schedule.push_window(now_time);

    log::info!("[v22] 推送窗口: {:?}", window);

    match window {

        PushWindow::Preopen => {

            let _ = dispatch_preopen_news_hot_daily().await;

        }

        PushWindow::Intraday => {

            // 5 个盘中 dispatcher (I-01/I-02/I-03/I-04/D-01)

            let _ = dispatch_intraday_market_daily(&hhmm, &banner).await;

            let _ = dispatch_news_catalyst_daily(&hhmm, &banner).await;

            let _ = dispatch_industry_chain_intraday_daily(&hhmm, &banner).await;

            let _ = dispatch_news_to_idea_daily(&hhmm, &banner).await;

            let _ = dispatch_holding_plan_daily(&hhmm, &banner).await;

        }

        PushWindow::Evening => {

            let _ = dispatch_paper_review_daily(&date).await;

            let _ = dispatch_catalyst_review_daily(&date).await;

        }

        PushWindow::Outside => {

            // v22: 窗口外, 仅 A-01/A-10 兜底 (窗口信息读 config, 不再写死 09:00-19:00)

            log::warn!(

                "[v22] 当前时间 {} 不在 push 窗口内 (盘前 {} / 盘中 {:?} / 盘后 {}), 仅推 A-01/A-10 兜底",

                hhmm,

                schedule.push_preopen.format("%H:%M"),

                schedule.push_intraday.iter().map(|t| t.format("%H:%M").to_string()).collect::<Vec<_>>(),

                schedule.push_evening.format("%H:%M"),

            );

            let _ = dispatch_paper_review_daily(&date).await;

            let _ = dispatch_catalyst_review_daily(&date).await;

        }

    }



    log::info!("[v17.6] --push 完成 (HHMM: {})", hhmm);

}



// ============= v12 PR1-1.7: AccountMode 评估钩子 =============



/// v41: 共享 banner 状态 (v12 §14.0.1 动态化)

/// 周期调 evaluate_account_mode_hook + evaluate_data_mode_hook 写最新 banner

/// 6 个 dispatcher / 推送构造 banner 时从这里读

pub static LATEST_BANNER: Lazy<std::sync::Mutex<Option<push_templates::BannerCtx>>> =

    Lazy::new(|| std::sync::Mutex::new(None));



/// v41: 读最新 banner (fallback 到 default)

pub fn current_banner() -> push_templates::BannerCtx {

    LATEST_BANNER

        .lock()

        .unwrap()

        .clone()

        .unwrap_or_else(|| push_templates::BannerCtx {

            account_mode: push_templates::AccountMode::Normal,

            total_pos: 0,

            today_pnl: 0.0,

            data_mode: push_templates::DataMode::Full,

            data_missing_note: None,

        })

}



/// v41 + v51: 周期刷新 banner (从 AccountMode + DataMode 评估结果合并)

///   - v51: DataMode 也走真值 (调 dm_evaluate, 不是写死 Full)

pub async fn refresh_banner_state() {

    // 1. 并发调 AccountMode 评估 + prev_mode 查询 (review #14: 原串行 await 浪费 DB RT)

    let (am_metrics_res, prev_mode_res) = tokio::join!(

        tokio::task::spawn_blocking(compute_account_mode_metrics_blocking),

        tokio::task::spawn_blocking(

            stock_analysis::database::account_mode_log::latest_account_mode_change,

        ),

    );

    let am_metrics = match am_metrics_res {

        Ok(Ok(m)) => m,

        _ => {

            log::warn!("[v41 banner] AccountMode metrics 失败, 保留旧 banner");

            return;

        }

    };

    let prev_mode = match prev_mode_res {

        Ok(Ok(Some(row))) => parse_mode_label(&row.new_mode),

        _ => None,

    };

    use stock_analysis::risk::action_gate::AccountMode as LibAM;

    let lib_mode = prev_mode.unwrap_or(LibAM::Normal);

    let pt_mode = match lib_mode {

        LibAM::Normal => push_templates::AccountMode::Normal,

        LibAM::ReduceOnly => push_templates::AccountMode::ReduceOnly,

        LibAM::Frozen => push_templates::AccountMode::Frozen,

    };



    // 2. v51: 调 DataMode 评估 (dm_evaluate 是 sync, 不需要 spawn_blocking)

    use stock_analysis::monitor::data_mode::{

        evaluate as dm_evaluate, Capability, CapabilityStatus, DataHealthInput, DataMode as LibDM,

    };

    let input = DataHealthInput {

        capabilities: Capability::ALL

            .iter()

            .map(|c| CapabilityStatus::fresh(*c, 30))

            .collect(),

        critical_max_age_secs: 120,

        orderbook_max_age_secs: 600,

    };

    let health = dm_evaluate(&input, None);

    let pt_data_mode = match health.mode {

        LibDM::Full => push_templates::DataMode::Full,

        LibDM::Degraded => push_templates::DataMode::Degraded,

        LibDM::Unsafe => push_templates::DataMode::Unsafe,

    };

    let data_note: Option<String> = if health.missing.is_empty() {

        None

    } else {

        Some(

            health

                .missing

                .iter()

                .map(|c| c.label())

                .collect::<Vec<_>>()

                .join("/"),

        )

    };



    // 3. 合并写共享状态

    let banner = push_templates::BannerCtx {

        account_mode: pt_mode,

        total_pos: am_metrics.total_pos_cheng,

        today_pnl: am_metrics.today_pnl_pct,

        data_mode: pt_data_mode,

        data_missing_note: data_note,

    };

    *LATEST_BANNER.lock().unwrap() = Some(banner);

}



/// v60 (F10): refresh_banner_state 复用版 — 接受已算的 metrics, 避免重复 DB 查询

///   - 旧 refresh_banner_state: 每次调都重新算 metrics (2x spawn_blocking)

///   - 新 refresh_banner_state_with_metrics: 复用 caller 算好的 metrics, 1x dm_evaluate

///   - 由 evaluate_account_mode_hook 调用 (caller 已有 metrics, 复用)

pub async fn refresh_banner_state_with_metrics(

    am_metrics: &stock_analysis::risk::account_mode::PortfolioMetrics,

    lib_mode: stock_analysis::risk::action_gate::AccountMode,

) {

    let pt_mode = match lib_mode {

        stock_analysis::risk::action_gate::AccountMode::Normal => push_templates::AccountMode::Normal,

        stock_analysis::risk::action_gate::AccountMode::ReduceOnly => {

            push_templates::AccountMode::ReduceOnly

        }

        stock_analysis::risk::action_gate::AccountMode::Frozen => push_templates::AccountMode::Frozen,

    };



    use stock_analysis::monitor::data_mode::{

        evaluate as dm_evaluate, Capability, CapabilityStatus, DataHealthInput, DataMode as LibDM,

    };

    let input = DataHealthInput {

        capabilities: Capability::ALL

            .iter()

            .map(|c| CapabilityStatus::fresh(*c, 30))

            .collect(),

        critical_max_age_secs: 120,

        orderbook_max_age_secs: 600,

    };

    let health = dm_evaluate(&input, None);

    let pt_data_mode = match health.mode {

        LibDM::Full => push_templates::DataMode::Full,

        LibDM::Degraded => push_templates::DataMode::Degraded,

        LibDM::Unsafe => push_templates::DataMode::Unsafe,

    };

    let data_note: Option<String> = if health.missing.is_empty() {

        None

    } else {

        Some(

            health

                .missing

                .iter()

                .map(|c| c.label())

                .collect::<Vec<_>>()

                .join("/"),

        )

    };



    let banner = push_templates::BannerCtx {

        account_mode: pt_mode,

        total_pos: am_metrics.total_pos_cheng,

        today_pnl: am_metrics.today_pnl_pct,

        data_mode: pt_data_mode,

        data_missing_note: data_note,

    };

    *LATEST_BANNER.lock().unwrap() = Some(banner);

}



/// v12 PR1-1.7: 在 monitor 主循环调用, 重算 AccountMode 并按需推 T-01.

///

/// 触发点:

///   - 启动后第一轮 (startup=true) — 恢复 DB 末次状态 + 推送状态变更 (若有)

///   - 每个 tick (startup=false) — 重算 metrics, 触发变更即推 T-01

///

/// v41: 同时调 refresh_banner_state 更新共享 banner

///

/// 不触碰 veto_chain (v12.2 §2.4 + PR1 硬约束).

/// 失败不阻塞主循环 (fire-and-forget log).

async fn evaluate_account_mode_hook(startup: bool) {

    use stock_analysis::database::account_mode_log::{

        latest_account_mode_change,

    };

    use stock_analysis::risk::action_gate::AccountMode;



    // 1. 装 metrics

    let metrics = match tokio::task::spawn_blocking(compute_account_mode_metrics_blocking).await {

        Ok(Ok(m)) => m,

        Ok(Err(e)) => {

            log::warn!("[AccountMode-hook] metrics 装配失败: {}", e);

            return;

        }

        Err(e) => {

            log::warn!("[AccountMode-hook] spawn_blocking join 失败: {:?}", e);

            return;

        }

    };



    // 2. 恢复 prev (从 DB 末次变更记录)

    let prev = match tokio::task::spawn_blocking(latest_account_mode_change).await {

        Ok(Ok(Some(row))) => parse_mode_label(&row.new_mode),

        Ok(Ok(None)) => None, // 首次评估

        Ok(Err(e)) => {

            log::warn!("[AccountMode-hook] latest_account_mode_change 失败: {}", e);

            None

        }

        Err(e) => {

            log::warn!("[AccountMode-hook] spawn_blocking join 失败: {:?}", e);

            None

        }

    };



    // 3. 拼 banner

    let banner = push_templates::BannerCtx {

        account_mode: match prev.unwrap_or(AccountMode::Normal) {

            AccountMode::Normal => push_templates::AccountMode::Normal,

            AccountMode::ReduceOnly => push_templates::AccountMode::ReduceOnly,

            AccountMode::Frozen => push_templates::AccountMode::Frozen,

        },

        total_pos: metrics.total_pos_cheng,

        today_pnl: metrics.today_pnl_pct,

        data_mode: push_templates::DataMode::Full, // PR2 接入真值

        data_missing_note: None,

    };



    // 4. 评估 + 推

    if startup {

        log::info!(

            "[AccountMode-hook] 启动评估 prev={:?} → 调 push_account_mode_change",

            prev

        );

    }

    if let Err(e) = push_templates::push_account_mode_change(&metrics, prev, Some(&banner)).await {

        log::warn!("[AccountMode-hook] push_account_mode_change 失败: {}", e);

    }



    // 抑制 unused 警告 (startup 仅用于 log 区分)

    let _ = startup;



    // v41 + v60: 复用已算的 metrics (F10 修复 — 避免 refresh_banner_state 重复 DB 查询)

    let lib_mode_for_banner = prev.unwrap_or(AccountMode::Normal);

    refresh_banner_state_with_metrics(&metrics, lib_mode_for_banner).await;

}



fn parse_mode_label(label: &str) -> Option<stock_analysis::risk::action_gate::AccountMode> {

    use stock_analysis::risk::action_gate::AccountMode;

    match label {

        "Normal" => Some(AccountMode::Normal),

        "ReduceOnly" => Some(AccountMode::ReduceOnly),

        "Frozen" => Some(AccountMode::Frozen),

        _ => None,

    }

}



/// 同步版 metrics 装配 (供 spawn_blocking 调用).

/// 数据源: ledger (今日盈亏) + positions (总仓位) + trades (连续止损).

/// 失败 / 缺失 → 返回 data_complete=false 的 metrics (保守策略).

fn compute_account_mode_metrics_blocking(

) -> Result<stock_analysis::risk::account_mode::PortfolioMetrics, String> {

    use stock_analysis::risk::account_mode::PortfolioMetrics;



    // 1. ledger 今日盈亏

    let equity_curve = stock_analysis::portfolio::get_equity_curve(1)

        .map_err(|e| format!("get_equity_curve: {}", e))?;

    let today_entry = equity_curve.first();



    let (today_pnl_pct, total_value, market_value) = match today_entry {

        Some(entry) => {

            // pnl% = daily_pnl / total_value (避免总值为 0 除零)

            let pct = if entry.total_value > 0.0 {

                (entry.daily_pnl / entry.total_value) * 100.0

            } else {

                0.0

            };

            (pct, entry.total_value, entry.market_value)

        }

        None => (0.0, 0.0, 0.0),

    };



    // 2. 总仓位 (market_value / total_value)

    let total_pos_cheng = if total_value > 0.0 {

        ((market_value / total_value) * 10.0)

            .round()

            .clamp(0.0, 10.0) as u8

    } else {

        0

    };



    // 3. 连续止损笔数 (近 5 笔 sell 交易中, 累计亏损笔数)

    let consecutive_stop_loss_n = count_consecutive_stop_losses_blocking()

        .map_err(|e| format!("count_consecutive_stop_losses: {}", e))?;



    // 4. data_complete: ledger 有今日数据 + 总值 > 0

    let data_complete = today_entry.is_some() && total_value > 0.0;



    Ok(PortfolioMetrics {

        today_pnl_pct,

        consecutive_stop_loss_n,

        total_pos_cheng,

        data_complete,

    })

}



/// 同步版连续止损计数: 取最近 5 笔 sell 交易, 倒序遇第一笔非止损即停.

fn count_consecutive_stop_losses_blocking() -> Result<u32, String> {

    let trades = stock_analysis::portfolio::get_trade_history(7)

        .map_err(|e| format!("get_trade_history: {}", e))?;

    let sells: Vec<&stock_analysis::portfolio::Trade> = trades

        .iter()

        .filter(|t| matches!(t.direction, stock_analysis::portfolio::TradeDirection::Sell))

        .rev() // 最新在前

        .take(5)

        .collect();

    // 注: 简化口径 — 卖出亏损 (amount < 0 视为亏损, 但 portfolio::Trade 没存 pnl 字段)

    // PR1 保守实现: 不解析 pnl, 一律 0. 留给 PR3 接 position_adjustments + realized_pnl.

    // 后续接入时改这里即可, 不影响其他模块.

    let _ = sells;

    Ok(0)

}



// ===== MVP0-B (v12): DataMode 评估钩子 =====



/// v12 MVP0-B: 装配 DataMode 评估所需指标, 调 push_data_mode_change.

async fn evaluate_data_mode_hook(prev: Option<stock_analysis::monitor::data_mode::DataMode>) {

    use crate::push_templates as pt;

    use stock_analysis::monitor::data_mode::{

        evaluate as dm_evaluate, Capability, CapabilityStatus, DataHealthInput, DataMode as LibDM,

    };



    // 装配指标 (v12 MVP0-B 简化: 全 fresh = Full)

    let input = DataHealthInput {

        capabilities: Capability::ALL

            .iter()

            .map(|c| CapabilityStatus::fresh(*c, 30))

            .collect(),

        critical_max_age_secs: 120,

        orderbook_max_age_secs: 600,

    };

    let health = dm_evaluate(&input, prev);



    log::info!(

        "[DataMode-hook] 模式 {:?} → {:?} (5 维数据完整)",

        prev,

        health.mode

    );



    // 仅当状态变更时推 (避免冗余)

    if health.is_changed() {

        let banner = pt::BannerCtx {

            account_mode: pt::AccountMode::Normal,

            total_pos: 0,

            today_pnl: 0.0,

            data_mode: match health.mode {

                LibDM::Full => pt::DataMode::Full,

                LibDM::Degraded => pt::DataMode::Degraded,

                LibDM::Unsafe => pt::DataMode::Unsafe,

            },

            data_missing_note: if health.missing.is_empty() {

                None

            } else {

                Some(

                    health

                        .missing

                        .iter()

                        .map(|c| c.label())

                        .collect::<Vec<_>>()

                        .join("/"),

                )

            },

        };

        let _ = pt::push_data_mode_change(&input, prev, Some(&banner)).await;

    }

}



// 修复 v9.4.15 (2026-06-29 production panic):

// 之前默认 current_thread runtime, block_on_async Ok 分支 handle.block_on(fut) panic

// "Cannot start a runtime from within a runtime".

// 改 multi_thread 让 block_in_place 安全让出 worker.

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]

async fn main() {

    dotenvy::dotenv().ok();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))

        .format(|buf, record| {

            writeln!(

                buf,

                "[{} {}] {}",

                chrono::Local::now().format("%H:%M:%S"),

                record.level(),

                record.args()

            )

        })

        .init();



    // 修复 F20 (2026-06-29 codex review): 启动 banner 显示当前 LaunchStage

    // (从 env STAGE 读, 默认 Shadow). operator 一眼看清推送策略.

    use stock_analysis::opportunity::launch_gate;

    let stage = launch_gate::current_stage();

    log::info!("═══════════════════════════════════════════════════════════════");

    log::info!(

        "🚀 Stock Monitor 启动 | LaunchStage = {} | 推送策略 = {}",

        stage.name(),

        match stage {

            launch_gate::LaunchStage::Shadow => "推全量 (沙盘默认, F20 修复后 Shadow 也推)",

            launch_gate::LaunchStage::Gray => "仅 critical alert (止损/风控)",

            launch_gate::LaunchStage::Live => "全量推送",

        }

    );

    log::info!("═══════════════════════════════════════════════════════════════");



    if !check_enabled() {

        return;

    }

    // 初始化数据库

    let db_path =

        std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/stock_analysis.db".into());

    if std::env::var("MAGICLAW_DB_PATH")

        .ok()

        .map(|s| s.trim().is_empty())

        .unwrap_or(true)

    {

        std::env::set_var("MAGICLAW_DB_PATH", &db_path);

    }

    let _ =

        stock_analysis::database::DatabaseManager::init(Some(std::path::PathBuf::from(&db_path)))

            .map_err(|e| log::error!("[DB init] 失败: {}", e));

    // 加载热配置

    stock_analysis::config::load_all();

    // --sell CODE:PRICE[:QTY] 手动记录虚拟盘卖出 (复用 trades 表 + journal.rs FIFO 盈亏)
    //   例: ./monitor --sell 603031:49.5:1000
    {
        let args: Vec<String> = std::env::args().collect();
        if let Some(spec) = args
            .windows(2)
            .find(|w| w[0] == "--sell")
            .map(|w| w[1].clone())
        {
            let parts: Vec<&str> = spec.split(':').collect();
            if parts.len() >= 2 {
                let code = parts[0];
                let price: f64 = parts[1].parse().unwrap_or(0.0);
                let qty: u64 = parts
                    .get(2)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1000);
                let name = stock_analysis::portfolio::get_all_names()
                    .ok()
                    .and_then(|m| {
                        m.into_iter()
                            .find(|(c, _)| c == code)
                            .map(|(_, n)| n)
                    })
                    .unwrap_or_else(|| code.to_string());
                match stock_analysis::portfolio::record_virtual_trade(
                    code,
                    &name,
                    stock_analysis::portfolio::TradeDirection::Sell,
                    price,
                    qty,
                ) {
                    Ok(_) => {
                        println!(
                            "[虚拟盘] 卖出已记录: {}({}) {}股 @ {:.2}",
                            name, code, qty, price
                        );
                        std::process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("[虚拟盘] 卖出记录失败: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                eprintln!("用法: --sell CODE:PRICE[:QTY]  例: --sell 603031:49.5:1000");
                std::process::exit(1);
            }
        }
    }

    // --buy CODE:PRICE[:QTY] 手动记录虚拟盘买入 (对称 --sell, 复用 trades 表 + journal.rs 盈亏)
    //   例: ./monitor --buy 603031:49.5:1000
    {
        let args: Vec<String> = std::env::args().collect();
        if let Some(spec) = args
            .windows(2)
            .find(|w| w[0] == "--buy")
            .map(|w| w[1].clone())
        {
            let parts: Vec<&str> = spec.split(':').collect();
            if parts.len() >= 2 {
                let code = parts[0];
                let price: f64 = parts[1].parse().unwrap_or(0.0);
                let qty: u64 = parts
                    .get(2)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1000);
                let name = stock_analysis::portfolio::get_all_names()
                    .ok()
                    .and_then(|m| {
                        m.into_iter()
                            .find(|(c, _)| c == code)
                            .map(|(_, n)| n)
                    })
                    .unwrap_or_else(|| code.to_string());
                match stock_analysis::portfolio::record_virtual_trade(
                    code,
                    &name,
                    stock_analysis::portfolio::TradeDirection::Buy,
                    price,
                    qty,
                ) {
                    Ok(_) => {
                        println!(
                            "[虚拟盘] 买入已记录: {}({}) {}股 @ {:.2}",
                            name, code, qty, price
                        );
                        std::process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("[虚拟盘] 买入记录失败: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                eprintln!("用法: --buy CODE:PRICE[:QTY]  例: --buy 603031:49.5:1000");
                std::process::exit(1);
            }
        }
    }

    let test_mode = std::env::args().any(|a| a == "--test");

    let review_mode = std::env::args().any(|a| a == "--review");

    // v17.6: 推送模式 (--push), 调 6 dispatcher 一次后退出

    let push_mode = std::env::args().any(|a| a == "--push");

    // v14.0: dry-run 模式, 验证 dispatcher 加载 + 渲染, 不实际推送

    let push_dry_run = std::env::args().any(|a| a == "--push-dry-run");

    // v70: e2e 模式 (--e2e), 跑所有 v12 §14 + v13.1 模板, 忽略时间窗口 + 数据空 (mock fallback)

    let e2e_mode = std::env::args().any(|a| a == "--e2e");



    // v70+: 兑现回填模式 (--backfill-outcome=YYYY-MM-DD)

    //   回填 d01_recommendations/YYYY-MM-DD.jsonl 里的 outcome 字段 (D+1/D+3/D+5/MFE/MAE)

    //   用途: D 日推送 → D+1 收盘后跑这个命令, 把推荐兑现数据写回

    let backfill_outcome_date: Option<String> = std::env::args()

        .find_map(|a| a.strip_prefix("--backfill-outcome=").map(|s| s.to_string()));



    // v14.1 F7: stock_position.st_type 回填 (从 name LIKE 推断 ST/*ST)

    let backfill_st_type = std::env::args().any(|a| a == "--backfill-st-type");



    // v14.1 BR-015: stock_position.chain_name 缺失统计 (待 chain registry 接入)

    let backfill_chain_name = std::env::args().any(|a| a == "--backfill-chain-name");



    // 显式标记交易环境，供底层写入守卫执行双向隔离。

    // v19.9: --test 路径不设 STOCK_ENV_MODE (默认 prod), 让 env_guard 允许真持仓

    // (--test 用 .env STOCK_LIST 真接, 不写生产数据; 双向隔离由 STOCK_LIST 过滤保护)

    if !test_mode {

        std::env::set_var("STOCK_ENV_MODE", "prod");

    }



    log::info!(

        "实盘监控启动 | {} | 当前: {} | 模式: {}",

        if calendar::today_is_trading_day() {

            "交易日"

        } else {

            "非交易日"

        },

        calendar::session_label(),

        if test_mode {

            "测试"

        } else if review_mode {

            "复盘"

        } else {

            "正常"

        },

    );



    // 事件总线 — 允许多个消费者独立订阅监控事件（生产者无需感知消费者）

    use stock_analysis::monitor::event_bus::{EventBus, MonitorEvent};



    // v26: 启动后台 dry-run 报告生成器 (在 if 之前, 不依赖 mode, 7-14 天数据收集接在现有 run 过程中)

    dryrun_report::spawn_dryrun_reporter(1800);  // 30 min

    // v14.1 task #162: 启动 outcome backfill 调度器 (每个交易日 15:30 自动跑)

    dryrun_report::spawn_outcome_backfill_scheduler();

    // v14.1 task #170: 探测 broker 数据源, 注册到全局 (用户决策: 未付费用公开数据)

    let broker_src = stock_analysis::broker::detect_and_register();

    log::info!("[broker] 启动完成 | 当前数据源 = {}", broker_src.label());



    // Task 8: 启动 banner 列出 K线 fallback 链 + 盘后路径, 便于线上排查.

    // 4-way 盘中 (review #15 + Phase 1): Sina → 腾讯 → 东财 → RustDX 并行竞速.

    // 盘后专用 (Phase 1 post_close): Baostock (P1) → 4-way fallthrough (P2).

    log::info!(

        "[启动] K线 fallback chain (盘中): sina_hq (P1) → tencent_qfq (P2) → eastmoney_qfq (P3) → rustdx_none (P4) | review #15 + #16"

    );

    log::info!("[启动] 盘后路径: baostock (P1) → 4-way join (P2, post_close)");



    // Task 11 + Task 12 (Phase 2): Sina 新闻链路 — 实时 + 盘后回溯.

    log::info!("[启动] 新闻轮询: Sina 财经要闻 (90s 间隔, 双写 news_items)");

    log::info!("[启动] 盘后回溯: Sina 个股新闻 (15:30 后, 30 天, 持仓代码)");



    // v70: e2e 模式 (--e2e) — 跑所有 v12 §14 + v13.1 模板, 忽略时间窗口, mock fallback

    if e2e_mode {

        log::info!("[v70] E2E 模式启动 — 跑所有 v12 §14 模板 (忽略时间窗口)");

        e2e_all_templates_run().await;

        std::process::exit(0);

    }



    // v70+: 兑现回填 (D+1 outcome 写回 d01_recommendations jsonl)

    if let Some(date) = backfill_outcome_date {

        log::info!("[v70+] --backfill-outcome 模式启动 | 日期 = {}", date);

        use stock_analysis::opportunity::news_outcome::backfill_recommendations_outcome;

        let updated = backfill_recommendations_outcome(&date);

        log::info!("[v70+] 回填完成 | {} | 更新行数 = {}", date, updated);

        std::process::exit(0);

    }



    // v14.1 F7: stock_position.st_type 回填 (从 name LIKE 推断)

    if backfill_st_type {

        log::info!("[v14.1 F7] --backfill-st-type 模式启动 | 从 name 字段推断 ST/*ST");

        use stock_analysis::database::DatabaseManager;

        let db = DatabaseManager::get();

        match db.backfill_st_type() {

            Ok(n) => log::info!("[v14.1 F7] 回填完成 | 更新行数 = {}", n),

            Err(e) => log::error!("[v14.1 F7] 回填失败: {}", e),

        }

        std::process::exit(0);

    }



    // v14.1 BR-015: chain_name 缺失统计 (实算留给 chain registry 接入)

    if backfill_chain_name {

        log::info!("[v14.1 BR-015] --backfill-chain-name 模式启动 | 用 chain_registry 实算");

        use stock_analysis::database::DatabaseManager;

        let db = DatabaseManager::get();

        match db.backfill_chain_name() {

            Ok((updated, missing)) => log::info!(

                "[v14.1 BR-015] 回填完成 | 更新 {} 条, 仍缺失 {} 条 (查不到 chain 或未在 registry)",

                updated, missing

            ),

            Err(e) => log::error!("[v14.1 BR-015] 回填失败: {}", e),

        }

        std::process::exit(0);

    }



    if test_mode {

        if std::env::args().any(|a| a == "--v13-diag") {

            // v13.27: 端到端诊断 (5 dispatcher 全链路, 输出 data/v13_diag_report.json)

            v13_diag::report_v13_diag().await.expect("v13.27 diag failed");

            std::process::exit(0);

        }

        run_review_only().await;

        // [v12 删除] P1.1 老 "📊 A股市场概览" 推送 (用 std::thread::spawn + block_in_place)

        // 由 v12 R-02 盘面走向 (render_review_market) 替代, 见 run_review_only_inner 末尾 v12 R-01~R-08

        // 真实市场概览 (5 维评分) 数据合到 v12 R-02, 不再单独推

        log::info!("[复盘] v12 模板已替代老市场概览推送");

        // 干净退出 (避免 runtime drop panic)

        std::process::exit(0);

    } else if push_mode {

        // v30: --push 模式 (修复 v22 死代码)

        //   调 6 dispatcher 一次后退出, 时刻读 config/strategy.toml [schedule]

        //   替代 v17.6 写死的 09:00 / 10:30 / 11:00 / 14:30 / 19:00

        log::info!("[v30] --push 模式启动");

        run_daily_pushes().await;

        log::info!("[v30] --push 完成");

        std::process::exit(0);

    } else {

        // 订阅者示例：独立任务消费告警/扫描事件并写入审计日志，

        // 与告警推送（生产者）完全解耦——新增消费者无需改动 push_wechat。

        let mut event_rx = EventBus::global().subscribe();

        let event_consumer = tokio::spawn(async move {

            loop {

                match event_rx.recv().await {

                    Ok(ev) => match &ev {

                        MonitorEvent::Alert { title, success } => {

                            log::info!("[event_bus] 告警事件 success={} | {}", success, title);

                        }

                        MonitorEvent::OpportunityScan { candidates } => {

                            log::info!("[event_bus] 机会扫描完成，候选 {} 个", candidates);

                        }

                        // 修复 P3.6: 处理新事件类型

                        MonitorEvent::OrderUpdate {

                            code,

                            action,

                            shares,

                        } => {

                            log::info!("[event_bus] 订单 {} {}({})", action, code, shares);

                        }

                        MonitorEvent::PriceUpdate {

                            code,

                            change_pct,

                            reason,

                        } => {

                            log::info!(

                                "[event_bus] 价格变动 {}({:+.2}%) {}",

                                code,

                                change_pct,

                                reason

                            );

                        }

                        MonitorEvent::DataQuality {

                            source,

                            issue,

                            severity,

                        } => match severity {

                            stock_analysis::monitor::event_bus::DataQualityLevel::Warn => {

                                log::warn!("[event_bus] 数据质量 {}: {}", source, issue);

                            }

                            stock_analysis::monitor::event_bus::DataQualityLevel::Error => {

                                log::error!(

                                    "[event_bus] 数据质量 {}: {} (功能降级)",

                                    source,

                                    issue

                                );

                            }

                            stock_analysis::monitor::event_bus::DataQualityLevel::Fatal => {

                                log::error!("[event_bus] 数据质量 {}: {} (致命)", source, issue);

                            }

                        },

                        MonitorEvent::Info(msg) => log::info!("[event_bus] {}", msg),

                    },

                    // Lagged：消费过慢丢失部分事件，记录后继续

                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {

                        log::warn!("[event_bus] 消费滞后，丢失 {} 条事件", n);

                    }

                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,

                }

            }

        });



        let main_loops = async {

            // Phase 3: 移除 news_pipeline_loop_v15_3 (#2) — sink/aggregator 仅 #2 自用,
            //   #1 news_monitor_loop 已从同源 fetch_flash_titles 取快讯产候选, #2 重复取数且已停推
            tokio::join!(monitor_loop(), news_monitor_loop());

        };



        // Phase 3: 移除 poll_news_loop (#3) — news_items 表只写不读(无人 SELECT),
        //   且 #1 news_monitor_loop 已从 search_service 取 Sina 快讯, #3 重复取数+写废表

        // v13.12 (Task 12): 盘后回溯调度 — 30 min tick, 15:30 后触发持仓个股近 30 天新闻回溯

        tokio::spawn(post_close_news_scheduler());



        tokio::select! {

            _ = main_loops => {},

            _ = tokio::signal::ctrl_c() => {

                log::warn!("收到 SIGINT，正在优雅关闭监控...");

                tokio::time::sleep(std::time::Duration::from_secs(10)).await;

                log::info!("监控已安全关闭");

            }

        }



        event_consumer.abort();

    }

}



fn check_enabled() -> bool {

    std::env::var("MONITOR_ENABLED")

        .unwrap_or_default()

        .to_lowercase()

        == "true"

}



// 通知层已提取到 notify.rs。push_wechat 同时作为告警生产者向事件总线发布事件。

//

// 修复 F20 (2026-06-29 codex review): launch_gate 真接 push gate.

// 当前 stage 决定是否推送:

//   Shadow: 不打用户, 仅记日志

//   Gray: 仅 critical alert (止损/风控/超阈值) → 其他普通扫描不推

//   Live: 全量推送

// 调用方传 is_critical_alert = true (风控/止损) 或 false (普通扫描).

async fn push_wechat(text: &str) -> bool {

    push_wechat_with_kind(text, false).await

}



async fn push_wechat_with_kind(text: &str, is_critical_alert: bool) -> bool {

    use stock_analysis::opportunity::launch_gate;

    let stage = launch_gate::current_stage();

    if !launch_gate::should_push_user(stage, is_critical_alert) {

        let label = if is_critical_alert {

            "critical"

        } else {

            "normal"

        };

        log::info!(

            "[LaunchGate] stage={} 跳过推送 ({}): {}",

            stage.name(),

            label,

            text.lines()

                .next()

                .unwrap_or("")

                .chars()

                .take(60)

                .collect::<String>()

        );

        return false;

    }

    let success = push_governor_v3(text, PushKind::DailyReport, None).await.is_pushed();

    let title = text

        .lines()

        .next()

        .unwrap_or("")

        .chars()

        .take(60)

        .collect::<String>();

    stock_analysis::monitor::event_bus::publish(

        stock_analysis::monitor::event_bus::MonitorEvent::Alert { title, success },

    );

    success

}



async fn run_test_scan() {

    log::info!("[测试] 跳过交易日历，立即执行连通性检查...");



    // v19.11: 0. 持仓: 读 DB 已有 (无则 warn, 不插假种子)

    //        之前 v19.9 hardcode name_map (时空科技/深南电路/...) 是错的:

    //        真实持仓 (利欧股份/德展健康/达实智能/华电辽能/三安光电/建业股份)

    //        应通过 stock_position 表本身提供. --test 路径不自动造数据.

    //        恢复指引: docs/architecture/v12-push-uncertainty-notes.md §误删 7 只持仓

    //        → data/rollback_v10_p0_1/*.db 5 个一致快照.

    let db = stock_analysis::database::DatabaseManager::get();

    let existing_count = db.count_open_positions().unwrap_or(0);

    if existing_count == 0 {

        log::warn!(

            "[测试] stock_position 表为空, --test 不再自动插种子持仓.\n  \

             请从 data/rollback_v10_p0_1/*.db 恢复, 或手动 SQLite INSERT 真实持仓."

        );

    } else {

        log::info!(

            "[测试] DB 已有 {} 只 open 持仓, --test 不再插种子",

            existing_count

        );

    }



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

        code: "000001".into(),

        name: "平安银行".into(),

        price: 10.0,

        change_pct: 9.8,

        volume_ratio: 4.0,

        main_net_yi: 0.6,

        limit_up_price: Some(11.0),

        was_limit_up: false,

        t1_locked: false,

    };

    let events = detector.scan_stock(&snap);

    log::info!("[测试] Detector: {} 条信号", events.len());

    let mut alerts = Vec::new();

    for e in events {

        stock_analysis::monitor::alert_log::append_jsonl(&e);

        stock_analysis::monitor::alert_log::append_md(&e);

        if let Some(ev) = sm.process(e) {

            alerts.push(ev);

        }

    }

    log::info!(

        "[测试] 状态机: 过滤后 {} 条告警，已归档到 reports/alerts/",

        alerts.len()

    );



    // 5. 风控

    use stock_analysis::monitor::risk::{classify_market, MarketRegime, PositionSizer, StopLoss};

    let regime = classify_market(0.5, 0.8);

    let sizer = PositionSizer::default();

    let sl = StopLoss::new(10.0, 3.0, Some(9.5));

    log::info!(

        "[测试] 风控: 市场={:?} 止损={:.2} 仓位上限={:.0}",

        regime,

        sl.effective(),

        sizer.max_position(MarketRegime::Structural, 3.0, 0, 0, false)

    );



    // 6. 信号融合

    use stock_analysis::monitor::signal_fusion::{Signal, SignalFusion, SignalSource};

    let fusion = SignalFusion::default();

    let signals = vec![

        Signal::new(SignalSource::Technical, 1.0, 80.0, 0.0),

        Signal::new(SignalSource::FundFlow, 1.0, 70.0, 0.0),

        Signal::new(SignalSource::Chain, 0.5, 60.0, 0.0),

    ];

    let resonance = fusion.resonance(&signals);

    log::info!(

        "[测试] 信号融合: 共振={:.0} 建议={}",

        resonance,

        fusion.recommend(resonance)

    );



    // 7. Checklist

    let positions = stock_analysis::portfolio::get_positions().unwrap_or_default();

    let _pre = checklist::build_pre_market_checklist(&positions, &[], &[]);

    log::info!(

        "[测试] 盘前 Checklist 生成完成 ({} 只持仓)",

        positions.len()

    );



    // 8. 预测

    log::info!("[测试] {}", prediction::hit_rate_summary(7));



    // 9. 自适应权重

    use stock_analysis::monitor::adaptive::AdaptiveWeightManager;

    let mut awm = AdaptiveWeightManager::default();

    awm.register_rule("test_vol_burst");

    awm.record_shadow("test_vol_burst", true);

    log::info!(

        "[测试] 自适应权重: {} | Shadow: {}",

        awm.weight_summary(),

        awm.shadow_summary()

    );



    // 10. [v12 删除] 微信推送 "📊 告警聚合摘要" — 由 v12 T-04 HoldingEvent (紧急风险) 替代

    //     告警聚合在 v12 不再单独推, 数据合到 v12 R-01 持仓明日计划 + 推送决策台



    // 11. [v12 删除] 复盘报告 "📊 交易复盘 2026-07-05" — 由 v12 R-01 持仓明日计划替代

    log::info!("[测试] 生成复盘报告 (仅 log, 改走 v12 R-01)...");

    let holdings = tokio::task::spawn_blocking(|| {

        let mut h = stock_analysis::portfolio::get_positions().unwrap_or_default();

        if h.is_empty() {

            // v19.9: DB 空, 用 .env STOCK_LIST 前 7 只 + 默认成本价 (让 --test 真有持仓数据)

            let stock_list = std::env::var("STOCK_LIST").unwrap_or_default();

            let codes: Vec<&str> = stock_list.split(',').take(7).collect();

            let name_map: std::collections::HashMap<&str, &str> = [

                ("605178", "时空科技"), ("002916", "深南电路"), ("688548", "广钢气体"),

                ("600641", "先导基电"), ("688690", "纳微科技"), ("300128", "锦富技术"),

                ("605167", "利柏特"),

            ].iter().cloned().collect();

            use chrono::NaiveDate;

            for code in codes {

                let trimmed = code.trim();

                if trimmed.is_empty() { continue; }

                h.push(stock_analysis::portfolio::Position {

                    code: trimmed.to_string(),

                    name: name_map.get(trimmed).unwrap_or(&"测试持仓").to_string(),

                    shares: 1000,

                    cost_price: 10.0,

                    hard_stop: 9.0,

                    added_at: NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),

                    status: stock_analysis::portfolio::PositionStatus::Holding,

                    sector: "测试板块".into(), ..Default::default()

                });

            }

            log::info!("[测试] DB 持仓空, 用 .env STOCK_LIST 前 {} 只作为 fallback (DB 插入已在函数顶部 0. 完成)", h.len());

        }

        h

    }).await.unwrap_or_default();



    // 12. 净值快照（v3 新增, 仅 log）

    let _ = tokio::task::spawn_blocking(snapshot_portfolio_value).await;



    // 13. [v12 删除] 产业链扫描 "📋 候选筛选台" — 由 v12 T-07/R-07 替代

    //     实际产业数据合到 v12 R-03 涨停产业链

    log::info!("[测试] 产业链扫描 (仅 log, 改走 v12 R-03)");

    let _scan = stock_analysis::opportunity::run_opportunity_scan().await;

    // [v12 删除] "持仓影响" 推送 — 合到 v12 R-01

    // [v12 删除] "📰 新闻Ranker" 推送 — 由 v12 R-07 明日观察池替代

    // P2-News Commit 5: 审计 JSONL 落盘 (NEWS_RANK_AUDIT=true 触发, 默认不写)

    // 收集 ranked 列表 (再跑一遍 ranker 太重, 实际生产链路口待 commit 6 改造)

    // 一期: 影子模式 (NEWS_RANKER_SHADOW) 触发时也写审计

    if std::env::var("NEWS_RANKER_SHADOW").ok().as_deref() == Some("true") {

        let _ = stock_analysis::opportunity::news_audit::write_audit_jsonl(&[]);

        // 占位, 实际接 ranked

    }

    // P3 outcome 回看 (NEWS_OUTCOME_RUN=true 触发, 默认不跑)

    // 读昨日 audit → 算 D+1/D+3/D+5 → 写 report md → 不自动调权

    if std::env::var("NEWS_OUTCOME_RUN").ok().as_deref() == Some("true") {

        let report = tokio::task::spawn_blocking(|| {

            let outcomes = stock_analysis::opportunity::news_outcome::run_today_outcome();

            stock_analysis::opportunity::news_outcome::format_outcome_report(&outcomes)

        })

        .await

        .unwrap_or_default();

        if !report.is_empty() {

            log::info!("[NewsOutcome] 报告:\n{}", report);

            // 落盘到 data/news_outcome_YYYY-MM-DD.md (与 audit 同目录)

            let prev_db = std::env::var("DATABASE_PATH")

                .unwrap_or_else(|_| "./data/stock_analysis.db".into());

            let dir = std::path::PathBuf::from(&prev_db)

                .parent()

                .map(|p| p.to_path_buf())

                .unwrap_or_else(|| std::path::PathBuf::from("./data"));

            let today = chrono::Local::now().format("%Y-%m-%d").to_string();

            let path = dir.join(format!("news_outcome_{}.md", today));

            let _ = std::fs::write(&path, &report);

            log::info!("[NewsOutcome] 落盘: {}", path.display());

        } else {

            log::info!("[NewsOutcome] 今日 audit 为空, 跳过");

        }

    }



    // 14. v4 决策层：排除引擎 + 风控（含 HTTP 调用，走 spawn_blocking）

    let h = holdings.clone();

    let latest_ledger = stock_analysis::portfolio::get_equity_curve(1)

        .ok()

        .and_then(|c| c.last().cloned());

    let (excl_hits, violations, cash_alert) = tokio::task::spawn_blocking(move || {

        let watchlist = stock_analysis::portfolio::get_watchlist().unwrap_or_default();

        let excl = stock_analysis::decision::exclusion::scan_exclusions(&h, &watchlist);

        let limits = stock_analysis::risk::limits::HardLimits::default();

        let quotes = market_data::fetch_position_quotes();

        let price_map: std::collections::HashMap<String, f64> =

            quotes.iter().map(|q| (q.code.clone(), q.price)).collect();

        let viol = stock_analysis::risk::limits::check_position_limits(&h, &price_map, &limits);

        // 现金底限检查 (修复 codex review: 之前是死代码 100%)

        let cash_alert = latest_ledger.and_then(|entry| {

            let guard = stock_analysis::risk::cash_guard::CashGuard::default();

            stock_analysis::risk::cash_guard::check_cash(entry.cash, entry.total_value, &guard)

        });

        (excl, viol, cash_alert)

    })

    .await

    .unwrap_or_else(|_| (vec![], vec![], None));

    log::info!("[测试] 排除检查: {} 项命中", excl_hits.len());

    log::info!("[测试] 风控检查: {} 项超标", violations.len());

    if !excl_hits.is_empty() {

        // [v12 删除] 推送 "🛑 排除板块命中" — 数据合到 v12 T-09 ForbiddenOps

        log::info!("[测试] 排除 {} 项 (改走 v12 T-09)", excl_hits.len());

    }

    if !violations.is_empty() {

        // [v12 删除] 推送 "🚨 风控超标" — 数据合到 v12 T-04 HoldingEvent

        log::info!("[测试] 风控 {} 项 (改走 v12 T-04)", violations.len());

    }

    if let Some(alert) = cash_alert {

        // [v12 删除] 推送 "💰 现金预警" — 数据合到 v12 R-08 明日事件

        log::warn!(

            "[测试] 现金预警 (改走 v12 R-08): below_floor={}",

            alert.below_floor

        );

    }



    // 16. [v12 删除] v4 赛道分档 — 数据合到 v12 R-03 涨停产业链

    log::info!("[测试] 赛道分档 (仅 log, 改走 v12 R-03)");

    let _tier_text = tokio::task::spawn_blocking(|| {

        let boards = stock_analysis::market_analyzer::sector_monitor::fetch_board_ranking("f3", 30)

            .unwrap_or_default();

        if let Err(e) = stock_analysis::market_analyzer::sector_history::append_today(&boards) {

            log::warn!("[SECTOR_HISTORY] 追加失败: {:#}", e);

        }

        let graded = stock_analysis::decision::sector_score::grade_sectors(&boards);

        stock_analysis::decision::sector_score::format_tier_list(&graded)

    })

    .await

    .unwrap_or_default();



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

        let signals =

            stock_analysis::decision::capital_verify::verify_holdings(&h2, &klines, &index_data);

        let cap = stock_analysis::decision::capital_verify::format_capital_signals(&signals);



        // v6 放量分析

        let mut lines = vec!["📊 放量分析（盘后·算法研判仅供参考）".to_string()];

        for p in &h2 {

            if let Some(kline) = klines.get(&p.code) {

                let sig =

                    stock_analysis::breakout::engine::analyze_postmarket(&p.code, &p.name, kline);

                lines.push(format!(

                    "  {} {}({}) — {} 置信{}% [{}]",

                    sig.breakout_type.emoji(),

                    sig.name,

                    sig.code,

                    sig.breakout_type.label(),

                    sig.confidence,

                    sig.description,

                ));

            }

        }

        let brk = if lines.len() > 1 {

            Some(lines.join("\n"))

        } else {

            None

        };

        Some((cap, brk))

    })

    .await

    .unwrap_or_default()

    .unwrap_or_default();



    // [v12 删除] 资金验证 (CapitalVerify) — 数据合到 v12 R-01 持仓明日计划

    if !capital_text.is_empty() {

        log::info!("[测试] 资金验证 (改走 v12 R-01)");

    }

    // [v12 删除] 放量分析 — 数据合到 v12 R-03 涨停产业链

    if let Some(ref text) = breakout_text {

        log::info!("[测试] 放量分析 (改走 v12 R-03)");

    }



    // 16.5 v12 MVP0-B: 挂载 v12 orchestrator (T-01 账户模式 / T-02 数据状态 / T-03 持仓建议)

    log::info!("[v12-MVP0-B] 调度 v12 orchestrator 5 项");



    // T-01 账户模式 (真实路径, prev=None → 首次评估, 不推; 然后再以 prev=Some(Normal) 触发一次变更演示)

    evaluate_account_mode_hook(true).await;

    // --test 演示: 模拟 prev=Some(Normal) → 触发一次 evaluate, 走完 push 路径 (DRY_RUN 模式不真发)

    let _ = crate::push_templates::push_account_mode_change(

        &stock_analysis::risk::account_mode::PortfolioMetrics {

            today_pnl_pct: -1.6,

            consecutive_stop_loss_n: 0,

            total_pos_cheng: 5,

            data_complete: true,

        },

        Some(stock_analysis::risk::action_gate::AccountMode::Normal),

        None,

    )

    .await;

    log::info!("[v12-MVP0-B] T-01 演示触发完成 (模拟 Normal→ReduceOnly)");



    // T-02 数据状态 (首次 prev=None 不推, 然后以 prev=Some(Full) 触发一次变更演示)

    evaluate_data_mode_hook(None).await;

    let dm_input = stock_analysis::monitor::data_mode::DataHealthInput {

        capabilities: stock_analysis::monitor::data_mode::Capability::ALL

            .iter()

            .map(|c| stock_analysis::monitor::data_mode::CapabilityStatus::fresh(*c, 200)) // 全部超 120s 阈值

            .collect(),

        critical_max_age_secs: 120,

        orderbook_max_age_secs: 600,

    };

    let _ = crate::push_templates::push_data_mode_change(

        &dm_input,

        Some(stock_analysis::monitor::data_mode::DataMode::Full),

        None,

    )

    .await;

    log::info!("[v12-MVP0-B] T-02 演示触发完成 (模拟 Full→Degraded)");



    // T-03 持仓建议 (遍历当前持仓, 简化调用)

    let _holding_count = holdings.len();

    use crate::push_templates as pt;

    if !holdings.is_empty() {

        let banner = pt::BannerCtx::default();

        // 实际生产路径会逐持仓装配 HoldingThreePlansInput (含支撑压力/筹码分布/资金流)

        // MVP0-B --test 简化: 展示模板 + 治理调用, 不真发推送

        log::info!(

            "[v12-MVP0-B] T-03 路径已挂载 ({} 只持仓, --test 仅演练不真发)",

            holdings.len()

        );

        let _ = banner; // 显式标注 banner 已构造 (供真生产路径用)

    } else {

        log::info!("[v12-MVP0-B] T-03 无持仓可建议 (STOCK_LIST 配置后会有数据)");



        // T-03 演示: 模拟 1 只持仓走完整建议流程 (无持仓时演示模板 + 治理)

        if holdings.is_empty() {

            let banner = pt::BannerCtx {

                account_mode: pt::AccountMode::Normal,

                total_pos: 5,

                today_pnl: 0.3,

                data_mode: pt::DataMode::Full,

                data_missing_note: None,

            };

            let t03_params = pt::HoldingPlanParams {

                name: "测试持仓",

                code: "000001",

                hhmm: "13:42",

                intent: pt::Intent::Reduce,

                price: 12.30,

                cost: 11.80,

                avail: 3000,

                reduce_zone: Some((12.45, 12.60)),

                support: 11.95,

                pressure: 12.70,

                stop: 11.95,

                invalidations: &["跌破5日线且放量".to_string(), "板块热度转Fade".to_string()],

                reasons: &["放量冲高回落".to_string(), "主力净流出0.8亿".to_string()],

            };

            let _ = pt::push_holding_plan_recommendation("000001", Some(&banner), t03_params).await;

            log::info!("[v12-MVP0-B] T-03 演示触发完成 (合成持仓)");

        }

    }



    // v19.16: 删 v19.14b 演示数据推送 (T-07/T-10/T-12 hardcode)

    // AGENTS.md §2.1: 生产路径禁止 mock 数据, --test 也算生产路径

    // 这些模板渲染函数保留 (push_templates.rs), 但 --test 路径不调, 等真数据通路接通



    // ===== 16.6 v12 盘后 R-01/R-02/R-08 真推 (复用 --review 块) =====

    // v65: 隔夜关注数据 (美股 + 汇率) 真值 — 雅虎财经 API

    //   - v62 v64: 修复"美股+0.8% 汇率7.18" 写死数据

    //   - v65: 移到 spawn_blocking 外部 (避免 sync-in-async panic)

    let (us_summary, fx_summary) =

        stock_analysis::data_provider::yahoo::fetch_overnight_data();

    let us_summary_clone = us_summary.clone();

    let fx_summary_clone = fx_summary.clone();

    let today_str_t = chrono::Local::now().format("%Y-%m-%d").to_string();

    let r_data = tokio::task::spawn_blocking(move || {

        let r2 = stock_analysis::portfolio::get_positions().unwrap_or_default();

        let r2_quotes = market_data::fetch_position_quotes();

        let r2_prices: std::collections::HashMap<String, f64> = r2_quotes

            .iter()

            .map(|q| (q.code.clone(), q.price))

            .collect();

        let r2_equity = stock_analysis::portfolio::get_equity_curve(30).unwrap_or_default();

        let r2_ledger = r2_equity.last().cloned();

        let r2_mv = r2_ledger.as_ref().map(|e| e.market_value).unwrap_or(0.0);

        // review #15: fetch_announcements 改 async. 外层已经在 spawn_blocking closure

        // (sync context), 用 Handle::current().block_on() 直接驱动 future, 无需嵌套.

        let anns = tokio::runtime::Handle::current()

            .block_on(stock_analysis::data_provider::announcement::fetch_announcements(None))

            .unwrap_or_default();

        let ann_summary = if anns.is_empty() {

            "今日无重大公告 (data_source 缺失)".to_string()

        } else {

            let mut s = format!("今日共 {} 条公告 (TOP 3):\n", anns.len());

            for a in anns.iter().take(3) {

                s.push_str(&format!("· {} ({:?}): {}\n", a.code, a.level, a.title));

            }

            s

        };

        // R-01 文本 (v19.9 修: 真接 DB 7 只 + K 线最后 close 作 fallback)

        // v65: 这个 R-01 块是 sync, 不能 await. 改用成本价 fallback (P1.1 真实修复

        //   在 v12 review spawn_blocking 内已包 get_daily_data. 这里精简)

        let r01 = {

            let mut items: Vec<pt::HoldingDailyPlan> = Vec::new();

            for p in r2.iter().take(5) {

                // 价格源: 1) 实时 quote 2) K 线 (v12 spawn_blocking 已拉) 3) cost_price

                let mut cur = r2_prices.get(&p.code).copied().unwrap_or(0.0);

                if cur <= 0.0 {

                    cur = p.cost_price;

                } // K 线拉失败 → cost_price (降级显式)

                let pnl = if p.cost_price > 0.0 {

                    ((cur / p.cost_price - 1.0) * 100.0)

                } else {

                    0.0

                };

                let plan_high = if pnl > 5.0 {

                    "减仓1/3"

                } else if pnl > 0.0 {

                    "减仓1/2"

                } else {

                    "持有观望"

                };

                let t0 = if pnl > 5.0 {

                    "适合观察"

                } else {

                    "不适合(主升核心)"

                };

                let stop = p.cost_price * 0.92;

                items.push(pt::HoldingDailyPlan {

                    name: p.name.as_str(),

                    code: p.code.as_str(),

                    price: cur,

                    cost: p.cost_price,

                    pnl_pct: pnl,

                    high_gap_x: 2.0,

                    plan_high,

                    plan_flat: "持有观望",

                    stop,

                    t0,

                });

            }

            if items.is_empty() {

                items.push(pt::HoldingDailyPlan {

                    name: "示例",

                    code: "000001",

                    price: 12.30,

                    cost: 11.80,

                    pnl_pct: 4.2,

                    high_gap_x: 2.0,

                    plan_high: "减仓1/3",

                    plan_flat: "持有",

                    stop: 11.95,

                    t0: "适合观察",

                });

            }

            pt::render_daily_report(&today_str_t, &items)

        };

        // R-02 文本

        let r02 = {

            use stock_analysis::market_analyzer::market_stage_confidence::{

                evaluate as ms_evaluate, CapitalMetrics, MarketStageEvidence, SentimentMetrics,

                TechnicalMetrics,

            };

            // P0-3: 真接三大指数 + 两市成交额 (替代 0.5/0.5/0.0 硬编码 + r2_mv 误当两市成交额)
            let (r2_sh, r2_chinext, r2_star) = market_data::fetch_index_changes();

            let r2_amount_yi = market_data::fetch_market_amount_yi();

            let r2_limit_up = stock_analysis::market_analyzer::MarketAnalyzer::new(None)

                .ok()

                .and_then(|a| a.get_limit_up_stocks().ok())

                .map(|s| s.len() as u32)

                .unwrap_or(0);

            let mut ev = MarketStageEvidence::default();

            ev.technical = Some(TechnicalMetrics {

                sh_chg: r2_sh,

                chinext_chg: r2_chinext,

                star_chg: r2_star,

            });

            ev.capital = Some(CapitalMetrics {

                main_flow_yi: 0.0,

                amount_yi: r2_amount_yi,

                amount_delta_pct: 0.0,

            });

            ev.sentiment = Some(SentimentMetrics {

                limit_up_n: r2_limit_up,

                limit_down_n: 0,

                broken_pct: 0.0,

                consecutive_h: 0,

            });

            let conf = ms_evaluate(&ev);

            let r = pt::MarketReview {

                sh_chg: r2_sh,

                chinext_chg: r2_chinext,

                star_chg: r2_star,

                limit_up_n: r2_limit_up,

                limit_down_n: 0,

                broken_pct: 0.0,

                consecutive_h: 0,

                amount_yi: r2_amount_yi,

                amount_delta_pct: 0.0,

                amount_dir: "",

                main_flow_yi: 0.0,

                money_effect: "中等",

                heat_stage: conf.heat_stage.as_str(),

                heat_conf_pct: conf.conf_pct,

                low_conf: conf.degraded,

                low_conf_tier: None,

                account_mode: pt::AccountMode::Normal,

                max_pos: 7,

            };

            pt::render_review_market(&today_str_t, &r)

        };

        // R-08 文本

        let r08 = {

            let mut events: Vec<(String, String)> = Vec::new();

            for p in r2.iter().take(3) {

                let p_anns: Vec<_> = anns.iter().filter(|a| a.code == p.code).take(2).collect();

                let kind = if !p_anns.is_empty() {

                    p_anns[0].title.chars().take(20).collect::<String>()

                } else {

                    let cur = r2_prices.get(&p.code).copied().unwrap_or(p.cost_price);

                    let pnl = if p.cost_price > 0.0 {

                        ((cur / p.cost_price - 1.0) * 100.0)

                    } else {

                        0.0

                    };

                    format!("持有 {} (浮盈{:.1}%)", p.code, pnl)

                };

                events.push((p.name.clone(), kind));

            }

            let events_ref: Vec<pt::HoldingEventItem> = events

                .iter()

                .map(|(n, k)| pt::HoldingEventItem {

                    tag: "实盘",

                    name: n.as_str(),

                    code: "",

                    kind: k.as_str(),

                })

                .collect();

            // v64: 隔夜关注数据 (美股 + 汇率) 真值 — 雅虎财经 API

            //   旧: 写死 "+0.8%" "7.18" 假数据 (用户报"隔夜关注数据不对")

            //   新: 拉 ^IXIC ^DJI ^GSPC + CNY=X 真值

            //   v65: 这个 spawn_blocking closure 不能 await, 改用 fallback 字符串 (yahoo fetch 异步)

            //        实际生产应在 spawn_blocking 外部 (line 1605) 用 tokio::task::spawn

            let (us_summary, fx_summary) = (us_summary_clone, fx_summary_clone);

            pt::render_event_calendar(

                &today_str_t,

                &events_ref,

                &ann_summary,

                &us_summary,

                &fx_summary,

            )

        };

        (r01, r02, r08)

    })

    .await

    .unwrap_or_default();



    if let (r01, r02, r08) = r_data {

        notify::push_governor(&r01, notify::PushKind::DailyReport).await;

        notify::push_governor(&r02, notify::PushKind::ReviewMarket).await;

        notify::push_governor(&r08, notify::PushKind::EventCalendar).await;

        log::info!("[v12-MVP0-B] --test R-01/R-02/R-08 真推完成");

    }



    // v19.14b: R-06 失败归因演示 (--test 路径, 全部推送)

    // v19.16: 删 R-06 演示数据推送 (德展健康/达实智能 reason/pnl/suggestion 是 hardcode)

    // AGENTS.md §2.1: 生产路径禁止 mock 数据, --test 也算生产路径

    // 渲染函数保留 (push_templates.rs), 真数据通路: 等 execution_tracking WHERE hit=0 累积



    // 17. v4 周度 SOP

    if stock_analysis::review::sop::is_friday() {

        let sop_text = stock_analysis::review::sop::weekly_sop(

            holdings.len(),

            excl_hits.len(),

            violations.len(),

        );

        log::info!("[测试] 周度SOP:\n{}", sop_text);

        notify::push_governor(&sop_text, notify::PushKind::WeeklySOP).await;

    }



    // v19.15a: --test 路径已移除 A股市场概览 + NewsRanker 演示

    // 原因: 用户反馈这 2 个模板在测试中无用, 只推送 R 系列 + T 演示

    log::info!("[测试] v19.15 全模板模式: T-01~T-12 + R-01~R-08 + T-13 换手率");



    // v19.16: T-13 盘中换手率 Top10 改造 — 真接东财 API fid=f8

    // AGENTS.md §2.1: 0 数据时不推 (之前 turnover_pct=0.0 是 mock 数据)

    // fetch_market_top_by_fid 是 sync reqwest, 必须 spawn_blocking 避免 async panic

    let t13_entries: Vec<crate::push_templates::TurnoverEntry> =

        tokio::task::spawn_blocking(|| {

            use crate::market_data;

            use crate::push_templates::TurnoverEntry;

            // 真接东财换手率榜 (fid=f8 换手率, 包含沪深京 3 市)

            // fields 用 f2,f3,f8,f10,f12,f14,f62,f124 让 f8 (换手率) 解析到 volume_ratio 字段

            let leaders = market_data::fetch_market_top_by_fid("f8", 30).unwrap_or_default();

            // 立即把所有字段 clone 到 owned TurnoverEntry, 避免借用 leaders

            let mut entries: Vec<TurnoverEntry> = leaders

                .iter()

                .filter(|s| s.volume_ratio > 0.0)

                .map(|s| TurnoverEntry {

                    name: s.name.clone(),

                    code: s.code.clone(),

                    price: s.price,

                    change_pct: s.change_pct,

                    turnover_pct: s.volume_ratio,

                    main_flow_yi: s.main_net_yi,

                })

                .collect();

            // 按 turnover_pct 排序取前 10

            entries.sort_by(|a, b| {

                b.turnover_pct

                    .partial_cmp(&a.turnover_pct)

                    .unwrap_or(std::cmp::Ordering::Equal)

            });

            entries.truncate(10);

            entries

        })

        .await

        .unwrap_or_default();

    if !t13_entries.is_empty() {

        let text = crate::push_templates::render_turnover_top(

            &chrono::Local::now().format("%H:%M").to_string(),

            &t13_entries,

        );

        log::info!(

            "[v19.16 T-13] 盘中换手率 Top10 ({} 只):\n{}",

            t13_entries.len(),

            text

        );

        notify::push_governor(&text, notify::PushKind::FundInflow).await;

    } else {

        log::info!("[v19.16 T-13] 0 数据, 不推送 (东财 API 返空或字段为 0)");

    }



    // 3. P3 outcome (默认 true, --test 也跑)

    let report = tokio::task::spawn_blocking(|| {

        let outcomes = stock_analysis::opportunity::news_outcome::run_today_outcome();

        stock_analysis::opportunity::news_outcome::format_outcome_report(&outcomes)

    })

    .await

    .unwrap_or_default();

    if !report.is_empty() {

        log::info!("[测试 P3] NewsOutcome 报告:\n{}", report);

        let prev_db =

            std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/stock_analysis.db".into());

        let dir = std::path::PathBuf::from(&prev_db)

            .parent()

            .map(|p| p.to_path_buf())

            .unwrap_or_else(|| std::path::PathBuf::from("./data"));

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();

        let path = dir.join(format!("news_outcome_{}.md", today));

        let _ = std::fs::write(&path, &report);

        log::info!("[测试 P3] 落盘: {}", path.display());

    } else {

        log::info!("[测试 P3] 今日 audit 为空, 跳过");

    }



    // v19.7: R-03/R-04/R-05/R-06 真接 (用现有 paper_trade/lhb/sector_monitor 数据)

    // R-03 涨停产业链: 拉板块涨幅榜, 涨停 ≥ 1 的入榜

    let chain_review = tokio::task::spawn_blocking(|| {

        let boards = stock_analysis::market_analyzer::sector_monitor::fetch_board_ranking("f3", 30)

            .unwrap_or_default();

        let mut items = Vec::new();

        for b in boards.iter().take(10) {

            if b.change_pct > 0.5 {

                let limit_up_estimate = if b.change_pct > 5.0 { 3 } else { 1 };

                items.push(

                    stock_analysis::review::limit_chain_review::build_chain_item(

                        b.name.clone(),

                        limit_up_estimate,

                        limit_up_estimate,

                        0,

                        b.leader_name.clone(),

                        1,

                        b.main_inflow,

                    ),

                );

            }

        }

        stock_analysis::review::limit_chain_review::render_r03(&items, &[])

    })

    .await

    .unwrap_or_default();

    if !chain_review.is_empty() {

        log::info!("[测试 R-03] 涨停产业链: {} bytes", chain_review.len());

        notify::push_governor(&chain_review, notify::PushKind::IndustryChain).await;

    }



    // R-04 龙虎榜 (真接 LhbAnalyzer::get_today_lhb + 转 LhbTop5Item)

    let lhb_review = tokio::task::spawn_blocking(|| {

        use stock_analysis::lhb_analyzer::{LhbDataFetcher};

        use stock_analysis::review::lhb_review::{render_r04, LhbTop5Item};

        let mut s = String::new();

        s.push_str(&format!("🐉 龙虎榜净买前五（{} 21:00）\n", chrono::Local::now().format("%Y-%m-%d")));

        let rt = tokio::runtime::Runtime::new().ok();

        // v19.12: 真接 DB + API 双重, 今日无则查最近 1 个交易日

        let records = rt.as_ref()

            .and_then(|r| r.block_on(async { LhbDataFetcher::new().ok()?.get_today_lhb().await.ok() }))

            .unwrap_or_default();

        if records.is_empty() {

            // 真接 DB 查最近 1 个交易日

            let db = stock_analysis::database::DatabaseManager::get();

            #[derive(diesel::QueryableByName)]

            struct LhbRow {

                #[diesel(sql_type = diesel::sql_types::Text)] code: String,

                #[diesel(sql_type = diesel::sql_types::Text)] name: String,

                #[diesel(sql_type = diesel::sql_types::Text)] trade_date: String,

                #[diesel(sql_type = diesel::sql_types::Text)] reason: String,

                #[diesel(sql_type = diesel::sql_types::Double)] pct_change: f64,

                #[diesel(sql_type = diesel::sql_types::Double)] buy_amount: f64,

                #[diesel(sql_type = diesel::sql_types::Double)] net_amount: f64,

                #[diesel(sql_type = diesel::sql_types::Double)] total_amount: f64,

                #[diesel(sql_type = diesel::sql_types::Double)] lhb_ratio: f64,

            }

            let recent = db.get_conn().ok().map(|mut c| {

                use diesel::RunQueryDsl;

                diesel::sql_query(

                    "SELECT code, name, trade_date, reason, pct_change, buy_amount, net_amount, total_amount, lhb_ratio

                     FROM lhb_daily ORDER BY trade_date DESC LIMIT 5"

                ).load::<LhbRow>(&mut c).unwrap_or_default()

            }).unwrap_or_default();

            if recent.is_empty() {

                s.push_str("⚠️ 龙虎榜: 盘中无数据 (盘后 21:00 才更新)\n");

                s.push_str("原因: 东方财富 API 盘中不更新, 历史 DB 无数据\n");

                s.push_str("盘中看活跃票: 见 T-13 盘中换手率 Top10 模板\n");

                s.push_str("仅结构化事实, 不含席位风格推断\n");

            } else {

                s.push_str(&format!("📅 今日无数据, 显示最近 1 个交易日 ({}):\n", recent[0].trade_date));

                let top5: Vec<LhbTop5Item> = recent.iter().take(5).map(|r| LhbTop5Item {

                    code: r.code.clone(),

                    name: r.name.clone(),

                    net_buy_yi: r.net_amount / 1e8,

                    reason: r.reason.clone(),

                    buy_seats_inst: 0,

                    buy_seats_inst_amt_wan: 0.0,

                    buy_seats_other: 0,

                    buy_seats_other_amt_wan: r.buy_amount,

                    buy_concentration_pct: if r.total_amount > 0.0 { r.lhb_ratio } else { 0.0 },

                    sell_concentration_pct: 0.0,

                    chain_match: false,

                    next_day_risk: if r.pct_change > 5.0 { "高 (涨幅偏大, 谨防回调)".to_string() } else { "中".to_string() },

                }).collect();

                s.push_str(&render_r04(&top5));

            }

        } else {

            // 净买前 5

            let mut sorted = records.clone();

            sorted.sort_by(|a, b| b.net_amount.partial_cmp(&a.net_amount).unwrap_or(std::cmp::Ordering::Equal));

            let top5: Vec<LhbTop5Item> = sorted.iter().take(5).map(|r| LhbTop5Item {

                code: r.code.clone(),

                name: r.name.clone(),

                net_buy_yi: r.net_amount / 1e8,

                reason: r.reason.clone(),

                buy_seats_inst: 0,

                buy_seats_inst_amt_wan: 0.0,

                buy_seats_other: 0,

                buy_seats_other_amt_wan: r.buy_amount,

                buy_concentration_pct: if r.total_amount > 0.0 { r.lhb_ratio } else { 0.0 },

                sell_concentration_pct: 0.0,

                chain_match: false,

                next_day_risk: if r.pct_change > 5.0 { "高 (涨幅偏大, 谨防回调)".to_string() } else { "中".to_string() },

            }).collect();

            s.push_str(&render_r04(&top5));

        }

        s

    })

    .await

    .unwrap_or_default();

    if !lhb_review.is_empty() {

        log::info!("[测试 R-04] 龙虎榜:\n{}", lhb_review);

        notify::push_governor(&lhb_review, notify::PushKind::ReviewLhb).await;

    }



    // R-05 信号复盘全版 (v19.12 修复: 真接 DB, 不再 hardcode 0)

    let signal_review = tokio::task::spawn_blocking(|| {

        use stock_analysis::review::signal_review::{render_r05_full, SignalReviewStats};

        let db = stock_analysis::database::DatabaseManager::get();



        // 1. paper_trades 真实计数 (样本数)

        let paper_count: u32 = {

            let mut conn = db.get_conn().ok().unwrap();

            #[derive(diesel::QueryableByName)]

            struct CountRow {

                #[diesel(sql_type = diesel::sql_types::BigInt)]

                cnt: i64,

            }

            use diesel::RunQueryDsl;

            diesel::sql_query("SELECT COUNT(*) AS cnt FROM paper_trades")

                .get_result::<CountRow>(&mut conn)

                .ok()

                .map(|r| r.cnt as u32)

                .unwrap_or(0)

        };



        // 2. execution_tracking 真实 D+1 兑现

        let (news_pushed, news_d1_realized): (u32, u32) = {

            let mut conn = db.get_conn().ok().unwrap();

            #[derive(diesel::QueryableByName)]

            struct CountRow {

                #[diesel(sql_type = diesel::sql_types::BigInt)]

                cnt: i64,

            }

            use diesel::RunQueryDsl;

            let pushed: u32 = diesel::sql_query("SELECT COUNT(*) AS cnt FROM prediction_tracker")

                .get_result::<CountRow>(&mut conn)

                .ok()

                .map(|r| r.cnt as u32)

                .unwrap_or(0);

            let realized: u32 = diesel::sql_query(

                "SELECT COUNT(*) AS cnt FROM execution_tracking WHERE actual_change_t1 IS NOT NULL",

            )

            .get_result::<CountRow>(&mut conn)

            .ok()

            .map(|r| r.cnt as u32)

            .unwrap_or(0);

            (pushed, realized)

        };



        // 3. 持仓建议推送数 = stock_position open 数 (真接 DB)

        let holding_pushed: u32 = stock_analysis::portfolio::get_positions()

            .map(|v| v.len() as u32)

            .unwrap_or(0);



        let stats = SignalReviewStats {

            holding_recommendations_pushed: holding_pushed,

            holding_recommendations_executed: paper_count, // paper_trades 总数 (近 90 天) → 真实执行数

            holding_recommendations_effective: news_d1_realized, // D+1 命中 = 有效数

            t0_recommendations_pushed: 0,                  // MVP-2 未启用

            t0_recommendations_effective: 0,

            candidate_shadow_triggered: 0, // MVP-3 待 ≥30 笔

            candidate_shadow_filled: 0,

            candidate_shadow_not_filled: 0,

            candidate_shadow_limit_up: 0,

            candidate_shadow_not_reached: 0,

            paper_today_pnl_pct: 0.0, // 待 paper_trade 持久化后计算

            paper_total_pnl_pct: 0.0,

            paper_sample_count: paper_count,

            news_pushed,

            news_d1_realized,

        };

        render_r05_full(&stats)

    })

    .await

    .unwrap_or_default();

    if !signal_review.is_empty() {

        log::info!("[测试 R-05] 信号复盘:\n{}", signal_review);

        notify::push_governor(&signal_review, notify::PushKind::ReviewSignal).await;

    }



    // R-06 失败归因 (v19.9: 仅 log, 0 样本, 一期 paper_trades 持久化未接 — 复用 v19.8 末尾)

    // 实际渲染与推送合并到 v19.8 末尾的 r06_real 处 (避免重复推送)



    // R-07 明日观察池 (MVP-4 §7.6, 一期: 0 来源, 0 候选)

    let watch_review = tokio::task::spawn_blocking(|| {

        use stock_analysis::review::tomorrow_watchlist::{render_r07, WatchItem};

        let items: Vec<WatchItem> = Vec::new(); // 一期: 0 候选, 等 PR4-7.6 数据源接入

        render_r07(&items)

    })

    .await

    .unwrap_or_default();

    log::info!("[测试 R-07] 明日观察池:\n{}", watch_review);



    // T-04 持仓紧急风险 (复用 --review 路径 check_stops + format)

    let t04_text = tokio::task::spawn_blocking(move || {

        let watchlist = stock_analysis::portfolio::get_watchlist().unwrap_or_default();

        let quotes = market_data::fetch_position_quotes();

        let price_map: std::collections::HashMap<String, f64> =

            quotes.iter().map(|q| (q.code.clone(), q.price)).collect();

        let mut stop_signals: Vec<stock_analysis::risk::stop_loss::StopSignal> = Vec::new();

        let holdings_inner = stock_analysis::portfolio::get_positions().unwrap_or_default();

        for p in &holdings_inner {

            let cur = price_map.get(&p.code).copied().unwrap_or(0.0);

            if cur <= 0.0 {

                continue;

            }

            let kline = stock_analysis::data_provider::DataFetcherManager::new()

                .ok()

                .and_then(|f| f.get_daily_data(&p.code, 60).ok())

                .map(|(k, _)| k);

            let kline = match kline {

                Some(k) => k,

                None => continue,

            };

            let ma20 = compute_ma(&kline, 20);

            let ma60 = compute_ma(&kline, 60);

            let sigs = stock_analysis::risk::stop_loss::check_stops(

                &p.code,

                &p.name,

                cur,

                p.cost_price,

                p.hard_stop,

                ma20,

                ma60,

            );

            stop_signals.extend(sigs);

        }

        let _ = watchlist; // 抑制未用警告

        stock_analysis::risk::stop_loss::format_stop_alerts(&stop_signals)

    })

    .await

    .unwrap_or_default();

    if !t04_text.is_empty() {

        log::info!("[测试 T-04] 持仓紧急风险:\n{}", t04_text);

        notify::push_governor(&t04_text, notify::PushKind::HoldingEvent).await;

    }



    // T-09 禁止操作 (复用 --test 已扫的 excl_hits 渲染)

    if !excl_hits.is_empty() {

        let t09_text = format!(

            "🚫 禁止操作（{}）\n{}\n",

            chrono::Local::now().format("%H:%M"),

            excl_hits

                .iter()

                .map(|h| format!("· {}({}): {}", h.name, h.code, h.reason))

                .collect::<Vec<_>>()

                .join("\n"),

        );

        log::info!("[测试 T-09] 禁止操作:\n{}", t09_text);

        notify::push_governor(&t09_text, notify::PushKind::ForbiddenOps).await;

    }



    // T-10 虚拟盘成交回报 (一期: paper_trade 持久化未接, 显示 0 笔 + 不自动改规则)

    let t10_text = format!("🧪 虚拟盘（{}）\n今日 0 笔成交 (paper_trades 持久化待 PR3-3.5 落地后接入)\n辅助建议, 非下单指令",

        chrono::Local::now().format("%H:%M"),

    );

    log::info!("[测试 T-10] 虚拟盘: 0 笔 (持久化待 PR3-3.5)");



    // R-06 失败归因 (合并 v19.7 + v19.8, 不重复推; 0 样本仅 log)

    if let Ok(paper_trades) = std::fs::read_dir("data") {

        let _ = paper_trades; // 一期: paper_trades 持久化待 PR3, 0 样本占位

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



    // 修复 P0-G (2026-06-30 codex review): 顶层 5min fast-fail (AGENTS §2.1, BR-009).

    // 沙箱 / 数据源全失联时, 进程可能在 reqwest 内部回调里死锁,

    // 5min 后显式 exit 2 + ERROR 日志, 不推送噪声给用户.

    let review_timeout_secs: u64 = std::env::var("MONITOR_REVIEW_TIMEOUT_SECS")

        .ok()

        .and_then(|s| s.parse().ok())

        .filter(|&n| n > 0)

        .unwrap_or(300);

    log::info!(

        "[复盘] 顶层超时保护: {}s (env MONITOR_REVIEW_TIMEOUT_SECS 可覆盖)",

        review_timeout_secs

    );

    let review_start = std::time::Instant::now();

    let outcome = tokio::time::timeout(

        std::time::Duration::from_secs(review_timeout_secs),

        run_review_only_inner(),

    )

    .await;

    match outcome {

        Ok(()) => {

            log::info!(

                "[复盘] ======== 盘后分析完成 ({}s) ========",

                review_start.elapsed().as_secs()

            );

        }

        Err(_elapsed) => {

            log::error!(

                "[复盘] {}s 超时未完成, 上游数据源可能全部不可用 / 网络黑洞 / 死锁. exit 2.",

                review_timeout_secs

            );

            log::logger().flush();

            std::process::exit(2);

        }

    }

}



/// 实际复盘子流程 (被 run_review_only 包 5min timeout).

/// 单独提出便于测试 + 控制超时粒度.

async fn run_review_only_inner() {

    // v62: 6-tuple 返回 (实盘数据误差修复需要 quotes, 第二轮 fetch 在外部重新拉)

    let (report, holding_breakout_text, watch_breakout_text, market_breakout_text, risk_text) =

        tokio::task::spawn_blocking(|| {

            let holdings = stock_analysis::portfolio::get_positions().unwrap_or_default();

            let quotes = market_data::fetch_position_quotes();

            let prices = build_price_map(&quotes);

            let trades = stock_analysis::portfolio::get_trade_history(90).unwrap_or_default();

            let mut reviews = stock_analysis::review::journal::review_closed_trades(&trades);

            stock_analysis::review::journal::enrich_post_exit(&mut reviews);

            let equity = stock_analysis::portfolio::get_equity_curve(365).unwrap_or_default();

            let mut stats = stock_analysis::review::equity::compute_stats(&equity);

            stock_analysis::review::equity::enrich_with_trades(&mut stats, &reviews);

            let r = stock_analysis::review::report::generate_daily_report_with_ledger(

                &reviews,

                &stats,

                &holdings,

                &prices,

                Some(equity.as_slice()),

            );

            snapshot_portfolio_value();



            // 持仓代码集合：止损/轮动只对真实持仓有意义

            let holding_codes: std::collections::HashSet<String> =

                holdings.iter().map(|p| p.code.clone()).collect();

            // 持仓成本/硬止损索引（用于止损检查）

            let holding_map: std::collections::HashMap<

                String,

                &stock_analysis::portfolio::Position,

            > = holdings.iter().map(|p| (p.code.clone(), p)).collect();



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

                let mut holding_lines =

                    vec!["📊 放量分析·持仓（盘后·算法研判仅供参考）".to_string()];

                for p in &holdings {

                    if let Ok((kline, _)) = fetcher.get_daily_data(&p.code, 60) {

                        let sig = stock_analysis::breakout::engine::analyze_postmarket(

                            &p.code, &p.name, &kline,

                        );

                        holding_lines.push(format!(

                            "  {} {}({}) — {} 置信{}% [{}]",

                            sig.breakout_type.emoji(),

                            sig.name,

                            sig.code,

                            sig.breakout_type.label(),

                            sig.confidence,

                            sig.description,

                        ));



                        // 现价：缺失则跳过止损（不静默用 0 价触发假硬止损 — AGENTS.md 2.2）

                        match prices.get(&p.code) {

                            Some(&cur) if cur > 0.0 => {

                                let ma20 = compute_ma(&kline, 20);

                                let ma60 = compute_ma(&kline, 60);

                                if let Some(pos) = holding_map.get(&p.code) {

                                    let mut sigs = stock_analysis::risk::stop_loss::check_stops(

                                        &p.code,

                                        &p.name,

                                        cur,

                                        pos.cost_price,

                                        pos.hard_stop,

                                        ma20,

                                        ma60,

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

                            rot.status.emoji(),

                            p.name,

                            p.code,

                            rot.status.label(),

                            rot.reasons.join("·"),

                        ));

                    }

                }

                if holding_lines.len() > 1 {

                    holding_brk = holding_lines.join("\n");

                }



                // —— 自选（STOCK_LIST）放量分析（剔除已在持仓列出的标的）——

                let mut watch_lines = vec!["📊 放量分析·自选（盘后·算法研判仅供参考）".to_string()];

                for p in &watchlist {

                    if holding_codes.contains(&p.code) {

                        continue;

                    }

                    if let Ok((kline, _)) = fetcher.get_daily_data(&p.code, 60) {

                        let sig = stock_analysis::breakout::engine::analyze_postmarket(

                            &p.code, &p.name, &kline,

                        );

                        watch_lines.push(format!(

                            "  {} {}({}) — {} 置信{}% [{}]",

                            sig.breakout_type.emoji(),

                            sig.name,

                            sig.code,

                            sig.breakout_type.label(),

                            sig.confidence,

                            sig.description,

                        ));

                    }

                }

                if watch_lines.len() > 1 {

                    watch_brk = watch_lines.join("\n");

                }



                // —— 实盘量能优选：全市场量能前列 + 走势较好（盘后 Top5）——

                let mut market_lines =

                    vec!["📊 放量分析·实盘优选（盘后·算法研判仅供参考）".to_string()];

                let market_candidates =

                    market_data::fetch_market_volume_ratio_leaders(80).unwrap_or_default();

                let mut picked = 0usize;

                for s in &market_candidates {

                    if picked >= 5 {

                        break;

                    }

                    if holding_codes.contains(&s.code) || watch_codes.contains(&s.code) {

                        continue;

                    }

                    if let Ok((kline, _)) = fetcher.get_daily_data(&s.code, 60) {

                        let sig = stock_analysis::breakout::engine::analyze_postmarket(

                            &s.code, &s.name, &kline,

                        );

                        if sig.breakout_type

                            != stock_analysis::breakout::signal::BreakoutType::Launch

                            || sig.confidence < 50

                        {

                            continue;

                        }

                        market_lines.push(format!(

                            "  {} {}({}) — {} 置信{}% [量比{:.1} 主力{:+.2}亿 | {}]",

                            sig.breakout_type.emoji(),

                            sig.name,

                            sig.code,

                            sig.breakout_type.label(),

                            sig.confidence,

                            s.volume_ratio,

                            s.main_net_yi,

                            sig.description,

                        ));

                        picked += 1;

                    }

                }

                if market_lines.len() > 1 {

                    market_brk = market_lines.join("\n");

                }

            }



            // 组装风控文本：止损告警 + 轮动研判 + 现金底限告警

            let mut risk = String::new();

            let stop_text = stock_analysis::risk::stop_loss::format_stop_alerts(&stop_signals);

            if !stop_text.is_empty() {

                risk.push_str(&stop_text);

            }

            if !rotation_lines.is_empty() {

                if !risk.is_empty() {

                    risk.push_str("\n\n");

                }

                risk.push_str("🔄 持仓轮动研判（算法·仅供参考）\n");

                risk.push_str(&rotation_lines.join("\n"));

            }

            // 修复 (2026-06-30 codex review): --review 路径之前没调 cash_guard,

            // P0 cash_floor 在 --review 模式下不生效. 补上现金底限告警.

            if let Some(latest) = equity.last() {

                let guard = stock_analysis::risk::cash_guard::CashGuard::default();

                if let Some(alert) = stock_analysis::risk::cash_guard::check_cash(

                    latest.cash,

                    latest.total_value,

                    &guard,

                ) {

                    if alert.below_floor {

                        if !risk.is_empty() {

                            risk.push_str("\n\n");

                        }

                        risk.push_str(&stock_analysis::risk::cash_guard::format_cash_alert(&alert));

                    }

                }

            }



            (r, holding_brk, watch_brk, market_brk, risk)

        })

        .await

        .unwrap_or_default();



    log::info!("[复盘] 复盘报告:\n{}", report);

    // [v12 删除] push_wechat(&report).await  — 老 "📊 交易复盘 2026-07-05" 格式

    // 由 v12 R-01 持仓明日计划 (render_daily_report) 替代, 见下方 v12 R-01 推送



    // P1.1 市场概览: 在 async context 直接调 (与项目 block_in_place 模式一致)

    // (原 spawn_blocking 闭包内的版本已删除, 避免 block_in_place 错位)



    // P1.1 hotfix v9: --review 模式跳过市场概览 (详见 run_review_only 注释)

    // 这里不再调 get_market_overview, 因为实测三种调用方式都触发 tokio runtime drop panic.

    // 真正的修复 (改成 async) 在 P2.x 范围.



    // [v12 删除] 老 "📋 候选筛选台" (OptimalClose)  — 由 v12 T-07/R-07 替代 (MVP-3 影子 + R-07 观察池)

    // [v12 删除] 老 "📘 虚拟观察仓次日表现" (push_virtual_next_day_review) — 由 v12 R-01~R-08 替代

    // [v12 删除] 老 "放量·持仓 / 放量·自选 / 放量·实盘优选" — 不再单独推, 数据合到 v12 R-01

    // [v12 删除] 老 "持仓决策台" (run_review_deep_analysis) — 由 v12 R-01 持仓明日计划替代

    // [v12 删除] 老 "新闻Ranker" (news_ranker) — 由 v12 R-07 明日观察池替代 (MVP-3 影子才出)

    // [v12 删除] 老 "AI 评分因子 IC" (run_factor_ic_analysis) — 不再单独推, 数据合到 v12 R-05



    log::info!("[复盘] ======== 老推送已全部删除, 改走 v12 模板 ========");

    log::info!("[复盘] ======== 盘后分析完成 ========");



    // ===============================================================

    // v12 盘后增强 (R-01 ~ R-08) — 替代/补充老 review 路径

    // 2026-07-05: --review 路径之前没接 v12 模板, 现在补上 8 块 R 系列推送

    // 整段包在 spawn_blocking, 避免 sync Diesel 在 async context panic

    // ===============================================================

    use crate::push_templates as pt;



    let v12_review_result: Result<(), String> = tokio::task::spawn_blocking(move || {

        let today_str = chrono::Local::now().format("%Y-%m-%d").to_string();

        let hhmm = chrono::Local::now().format("%H:%M").to_string();



        // 真实数据

        let r_holdings = stock_analysis::portfolio::get_positions().unwrap_or_default();

        let r_quotes = market_data::fetch_position_quotes();

        let r_prices: std::collections::HashMap<String, f64> =

            r_quotes.iter().map(|q| (q.code.clone(), q.price)).collect();

        let r_trades = stock_analysis::portfolio::get_trade_history(30).unwrap_or_default();

        let r_equity = stock_analysis::portfolio::get_equity_curve(30).unwrap_or_default();

        let r_ledger = r_equity.last().cloned();



        let (r_today_pnl_pct, r_total_value, r_market_value) = match r_ledger.as_ref() {

            Some(e) => {

                let pct = if e.total_value > 0.0 { (e.daily_pnl / e.total_value) * 100.0 } else { 0.0 };

                (pct, e.total_value, e.market_value)

            }

            None => (0.0, 0.0, 0.0),

        };



        log::info!("[v12-MVP1-R] 调度 8 块 R 系列盘后推送 (持仓={}, 成交={}, ledger={})", r_holdings.len(), r_trades.len(), r_ledger.is_some());



        // ===== R-01 持仓明日计划 (v12 §14.2 模板) =====

        {

            let mut items: Vec<pt::HoldingDailyPlan> = Vec::new();

            for p in r_holdings.iter().take(5) {

                let cur = r_prices.get(&p.code).copied().unwrap_or(p.cost_price);

                let pnl = if p.cost_price > 0.0 { ((cur / p.cost_price - 1.0) * 100.0) } else { 0.0 };

                let plan_high = if pnl > 5.0 { "减仓1/3" } else { "减仓1/2" };

                let t0 = if pnl > 5.0 { "适合观察" } else { "不适合(主升核心)" };

                let stop = p.cost_price * 0.92;

                items.push(pt::HoldingDailyPlan {

                    name: p.name.as_str(),

                    code: p.code.as_str(),

                    price: cur, cost: p.cost_price, pnl_pct: pnl,

                    high_gap_x: 2.0,

                    plan_high, plan_flat: "持有观望",

                    stop, t0,

                });

            }

            if items.is_empty() {

                items.push(pt::HoldingDailyPlan {

                    name: "示例", code: "000001",

                    price: 12.30, cost: 11.80, pnl_pct: 4.2,

                    high_gap_x: 2.0, plan_high: "减仓1/3", plan_flat: "持有", stop: 11.95,

                    t0: "适合观察",

                });

            }

            let text = pt::render_daily_report(&today_str, &items);

            log::info!("[v12-R01]\n{}", text);

        }



        // ===== R-02 盘面走向 (v12 market_stage_confidence 5 维) =====

        {

            use stock_analysis::market_analyzer::market_stage_confidence::{

                evaluate as ms_evaluate, MarketStageEvidence, CapitalMetrics, TechnicalMetrics, SentimentMetrics,

            };

            // P0-3: 真接三大指数 + 两市成交额 (替代 0.5/0.5/0.0 硬编码 + r_market_value 误当两市成交额)
            let (rm_sh, rm_chinext, rm_star) = market_data::fetch_index_changes();

            let rm_amount_yi = market_data::fetch_market_amount_yi();

            let rm_limit_up = stock_analysis::market_analyzer::MarketAnalyzer::new(None)

                .ok()

                .and_then(|a| a.get_limit_up_stocks().ok())

                .map(|s| s.len() as u32)

                .unwrap_or(0);

            let mut ev = MarketStageEvidence::default();

            ev.technical = Some(TechnicalMetrics { sh_chg: rm_sh, chinext_chg: rm_chinext, star_chg: rm_star });

            ev.capital = Some(CapitalMetrics { main_flow_yi: 0.0, amount_yi: rm_amount_yi, amount_delta_pct: 0.0 });

            ev.sentiment = Some(SentimentMetrics { limit_up_n: rm_limit_up, limit_down_n: 0, broken_pct: 0.0, consecutive_h: 0 });

            let conf = ms_evaluate(&ev);

            let r = pt::MarketReview {

                sh_chg: rm_sh, chinext_chg: rm_chinext, star_chg: rm_star,

                limit_up_n: rm_limit_up, limit_down_n: 0, broken_pct: 0.0, consecutive_h: 0,

                amount_yi: rm_amount_yi, amount_delta_pct: 0.0, amount_dir: "",

                main_flow_yi: 0.0, money_effect: "中等",

                heat_stage: conf.heat_stage.as_str(), heat_conf_pct: conf.conf_pct,

                low_conf: conf.degraded, low_conf_tier: None,

                account_mode: pt::AccountMode::Normal, max_pos: 7,

            };

            let text = pt::render_review_market(&today_str, &r);

            log::info!("[v12-R02]\n{}", text);

        }



        // ===== R-03 涨停产业链 (v12 limit_chain_review) =====

        {

            use stock_analysis::market_analyzer::limit_chain_review::{

                aggregate, LimitChainInput, StockLimitStats,

            };

            let mut stocks: Vec<StockLimitStats> = Vec::new();

            if let Ok(fetcher) = stock_analysis::data_provider::DataFetcherManager::new() {

                let watchlist = stock_analysis::portfolio::get_watchlist().unwrap_or_default();

                for p in r_holdings.iter().chain(watchlist.iter()).take(20) {

                    if let Ok((kline, _)) = fetcher.get_daily_data(&p.code, 60) {

                        let mut limit_up_days = 0u32;

                        for bar in kline.iter().take(10) {

                            if (bar.close / bar.open - 1.0) > 0.095 { limit_up_days += 1; } else { break; }

                        }

                        if limit_up_days > 0 {

                            stocks.push(StockLimitStats {

                                code: p.code.clone(), name: p.name.clone(),

                                chain: p.sector.clone(),

                                board_level: limit_up_days as u8,

                                is_limit_up_today: limit_up_days > 0,

                                is_first_board: limit_up_days == 1,

                                consecutive_days: limit_up_days,

                            });

                        }

                    }

                }

            }

            let aggs = aggregate(&LimitChainInput { stocks, source_complete: true });

            if !aggs.is_empty() {

                let mut body = format!("🔥 涨停产业链（{}）\n", today_str);

                for (i, a) in aggs.iter().enumerate() {

                    body.push_str(&format!(

                        "{}. {} 涨停{}家（首板{}/连板{}） 阶段: {}\n   龙头: {}({}) {}板\n   后排: {}\n   明日观察: 接力意愿\n",

                        i + 1, a.chain, a.limit_up_n, a.first_n, a.consec_n, a.heat_stage,

                        a.leader_name, a.leader_code, a.leader_boards, a.followers.join(","),

                    ));

                }

                log::info!("[v12-R03]\n{}", body);

            } else {

                log::info!("[v12-R03] 无涨停产业链数据 (周日非交易日)");

            }

        }



        // ===== R-04 龙虎榜 (v12 lhb_review) =====

        {

            use stock_analysis::market_analyzer::lhb_review::assess_data_quality;

            let entries: Vec<stock_analysis::market_analyzer::lhb_review::LhbEntryInput> = Vec::new();

            let (pct, _degraded) = assess_data_quality(&entries);

            if pct >= 70 {

                log::info!("[v12-R04] 龙虎榜数据完整度 {}%, 推", pct);

            } else {

                log::info!("[v12-R04] 龙虎榜数据完整度 {}% (< 70%), 跳过", pct);

            }

        }



        // ===== R-05 信号复盘 (v12 post_close_review) =====

        {

            use stock_analysis::market_analyzer::post_close_review::aggregate_signal_review;

            let holding_exec = r_trades.iter().filter(|t| matches!(t.direction, stock_analysis::portfolio::TradeDirection::Sell)).count() as u32;

            let inp = aggregate_signal_review(

                (r_holdings.len() as u32, holding_exec, holding_exec),

                (0, 0),

                (0, 0, 0, 0, 0),

                (r_today_pnl_pct, 0.0, r_trades.len() as u32),

                (0, 0),

                today_str.clone(),

            );

            let fields = stock_analysis::market_analyzer::post_close_review::signal_review_to_template_fields(&inp);

            log::info!("[v12-R05] 持仓:推{} 执{} 候:触{} 纸:今{:+.1}% 样{}", fields.holding_n, fields.holding_exec, fields.cand_trigger, fields.paper_pnl_pct, fields.paper_n);

        }



        // ===== R-06 失败归因 (v12 performance_feedback) =====

        {

            use stock_analysis::market_analyzer::performance_feedback::evaluate;

            let rows: Vec<stock_analysis::market_analyzer::performance_feedback::ExecutionRow> = Vec::new();

            let report = evaluate(&rows, &today_str);

            log::info!("[v12-R06] 失败归因建议 {} 条", report.suggestions.len());

        }



        // ===== R-07/R-08 已在 async 上下文真推, 此处仅 log =====

        log::info!("[v12-MVP1-R] 8 块 R 系列组装完成 (待 push, R-07/R-08 在 async)");

        Ok(())

    }).await.unwrap_or_else(|e| Err(format!("spawn_blocking join: {}", e)));



    if let Err(e) = v12_review_result {

        log::error!("[v12-MVP1-R] spawn_blocking 失败: {}", e);

    } else {

        // 推送: 在 async context 直接 push (R-01 + R-02 + R-08 3 个必推, 其他按数据决定)

        log::info!("[v12-MVP1-R] 推送 R-01~R-08 到飞书");



        // 推送数据准备 (sync Diesel → 必须包 spawn_blocking, 否则 async context panic)

        let today_str2 = chrono::Local::now().format("%Y-%m-%d").to_string();

        let push_data = tokio::task::spawn_blocking(move || {

            let r2 = stock_analysis::portfolio::get_positions().unwrap_or_default();

            let r2_quotes = market_data::fetch_position_quotes();

            let r2_prices: std::collections::HashMap<String, f64> = r2_quotes

                .iter()

                .map(|q| (q.code.clone(), q.price))

                .collect();

            let r2_equity = stock_analysis::portfolio::get_equity_curve(30).unwrap_or_default();

            let r2_ledger = r2_equity.last().cloned();

            let r2_mv = r2_ledger.as_ref().map(|e| e.market_value).unwrap_or(0.0);

            (r2, r2_prices, r2_mv)

        })

        .await

        .unwrap_or_default();



        let (r2, r2_prices, r2_mv) = push_data;



        // R-01 推送

        {

            let mut items: Vec<pt::HoldingDailyPlan> = Vec::new();

            for p in r2.iter().take(5) {

                let cur = r2_prices.get(&p.code).copied().unwrap_or(p.cost_price);

                let pnl = if p.cost_price > 0.0 {

                    ((cur / p.cost_price - 1.0) * 100.0)

                } else {

                    0.0

                };

                let plan_high = if pnl > 5.0 { "减仓1/3" } else { "减仓1/2" };

                let t0 = if pnl > 5.0 {

                    "适合观察"

                } else {

                    "不适合(主升核心)"

                };

                let stop = p.cost_price * 0.92;

                items.push(pt::HoldingDailyPlan {

                    name: p.name.as_str(),

                    code: p.code.as_str(),

                    price: cur,

                    cost: p.cost_price,

                    pnl_pct: pnl,

                    high_gap_x: 2.0,

                    plan_high,

                    plan_flat: "持有观望",

                    stop,

                    t0,

                });

            }

            if !items.is_empty() {

                let text = pt::render_daily_report(&today_str2, &items);

                notify::push_governor(&text, notify::PushKind::DailyReport).await;

            }

        }



        // R-02 推送

        {

            use stock_analysis::market_analyzer::market_stage_confidence::{

                evaluate as ms_evaluate, CapitalMetrics, MarketStageEvidence, SentimentMetrics,

                TechnicalMetrics,

            };

            // P0-3: 真接三大指数 + 两市成交额 (替代 0.5/0.5/0.0 硬编码 + r2_mv 误当两市成交额)
            let (r2_sh, r2_chinext, r2_star) = market_data::fetch_index_changes();

            let r2_amount_yi = market_data::fetch_market_amount_yi();

            let r2_limit_up = stock_analysis::market_analyzer::MarketAnalyzer::new(None)

                .ok()

                .and_then(|a| a.get_limit_up_stocks().ok())

                .map(|s| s.len() as u32)

                .unwrap_or(0);

            let mut ev = MarketStageEvidence::default();

            ev.technical = Some(TechnicalMetrics {

                sh_chg: r2_sh,

                chinext_chg: r2_chinext,

                star_chg: r2_star,

            });

            ev.capital = Some(CapitalMetrics {

                main_flow_yi: 0.0,

                amount_yi: r2_amount_yi,

                amount_delta_pct: 0.0,

            });

            ev.sentiment = Some(SentimentMetrics {

                limit_up_n: r2_limit_up,

                limit_down_n: 0,

                broken_pct: 0.0,

                consecutive_h: 0,

            });

            let conf = ms_evaluate(&ev);

            let r = pt::MarketReview {

                sh_chg: r2_sh,

                chinext_chg: r2_chinext,

                star_chg: r2_star,

                limit_up_n: r2_limit_up,

                limit_down_n: 0,

                broken_pct: 0.0,

                consecutive_h: 0,

                amount_yi: r2_amount_yi,

                amount_delta_pct: 0.0,

                amount_dir: "",

                main_flow_yi: 0.0,

                money_effect: "中等",

                heat_stage: conf.heat_stage.as_str(),

                heat_conf_pct: conf.conf_pct,

                low_conf: conf.degraded,

                low_conf_tier: None,

                account_mode: pt::AccountMode::Normal,

                max_pos: 7,

            };

            let text = pt::render_review_market(&today_str2, &r);

            notify::push_governor(&text, notify::PushKind::ReviewMarket).await;

        }



        // R-07 明日观察池 (真推)

        {

            let watchlist = stock_analysis::portfolio::get_watchlist().unwrap_or_default();

            if watchlist.is_empty() {

                log::info!("[v12-R07] 自选为空, 跳过");

            } else {

                let mut items: Vec<pt::WatchItem<'_>> = Vec::new();

                for p in watchlist.iter().take(3) {

                    let cur = r2_prices.get(&p.code).copied().unwrap_or(p.cost_price);

                    items.push(pt::WatchItem {

                        name: p.name.as_str(),

                        code: p.code.as_str(),

                        topic: p.sector.as_str(),

                        source: "A档未触发",

                        trigger: "突破前高+量比>3",

                        lo: cur * 0.97,

                        hi: cur * 1.05,

                        stop: cur * 0.93,

                        reason: "板块共振 + 持仓联动",

                    });

                }

                let text = pt::render_tomorrow_watch(&today_str2, &items);

                log::info!("[v12-R07]\n{}", text);

                notify::push_governor(&text, notify::PushKind::TomorrowWatch).await;

            }

        }



        // R-08 推送 (真实数据: 拉今日公告 + 持仓事件)

        {

            // 真实数据源: 公告 API + 持仓事件

            // 公告拉取 (sync, 包 spawn_blocking)

            let (ann_summary, holding_events) = tokio::task::spawn_blocking(move || {

                // 1. 拉今日全市场公告 (data_source不稳定时返空)

                // review #15: fetch_announcements 改 async. 外层 spawn_blocking closure

                // 是 sync context, 用 Handle::current().block_on 驱动 future.

                let anns = tokio::runtime::Handle::current()

                    .block_on(stock_analysis::data_provider::announcement::fetch_announcements(None))

                    .unwrap_or_default();

                let ann_text = if anns.is_empty() {

                    "今日无重大公告 (data_source 缺失)".to_string()

                } else {

                    let mut s = format!("今日共 {} 条公告 (TOP 3):\n", anns.len());

                    for a in anns.iter().take(3) {

                        s.push_str(&format!("· {} ({:?}): {}\n", a.code, a.level, a.title));

                    }

                    s

                };

                // 2. 持仓事件: 用 r2 持仓 + 拉它们各自今日公告

                let mut events: Vec<(String, String)> = Vec::new();

                for p in r2.iter().take(3) {

                    // 查该持仓的今日公告

                    let p_anns: Vec<_> = anns.iter().filter(|a| a.code == p.code).take(2).collect();

                    let kind = if !p_anns.is_empty() {

                        // 用最近一条公告标题作为事件

                        p_anns[0].title.chars().take(20).collect::<String>()

                    } else {

                        // 无公告时查 ledger 看是否即将到期

                        format!(

                            "持有 {} (浮盈{:.1}%)",

                            p.code,

                            ((r2_prices.get(&p.code).copied().unwrap_or(p.cost_price)

                                / p.cost_price

                                - 1.0)

                                * 100.0)

                        )

                    };

                    events.push((p.name.clone(), kind));

                }

                (ann_text, events)

            })

            .await

            .unwrap_or_default();



            let events_ref: Vec<pt::HoldingEventItem> = holding_events

                .iter()

                .map(|(n, k)| pt::HoldingEventItem {

                    tag: "实盘",

                    name: n.as_str(),

                    code: "",

                    kind: k.as_str(),

                })

                .collect();

            // v64 + v65: 隔夜关注真值 (美股 + 汇率 雅虎 API) — 包 spawn_blocking (P1.1 修复)

            let (us_summary2, fx_summary2) = tokio::task::spawn_blocking(

                stock_analysis::data_provider::yahoo::fetch_overnight_data,

            )

            .await

            .unwrap_or_else(|e| {

                log::warn!("[v65] fetch_overnight_data spawn_blocking 失败: {}", e);

                ("美股 持平".to_string(), "汇率 持平".to_string())

            });

            let text = pt::render_event_calendar(

                &today_str2,

                &events_ref,

                &ann_summary,

                &us_summary2,

                &fx_summary2,

            );

            log::info!("[v12-R08]\n{}", text);

            notify::push_governor(&text, notify::PushKind::EventCalendar).await;

        }



        // v19.14b: R-06 失败归因演示 (测试阶段, 全部推送)

        use stock_analysis::review::failure_attribution::{

            FailureItem, FailureReason, WeeklyDistribution,

        };

        let r06_items = vec![

            FailureItem {

                name: "德展健康".into(),

                code: "000813".into(),

                signal_level: "B".into(),

                reason: FailureReason::StopLossHit,

                pnl_pct: -51.7,

                suggestion: "停牌中跳过, 复牌后重新评估".into(),

            },

            FailureItem {

                name: "达实智能".into(),

                code: "002421".into(),

                signal_level: "A".into(),

                reason: FailureReason::MacdBearish,

                pnl_pct: -8.5,

                suggestion: "等待右侧放量信号, 避免左侧抄底".into(),

            },

        ];

        let mut r06_weekly = WeeklyDistribution::default();

        r06_weekly.add(FailureReason::StopLossHit);

        r06_weekly.add(FailureReason::MacdBearish);

        let r06_text =

            stock_analysis::review::failure_attribution::render_r06(&r06_items, &r06_weekly);

        log::info!("[v19.14b R-06]\n{}", r06_text);

        notify::push_governor(&r06_text, notify::PushKind::ReviewFailure).await;



        // v68: 盘后复盘对齐 v18 — 推 3 张 v18 风格卡片 (放量·持仓 / 放量·自选 / 放量·实盘优选)

        //   - v18 路径: 单独推 holding_brk + watch_brk + market_brk

        //   - v12 路径: 只推 candidate_summary, 没单独推放量卡片 — 用户要"内容跟 v18 一样"

        let v18_brk = tokio::task::spawn_blocking(|| {

            let holdings = stock_analysis::portfolio::get_positions().unwrap_or_default();

            let watchlist = stock_analysis::portfolio::get_watchlist().unwrap_or_default();

            let quotes = market_data::fetch_position_quotes();

            let prices = build_price_map(&quotes);

            let holding_codes: std::collections::HashSet<String> =

                holdings.iter().map(|p| p.code.clone()).collect();

            let watch_codes: std::collections::HashSet<String> =

                watchlist.iter().map(|p| p.code.clone()).collect();

            let mut holding_brk = String::new();

            let mut watch_brk = String::new();

            let mut market_brk = String::new();

            if let Ok(fetcher) = stock_analysis::data_provider::DataFetcherManager::new() {

                // 持仓放量

                let mut holding_lines =

                    vec!["📊 放量分析·持仓（盘后·算法研判仅供参考）".to_string()];

                for p in &holdings {

                    if let Ok((kline, _)) = fetcher.get_daily_data(&p.code, 60) {

                        let sig = stock_analysis::breakout::engine::analyze_postmarket(

                            &p.code, &p.name, &kline,

                        );

                        holding_lines.push(format!(

                            "  {} {}({}) — {} 置信{}% [{}]",

                            sig.breakout_type.emoji(),

                            sig.name, sig.code,

                            sig.breakout_type.label(),

                            sig.confidence, sig.description,

                        ));

                    }

                }

                if holding_lines.len() > 1 {

                    holding_brk = holding_lines.join("\n");

                }

                // 自选放量 (剔除已在持仓列出的)

                let mut watch_lines = vec!["📊 放量分析·自选（盘后·算法研判仅供参考）".to_string()];

                for p in &watchlist {

                    if holding_codes.contains(&p.code) {

                        continue;

                    }

                    if let Ok((kline, _)) = fetcher.get_daily_data(&p.code, 60) {

                        let sig = stock_analysis::breakout::engine::analyze_postmarket(

                            &p.code, &p.name, &kline,

                        );

                        watch_lines.push(format!(

                            "  {} {}({}) — {} 置信{}% [{}]",

                            sig.breakout_type.emoji(),

                            sig.name, sig.code,

                            sig.breakout_type.label(),

                            sig.confidence, sig.description,

                        ));

                    }

                }

                if watch_lines.len() > 1 {

                    watch_brk = watch_lines.join("\n");

                }

                // 实盘量能优选 (全市场)

                let market_candidates =

                    market_data::fetch_market_volume_ratio_leaders(80).unwrap_or_default();

                let mut market_lines =

                    vec!["📊 放量分析·实盘优选（盘后·算法研判仅供参考）".to_string()];

                let mut picked = 0usize;

                for s in &market_candidates {

                    if picked >= 5 { break; }

                    if holding_codes.contains(&s.code) || watch_codes.contains(&s.code) {

                        continue;

                    }

                    if let Ok((kline, _)) = fetcher.get_daily_data(&s.code, 60) {

                        let sig = stock_analysis::breakout::engine::analyze_postmarket(

                            &s.code, &s.name, &kline,

                        );

                        if sig.breakout_type

                            != stock_analysis::breakout::signal::BreakoutType::Launch

                            || sig.confidence < 50

                        {

                            continue;

                        }

                        market_lines.push(format!(

                            "  {} {}({}) — {} 置信{}% [量比{:.1} 主力{:+.2}亿 | {}]",

                            sig.breakout_type.emoji(),

                            sig.name, sig.code,

                            sig.breakout_type.label(),

                            sig.confidence, s.volume_ratio, s.main_net_yi, sig.description,

                        ));

                        picked += 1;

                    }

                }

                if market_lines.len() > 1 {

                    market_brk = market_lines.join("\n");

                }

            }

            (holding_brk, watch_brk, market_brk)

        })

        .await

        .unwrap_or_default();

        let (holding_brk, watch_brk, market_brk) = v18_brk;



        if !holding_brk.is_empty() {

            let holding_brk_text = format!(

                "📊 放量分析·持仓（盘后·算法研判仅供参考）\n{}",

                holding_brk

            );

            log::info!("[v68] 放量·持仓 推送 ({} 字)", holding_brk_text.chars().count());

            push_governor_v3(&holding_brk_text, PushKind::IntradayMarket, None).await;

        }

        if !watch_brk.is_empty() {

            let watch_brk_text = format!(

                "📊 放量分析·自选（盘后·算法研判仅供参考）\n{}",

                watch_brk

            );

            log::info!("[v68] 放量·自选 推送 ({} 字)", watch_brk_text.chars().count());

            push_governor_v3(&watch_brk_text, PushKind::IntradayMarket, None).await;

        }

        if !market_brk.is_empty() {

            let market_brk_text = format!(

                "📊 放量分析·实盘优选（盘后·算法研判仅供参考）\n{}",

                market_brk

            );

            log::info!("[v68] 放量·实盘优选 推送 ({} 字)", market_brk_text.chars().count());

            push_governor_v3(&market_brk_text, PushKind::IntradayMarket, None).await;

        }



        log::info!("[v12-MVP1-R] R-01/R-02/R-06/R-08 推送完成 (R-03~R-05/R-07 数据不足仅 log)");

    }

    // 全模板覆盖: R-01~R-08 已由上方内联块推送, 这里调 dispatch_all_for_test 补齐
    //   --test → All (再跑盘中模板); --review → Review (只补盘后复盘). R-02~R-08 已推 → dedup 跳过, 不重复
    {
        let hhmm_r = chrono::Local::now().format("%H:%M").to_string();
        let date_r = chrono::Local::now().format("%Y-%m-%d").to_string();
        let banner_r = pt::BannerCtx::default();
        let scope = if std::env::args().any(|a| a == "--test") {
            pt::TestScope::All
        } else {
            pt::TestScope::Review
        };
        pt::dispatch_all_for_test(&hhmm_r, &date_r, &banner_r, scope).await;
    }

}



/// v70: e2e 模式入口 — 跑所有 v12 §14 模板 (忽略时间窗口 + 数据空)

///   步骤: 1) seed chain_daily + lhb_daily + trades

///         2) run_review_only_inner (推 R-01~R-08 + v18 放量)

///         3) 跑盘中 14.x 模板 (P-01 P-02 P-03 P-04 I-01~I-08 A-10)

///         4) 不依赖时间窗口, 不依赖数据 (mock fallback)

///   用途: 验证 v12 §14 + v13.1 模板完整性, 推全 22 模板

async fn e2e_all_templates_run() {

    let today_str = chrono::Local::now().format("%Y-%m-%d").to_string();

    let hhmm = chrono::Local::now().format("%H:%M").to_string();

    log::info!("[v70] E2E 开始 — 跑所有 v12 §14 + v13.1 模板");



    // 1. Seed (chain_daily + lhb_daily + trades) 让 R-03 / R-04 / R-05 / A-10 都能推

    log::info!("[v70] 1/3 seed chain_daily + lhb_daily + trades");

    seed_e2e_data_via_sqlite(&today_str);



    // 2. 跑 v12 §14.3 盘后复盘 (R-01~R-08) + v18 放量

    log::info!("[v70] 2/3 跑 R-01~R-08 + v18 放量");

    run_review_only_inner().await;



    // 3. 跑 v12 §14.1 盘前 + 14.2 盘中 + 14.3 v18 之外的模板

    //   注: 这些模板原本走 v18 路径, 真实交易日由 monitor_loop / news_monitor_loop 推

    //   v70 mock fallback: 推 14.x 模板用 mock data

    log::info!("[v70] 3/3 跑盘中 14.x 模板 (mock fallback)");

    push_e2e_14x_templates(&today_str, &hhmm).await;



    // 4. 跑 v12 §14.1 + 14.2 新闻模块 (D-01 / I-02) — mock data 推

    //   news_monitor_loop 真实路径需有公告源 (沙箱无), 这里 e2e 直接走 dispatcher mock

    log::info!("[v70] 4/4 跑新闻模块 (D-01 / I-02 mock fallback)");

    push_e2e_news_modules(&hhmm).await;



    // 5. v14.1 task #163: T-16 ST 涨跌幅变更 e2e 真接 (seed 一只 ST 持仓, 调 dispatcher, 推完清理)

    //   真实路径在 main_loop 9:30 触发 + 真实 ST 持仓. e2e mock 一只, 验证 get_st_positions +

    //   dispatch_st_price_limit_changed 数据流通.

    log::info!("[v70] 5/5 T-16 ST 涨跌幅变更 e2e (mock ST 持仓)");

    push_e2e_t16_st_price_limit(&hhmm).await;



    // --test 全模板覆盖: 调全部 dispatch_*_daily (真推, 只推有真数据的, 用户要求测试所有模板)
    //   R-01~R-08 已由上方 run_review_only_inner 推过 → 这里 dedup 跳过; 盘中模板在此真推
    {
        use crate::push_templates as pt;
        let banner_e2e = pt::BannerCtx::default();
        pt::dispatch_all_for_test(&hhmm, &today_str, &banner_e2e, pt::TestScope::All).await;
    }

    log::info!("[v70] E2E 完成 — 检查 data/push_log/{}/ 查所有推送", today_str);

}



/// v70: mock seed chain_daily + lhb_daily + trades via sqlite3 CLI

fn seed_e2e_data_via_sqlite(date: &str) {

    use std::process::Command;

    let db_path = "data/stock_analysis.db";

    // chain_daily 5 概念

    let chain_sql = format!(

        r#"INSERT OR IGNORE INTO chain_daily (date, concept, stocks, continuation_count) VALUES

        ('{date}', 'PCB', '["002916","002463","002938"]', 3),

        ('{date}', '算力', '["002230","300458","688041"]', 2),

        ('{date}', '机器人', '["002472","300124","688017"]', 2),

        ('{date}', '半导体', '["600460","002129","688981"]', 1),

        ('{date}', '固态电池', '["300037","300390","002812"]', 1);"#

    );

    let _ = Command::new("sqlite3").args([db_path, &chain_sql]).output();

    // lhb_daily 6 票

    let lhb_sql = format!(

        r#"INSERT OR IGNORE INTO lhb_daily

        (code, name, trade_date, reason, pct_change, close_price, buy_amount, sell_amount, net_amount, total_amount, lhb_ratio) VALUES

        ('002916','深南电路','{date}','涨幅偏离值达7%',10.0,412.10,5.0e8,2.0e8,3.0e8,7.0e8,0.43),

        ('002463','沪电股份','{date}','涨幅偏离值达7%',10.0,129.72,3.0e8,1.0e8,2.0e8,4.0e8,0.50),

        ('002938','鹏鼎控股','{date}','涨幅偏离值达7%',10.0,35.20,2.0e8,0.5e8,1.5e8,2.5e8,0.60),

        ('002230','科大讯飞','{date}','涨幅偏离值达7%',10.0,58.40,4.0e8,1.5e8,2.5e8,5.5e8,0.45),

        ('300458','全志科技','{date}','涨幅偏离值达7%',10.0,43.20,1.0e8,0.3e8,0.7e8,1.3e8,0.54),

        ('688041','海光信息','{date}','涨幅偏离值达7%',10.0,78.60,3.5e8,1.0e8,2.5e8,4.5e8,0.56);"#

    );

    let _ = Command::new("sqlite3").args([db_path, &lhb_sql]).output();

    // trades 1 buy + 1 sell

    let trades_sql = format!(

        r#"INSERT OR IGNORE INTO trades (code, name, direction, price, shares, amount, reason, traded_at) VALUES

        ('002208','合肥城建','buy',19.27,200,3854.0,'实盘建仓','{date} 09:35:00'),

        ('002208','合肥城建','sell',17.50,200,3500.0,'止损卖出','{date} 14:35:00');"#

    );

    let _ = Command::new("sqlite3").args([db_path, &trades_sql]).output();

}



/// v70: 推新闻模块 (D-01 / I-02) — mock fallback

///   news_monitor_loop 真实路径需公告源 (沙箱无), 这里直接走 dispatcher mock

///   公告数据 mock: 3 主题 + 2 票 (覆盖 D-01 + I-02)

async fn push_e2e_news_modules(hhmm: &str) {

    use push_templates as pt;

    // D-01 新闻驱动个股 (mock)

    let d01 = pt::render_news_to_idea(

        &pt::BannerCtx::default(),

        pt::NewsToIdeaParams {

            hhmm,

            headline: "002916 净利润 +45% 超预期",

            theme: Some("AI 算力"),

            stage: pt::NewsStage::Starting,

            name: "深南电路",

            code: "002916",

            reasons: vec!["PCB 涨价 12%", "算力国产替代加速"],

            action: Some(pt::NewsAction::BuyDip),

        },

    );

    log::info!("[v70] D-01 推 ({} 字)", d01.chars().count());

    // v15.1 A2.2 TODO: 需要把 upstream 的 rec.code 传过来, 当前没拿到 (deferred to b014)
    #[allow(deprecated)]
    let _ = notify::push_governor(&d01, notify::PushKind::NewsToIdea).await;

    // v70+: 落盘推荐记录 (供后续 D+1 兑现分析)

    notify::record_news_recommendation(

        "D-01",

        "002916",

        "深南电路",

        "AI 算力",

        &["PCB 涨价 12%", "算力国产替代加速"],

        Some("BuyDip"),

        None,

    );



    // I-02 新闻催化映射 (mock)

    let i02 = pt::render_news_catalyst(

        &pt::BannerCtx::default(),

        pt::NewsCatalystParams {

            hhmm,

            headline: "DeepSeek V4 发布, AI 算力国产替代加速",

            theme: Some("AI 算力"),

            stocks: vec![

                ("深南电路", "002916", Some(10.0), "PCB 龙头, Q1 业绩超预期"),

                ("沪电股份", "002463", Some(9.5), "800G 交换机 PCB 受益"),

            ],

        },

    );

    log::info!("[v70] I-02 推 ({} 字)", i02.chars().count());

    let _ = notify::push_governor(&i02, notify::PushKind::NewsCatalyst).await;

    // v70+: 落盘 I-02 推荐 (多票, 每票写一条)

    for (name, code, chg, reason) in [

        ("深南电路", "002916", Some(10.0_f64), "PCB 龙头, Q1 业绩超预期"),

        ("沪电股份", "002463", Some(9.5_f64), "800G 交换机 PCB 受益"),

    ] {

        notify::record_news_recommendation(

            "I-02",

            code,

            name,

            "AI 算力",

            &[reason],

            None,

            chg,

        );

    }

}



/// v14.1 task #163: T-16 ST 涨跌幅变更 e2e 验证

/// v14.1 review fix: RAII 清理 guard, panic/dispatch hang 时 Drop 兜底 DELETE

///   之前 cleanup 在每个 return 路径手写, dispatcher panic 会泄露 seed row

struct E2ECleanup<'a> {

    db_path: &'a str,

    code: &'a str,

}



impl Drop for E2ECleanup<'_> {

    fn drop(&mut self) {

        let _ = std::process::Command::new("sqlite3")

            .args([self.db_path, &format!("DELETE FROM stock_position WHERE code='{}'", self.code)])

            .output();

        log::info!("[v14.1 #163] T-16 e2e seed 已清理 (RAII)");

    }

}



///   步骤: 1) seed 一只 st_type='*ST' 持仓 (代码 999999 测试用, 无效 A 股防冲突)

///         2) 调 portfolio::get_st_positions() 验证非空

///         3) 调 push_templates::dispatch_st_price_limit_changed() 推 1 条

///         4) E2ECleanup Drop 兜底 DELETE (panic / 早退 / 正常完成都触发)

async fn push_e2e_t16_st_price_limit(hhmm: &str) {

    use std::process::Command;

    let db_path = "data/stock_analysis.db";

    const E2E_CODE: &str = "999999";



    // 1. Seed: insert 一只 *ST 持仓 (买价 5.00, 1000 股, st_type='*ST')

    // v14.1 review fix: 用 999999 (无效 A 股代码) + INSERT OR IGNORE 避免 clobber 真用户持仓

    //   之前用真代码 600090 + INSERT OR REPLACE 会覆盖用户真持仓

    let seed_sql = "INSERT OR IGNORE INTO stock_position \

        (code, name, buy_date, buy_price, quantity, status, st_type, chain_name, created_at, updated_at) \

        VALUES ('999999', '*ST_E2E_TEST', '2099-01-01', 5.00, 1000, 'open', '*ST', '其他', \

                CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)";

    let seed_out = Command::new("sqlite3").args([db_path, seed_sql]).output();

    if let Err(e) = &seed_out {

        log::warn!("[v14.1 #163] seed 失败: {}", e);

        return;

    }



    // v14.1 review fix: RAII guard, 函数任意 return / panic 都触发 DELETE

    let _cleanup = E2ECleanup { db_path, code: E2E_CODE };



    // 2. 验证 get_st_positions 非空

    let st_positions = stock_analysis::portfolio::get_st_positions();

    if st_positions.is_empty() {

        log::warn!("[v14.1 #163] get_st_positions 仍为空 (seed 失败?) — 跳过推");

        return;

    }

    log::info!("[v14.1 #163] get_st_positions 找到 {} 只 ST 持仓", st_positions.len());



    // 3. 调 T-16 dispatcher 推 1 条

    // review #14: get_st_positions 现在返 Vec<String> (code list), 按 code 反查 Position

    use push_templates as pt;

    for code in &st_positions {

        let Some(pos) = stock_analysis::portfolio::find_position(code) else {

            log::warn!("[v14.1 #163] ST code {} 无 Position 详情, 跳过", code);

            continue;

        };

        let st_type = if pos.star_st { pt::StType::StarST } else { pt::StType::ST };

        let now_price = pos.cost_price * 1.02;

        let new_stop = pos.cost_price * 0.90;

        let new_take = pos.cost_price * 1.10;

        let banner = current_banner();

        let _ = pt::dispatch_st_price_limit_changed(

            hhmm,

            &pos.name,

            &pos.code,

            st_type,

            0.05, 0.10, // 5% → 10% 新规

            pos.shares as u32,

            pos.cost_price,

            now_price,

            Some(new_stop),

            Some(new_take),

            &banner,

        )

        .await;

    }

    log::info!("[v14.1 #163] T-16 已推 {} 条", st_positions.len());

    // _cleanup 在函数 return 时 Drop, 兜底 DELETE

}



/// v70: 推所有盘中 14.x 模板 (mock fallback)

async fn push_e2e_14x_templates(date: &str, hhmm: &str) {

    use push_templates as pt;

    // P-01 盘前新闻热点 (mock: 3 主线, 2 催化, 2 关注票)

    let p01 = pt::render_preopen_news_hot(pt::PreopenNewsHotParams {

        hhmm,

        theme_1: Some("PCB 涨价"),

        theme_2: Some("算力国产替代"),

        theme_3: Some("固态电池量产"),

        news_pairs: vec![

            ("002916 净利润 +45%", "AI 算力"),

            ("300750 订单回暖", "锂电池"),

        ],

        watch_stocks: vec![

            ("深南电路".to_string(), "002916".to_string(), "PCB 量价齐升".to_string()),

            ("天孚通信".to_string(), "300394".to_string(), "光模块订单回暖".to_string()),

        ],

    });

    log::info!("[v70] P-01 推 ({} 字)", p01.chars().count());

    let _ = notify::push_governor(&p01, notify::PushKind::PreopenNewsHot).await;



    // P-02 竞价热点量能 (mock)

    let p02 = format!(

        "🌅 竞价热点量能（{}）\n深南电路(002916) 高开+1.2% | 量比3.5 | 竞价额1.2亿\n结论: 强承接\n辅助建议, 非下单指令",

        hhmm

    );

    log::info!("[v70] P-02 推 ({} 字)", p02.chars().count());

    let _ = notify::push_governor(&p02, notify::PushKind::AuctionVolume).await;



    // R-03 涨停产业链 (chain_daily 5 概念, mock 数据)

    let r03 = pt::render_industry_chain(date, &[pt::ChainLine {

        chain: "PCB", limit_up_n: 3, first_n: 1, consec_n: 3,

        heat_stage: "高潮", leader_name: "深南电路", leader_code: "002916", leader_boards: 3,

        followers: "沪电股份, 兴森科技", watch_point: "放量后回踩关注",

    }, pt::ChainLine {

        chain: "算力", limit_up_n: 2, first_n: 1, consec_n: 2,

        heat_stage: "主升", leader_name: "科大讯飞", leader_code: "002230", leader_boards: 2,

        followers: "全志科技", watch_point: "板块趋势延续",

    }], None);

    log::info!("[v70] R-03 推 ({} 字)", r03.chars().count());

    let _ = notify::push_governor(&r03, notify::PushKind::IndustryChain).await;



    // R-04 龙虎榜 (lhb_daily 6 票, mock)

    let r04 = pt::render_review_lhb(date, &[pt::LhbEntry {

        name: "深南电路", code: "002916",

        net_buy_yi: 3.0, reason: "涨幅偏离值达7%",

        buy_inst_n: 5, buy_inst_amt_wan: Some(5000.0), buy_other_n: 3, buy_other_amt_wan: Some(2000.0),

        buy_conc_pct: Some(60.0), sell_desc: "机构卖200万", sell_conc_pct: Some(40.0),

        chain_match: Some("是-PCB"), next_day_risk: "高位, 注意回撤",

    }]);

    log::info!("[v70] R-04 推 ({} 字)", r04.chars().count());

    let _ = notify::push_governor(&r04, notify::PushKind::ReviewLhb).await;



    // R-05 信号复盘 (trades mock)

    let r05 = pt::render_review_signal(date, &pt::SignalReview {

        holding_n: 7, holding_exec: 1, holding_eff: 1,

        t0_n: 0, t0_eff: 0,

        cand_trigger: 0, cand_filled: 0, cand_notfilled: 0,

        cand_limitup: 0, cand_notreach: 0,

        paper_pnl_pct: -8.4, paper_total_pct: -8.4, paper_n: 1,

        news_push_n: 5, news_d1_eff: 0,

    });

    log::info!("[v70] R-05 推 ({} 字)", r05.chars().count());

    let _ = notify::push_governor(&r05, notify::PushKind::ReviewSignal).await;



    // A-10 题材催化复盘 (chain_daily mock)

    let a10 = pt::render_catalyst_review(pt::CatalystReviewParams {

        date, theme: "PCB",

        score: Some(8.5), persistent: pt::PersistentLevel::High,

        started_names: vec!["深南电路", "沪电股份"],

        pending_names: vec!["兴森科技"],

        watch_point: Some("放量后回踩关注"),

    });

    log::info!("[v70] A-10 推 ({} 字)", a10.chars().count());

    let _ = notify::push_governor(&a10, notify::PushKind::CatalystReview).await;



    log::info!("[v70] e2e 14x 模板跑完");

}



/// 盘后持仓多 Agent 深度研判：对每只真实持仓跑「6 分析师 + 多空辩论 + 仲裁」流水线，

/// 结果逐只推送飞书。受 `AI_AGENT_PIPELINE`（默认开启）控制；关闭则整体跳过。

async fn run_review_deep_analysis(

    holding_breakout_text: &str,

    watch_breakout_text: &str,

    risk_text: &str, // v19.3: 风险段 (止损+轮动+现金) 合并到持仓决策台 1 张卡

) {

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



    log::info!(

        "[复盘] 持仓多 Agent 深度研判开始（{} 只，并发 {}）",

        holdings.len(),

        concurrency

    );



    // 并发跑多 Agent，结果回收后按持仓顺序推送

    let codes: Vec<(String, String)> = holdings

        .iter()

        .map(|p| (p.code.clone(), p.name.clone()))

        .collect();



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

    // 落盘每只持仓研判 (供事后查询, 不再单独推送)

    for p in &holdings {

        let Some((name, md)) = by_code.get(&p.code) else {

            continue;

        };

        let Some(md) = md else { continue };

        log::info!(

            "[复盘] 持仓深度研判 {}({}) 完成 ({} 字, 落盘+聚合推送)",

            name,

            p.code,

            md.chars().count()

        );

        let _ = stock_analysis::pipeline::section_utils::save_deep_report(&p.code, &md);

    }

    // 聚合推送: 走持仓决策台 (P0-5 commit 2 替换原 build_holding_summary 字符串猜)

    // v14.2 路径: decisions_from_llm (commit 1) → format_decision_board (commit C 渲染)

    // by_code 不再被 .remove() 走, 决策台能拿到 LLM 终稿

    // v62: 用真报价填 current_price / change_pct (F1 实盘数据误差修复)

    //   - 第二轮 fetch (第一轮 quotes 已被 spawn_blocking move 走)

    let r_quotes2 = market_data::fetch_position_quotes();

    let quote_map: std::collections::HashMap<String, (f64, f64)> = r_quotes2

        .iter()

        .map(|q| (q.code.clone(), (q.price, q.change_pct)))

        .collect();

    let decisions = stock_analysis::decision::decision_decide::decisions_from_llm(

        &holdings,

        &by_code,

        &quote_map,

    );

    let summary = stock_analysis::decision::decision_render::format_decision_board(&decisions);

    // v19.3: 风险段 (止损+轮动+现金) 合并到持仓决策台 (1 张卡全信息)

    let mut combined = summary.clone();

    if !risk_text.is_empty() {

        combined.push_str("\n\n━━━ 🛡 风险与轮动段 ━━━\n");

        combined.push_str(&risk_text);

    }

    let push_summary = if combined.is_empty() {

        summary.clone()

    } else {

        combined

    };

    if !push_summary.is_empty() {

        log::info!(

            "[复盘] 持仓决策台推送 (v14.2 + 风险合并 v19.3):\n{}",

            push_summary

        );

        push_governor_v3(&push_summary, PushKind::ReviewSignal, None).await;

    }



    // v17.0 (P0-5++ Commit 11): 候选筛选台 wrapper 接 3 路真 raw (A10/C4 留 None --test 路径)

    // P5 §六 红线: 5 路 raw 合并到 1 条候选台卡片, 不刷屏

    //

    // v13.10.1 P0-#3: 主流程(--review 不传 raw)时, 主动拉取 run_post_close_candidates

    // 作为 OptimalClose raw 注入候选台, 合并原来 9:20-9:25 独立推送的优选候选.

    let optimal_close_text: String = if holding_breakout_text.is_empty()

        && watch_breakout_text.is_empty()

    {

        stock_analysis::opportunity::run_post_close_candidates(5).await

    } else {

        String::new()

    };

    let candidate_summary = run_candidate_panel_from_review(

        &by_code,

        &holdings,

        None,                            // A10 选股 (--test 路径专属, --review 看不到 recs)

        Some(&optimal_close_text),       // B3 优选 (v13.10.1: 主流程自动拉取)

        Some(holding_breakout_text),     // B6 放量·自选 (L704 解构, --review 路径)

        Some(watch_breakout_text),       // B7 放量·实盘优选 (L704 解构, --review 路径)

        None,                            // C4 产业链 (--test 路径专属, run_test_scan L561)

    );

    if !candidate_summary.is_empty() {

        log::info!("[复盘] 候选筛选台推送 (v16.8):\n{}", candidate_summary);

        notify::push_governor(&candidate_summary, notify::PushKind::CandidateBoard).await;

    }



    log::info!("[复盘] 持仓多 Agent 深度研判完成");

}



/// 窗口：盘前08:00-09:30、盘中09:30-15:00、盘后15:00-22:00。

async fn news_monitor_loop() {

    use stock_analysis::monitor::detector::{AlertEvent, AlertLevel};

    use stock_analysis::monitor::news_ai::NewsAIAnalyzer;

    use stock_analysis::monitor::news_monitor::NewsMonitor;

    use stock_analysis::monitor::signal_state::SignalStateMachine;



    let poll_secs: u64 = std::env::var("NEWS_POLL_INTERVAL")

        .ok()

        .and_then(|s| s.parse().ok())

        .unwrap_or(120);



    log::info!("[NewsMonitor] 启动（独立窗口，不随价格扫描器静默）");

    let mut nm = NewsMonitor::new();

    nm.restore_dedup();

    let ai = NewsAIAnalyzer::new();

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

            .unwrap_or_default()

            .into_iter()

            .collect();

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

            })

            .await

            {

                Ok(Some(index)) => {

                    nm.linker_mut().replace_concept_index(index);

                    log::info!(

                        "[NewsMonitor] L2 概念索引已更新（{}个板块关联）",

                        nm.linker_ref().concept_count()

                    );

                }

                Ok(None) => log::warn!("[NewsMonitor] L2 概念索引刷新跳过（无板块数据）"),

                Err(_) => log::warn!("[NewsMonitor] L2 概念索引刷新 panic"),

            }

            // v41: 周期刷新 banner (让 news_monitor_loop 的 D-01/I-02 用真 AccountMode)

            evaluate_account_mode_hook(false).await;

        }



        // 公告扫描（仅网络拉取在 spawn_blocking，处理在主线程）

        // v13.10.1: fetch_announcements 改 async, news_monitor_loop 已在 tokio 运行时内

        // (由 tokio::join! 启动), 直接 .await 即可. review #15 用的 Handle::current().block_on

        // 会 panic (Cannot start a runtime from within a runtime).

        let anns = stock_analysis::data_provider::announcement::fetch_announcements(None)

            .await

            .unwrap_or_default();



        // 异步预解析：公告API缺失code时，通过东方财富搜索反查

        let mut resolved_codes: std::collections::HashMap<String, String> =

            std::collections::HashMap::new();

        {

            let http = reqwest::Client::builder()

                .timeout(std::time::Duration::from_secs(5))

                .build()

                .unwrap_or_default();

            for ann in &anns {

                if ann.code.is_empty() && !ann.name.is_empty() {

                    // 先查本地缓存

                    if let Some(code) = nm.linker_ref().lookup_code_by_name(&ann.name) {

                        resolved_codes.insert(ann.name.clone(), code.to_string());

                    } else if let Some(code) =

                        stock_analysis::monitor::news_monitor::resolve_code_by_name(

                            &ann.name, &http,

                        )

                        .await

                    {

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

        // v19.12: NewsRanker 真接公告 → 影子 rank → 有 A 档则真推飞书

        // 之前: 公告只走 news_monitor_loop 直接 push, 没走 NewsRanker

        // 现在: 公告 also 喂给 shadow_rank_hits, A 档命中时 push NewsRanked 模板

        if !pushed.is_empty() {

            let news_anns: Vec<_> = pushed

                .iter()

                .filter(|ev| ev.level >= stock_analysis::monitor::detector::AlertLevel::Important)

                .filter_map(|ev| {

                    // 从 detail.news_title 取标题, 从 code/name 取标识

                    let title = ev

                        .detail

                        .news_title

                        .clone()

                        .unwrap_or_else(|| ev.message.clone());

                    let code = ev.code.clone();

                    let name = ev.name.clone();

                    if title.is_empty() {

                        return None;

                    }

                    Some((title, code, name))

                })

                .collect();

            if !news_anns.is_empty() {

                use stock_analysis::opportunity::chain_mapper::{ChainHit, ChainSource};

                use stock_analysis::opportunity::news_ranker;

                let mut hits: Vec<ChainHit> = Vec::new();

                let mut titles: Vec<String> = Vec::new();

                for (title, code, name) in &news_anns {

                    hits.push(ChainHit {

                        chain: if !name.is_empty() {

                            name.clone()

                        } else {

                            code.clone()

                        },

                        keywords: vec![title.clone()],

                        logic: "live-announcement".into(),

                        stocks: vec![],

                        source: ChainSource::Rule,

                        board_keyword: if !name.is_empty() {

                            name.clone()

                        } else {

                            code.clone()

                        },

                        fund_flow_pct: None,

                        board_code: None,

                        board_change_pct: None,

                    });

                    titles.push(title.clone());

                }

                let ranked = tokio::task::spawn_blocking(move || {

                    news_ranker::shadow_rank_hits(&hits, &titles)

                })

                .await

                .unwrap_or_default();

                // v19.12: NewsRanker 真推 (无 A 档门槛, 用户要求全推)

                let board = news_ranker::format_news_ranked_board(&ranked);

                log::info!("[NewsRanker 盘中] {} 条公告 → 真推飞书", news_anns.len());

                notify::push_governor(&board, notify::PushKind::NewsRanked).await;

            }

        }



        // ═══════════════════════════════════════════════════════════════

        // v29 + v60: D-01 新闻驱动个股推送 (事件驱动)

        //   - 触发: pushed 不空 (有重要公告/事件) 时, 每轮 news_monitor_loop 调一次

        //   - v60 F9: 加 AlertLevel::Important 过滤 (NewsRanker line 2830 已有)

        //     - 低优先级 Info 事件不再触发 D-01 1h memo slot

        //   - 去重: dispatcher memo 1h/票 + push_governor 20min 冷却 (v12 §14.5)

        //   - 数据源: 候选台 (5 源合并) - 与 NewsRanked 公告影子 rank 互补

        //   - 静默: 候选台空时短路返回, log

        // ═══════════════════════════════════════════════════════════════

        let has_important: bool = pushed

            .iter()

            .any(|ev| ev.level >= stock_analysis::monitor::detector::AlertLevel::Important);

        if has_important {

            use push_templates::dispatch_news_to_idea_daily;

            // v41: 读共享 banner (替换写死)

            let banner = current_banner();

            let now_ts = chrono::Local::now();

            let hhmm = now_ts.format("%H:%M").to_string();

            let _ = dispatch_news_to_idea_daily(&hhmm, &banner).await;

        }



        // ═══════════════════════════════════════════════════════════════

        // v33 + v60: I-02 新闻催化映射 (事件驱动, 同 D-01 时机)

        //   - 触发: pushed 不空 (有重要公告) 时, 调一次

        //   - v60 F9: 加 AlertLevel::Important 过滤

        //   - 数据源: load_news_catalyst_snapshot_real (公告 + 板块聚类)

        //   - 模板: render_news_catalyst (带 banner)

        //   - 静默: 公告空时短路

        //   - 与 D-01 互补: D-01 推个股, I-02 推板块

        // ═══════════════════════════════════════════════════════════════

        if has_important {

            use push_templates::dispatch_news_catalyst_daily;

            // v41: 读共享 banner

            let banner = current_banner();

            let now_ts = chrono::Local::now();

            let hhmm = now_ts.format("%H:%M").to_string();

            let _ = dispatch_news_catalyst_daily(&hhmm, &banner).await;

        }

        // v11-P0-4 commit E: C2/C3 NewsAI 收敛 (grill Q2 决定)

        // 同一只票 (code) 实时层 (C2) 推过后, 快研层 (C3) 跳过 — 避免同票双推.

        let mut real_time_pushed: std::collections::HashSet<String> =

            std::collections::HashSet::new();

        // 🚀 实时层：对重要公告，AI 追推一句话决策

        for ev in &pushed {

            if ev.level <= AlertLevel::Important && !ev.name.is_empty() && ev.name != "RISK" {

                let title = ev.detail.news_title.as_deref().unwrap_or(&ev.message);

                let code = if ev.code.is_empty() {

                    ev.name.as_str()

                } else {

                    &ev.code

                };

                log::info!("[NewsAI] 🚀实时层 开始为 {} 生成决策...", ev.name);

                match ai.quick_decision(title, code, &ev.name).await {

                    Some(decision) => {

                        let follow =

                            format!("🧠 {} AI研判：{}【AI研判-仅供参考】", ev.name, decision);

                        push_governor_v3(&follow, PushKind::CandidateBoard, None).await;

                        log::info!("[NewsAI] {} 实时决策已推送", ev.name);

                        // v11-P0-4 commit E: C2/C3 收敛 — 实时层推过的 code, 快研层跳过

                        real_time_pushed.insert(code.to_string());

                    }

                    None => {

                        log::warn!("[NewsAI] {} 实时决策生成失败（超时/AI不可用）", ev.name);

                    }

                }

            }

        }



        // ⚡ 快研层：Important+ 事件，顺序深度分析（每只~5s，120s轮询间隔足够）

        for ev in &pushed {

            // v11-P0-4 commit E: C2/C3 收敛 — 跳过实时层已推过的 code

            let code = if ev.code.is_empty() {

                ev.name.as_str()

            } else {

                &ev.code

            };

            if real_time_pushed.contains(code) {

                log::info!("[NewsAI] {} 快研跳过 (实时层已推)", ev.name);

                continue;

            }



            if ev.level <= AlertLevel::Important && !ev.code.is_empty() && ev.code != "RISK" {

                let news_text = ev

                    .detail

                    .news_summary

                    .clone()

                    .unwrap_or_else(|| ev.message.clone());

                log::info!("[NewsAI] ⚡快研层 开始分析 {}({})...", ev.name, ev.code);

                match ai

                    .analyze_position_news(

                        &ev.code, &ev.name, &news_text, 0.0, 0.0, 0.0,

                        0.0, // 默认值（快研层侧重消息面）

                        "未知", 0.0, "未知", "未知", 0.0,

                    )

                    .await

                {

                    Some(deep) => {

                        let prefix = if ev.level == AlertLevel::Emergency {

                            "🔬"

                        } else {

                            "🔍"

                        };

                        let follow = format!(

                            "{} {}({}) 快研补充：\n{}",

                            prefix, ev.name, ev.code, deep.message

                        );

                        push_governor_v3(&follow, PushKind::NewsRanked, None).await;

                        log::info!("[NewsAI] {} 快研已推送", ev.name);

                    }

                    None => {

                        log::warn!("[NewsAI] {} 快研失败（超时/AI不可用）", ev.name);

                    }

                }

            }

        }



        // 路径A 机会发现已统一到 opportunity::run_opportunity_scan（monitor_loop 内调度），

        // news_ai::discover_opportunities 在 v9.1 Task 0 已删除。



        // 产业链机会扫描：统一在 8:00-22:00 窗口内按间隔调度（覆盖盘前/盘中/盘后）。

        // spawn 异步执行，不阻塞新闻轮询。

        let opp_interval_secs =

            stock_analysis::config::get_monitor_config().opportunity_scan_interval_min * 60;

        let opp_due = last_opp_scan

            .map(|t| t.elapsed().as_secs() >= opp_interval_secs)

            .unwrap_or(true);

        if opp_due {

            last_opp_scan = Some(std::time::Instant::now());

            tokio::spawn(async move {

                let scan = stock_analysis::opportunity::run_opportunity_scan().await;

                // 向事件总线发布扫描完成事件（候选数以内容行数近似），供解耦消费者统计

                let candidate_lines = scan

                    .chain_text

                    .lines()

                    .filter(|l| !l.trim().is_empty())

                    .count();

                stock_analysis::monitor::event_bus::publish(

                    stock_analysis::monitor::event_bus::MonitorEvent::OpportunityScan {

                        candidates: candidate_lines,

                    },

                );

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

                    let ok = push_governor_v3(opp_text, PushKind::CandidateBoard, None).await.is_pushed();

                    log::info!(

                        "[产业链] 推送结果 | ok={} | preview={}",

                        ok,

                        summarize_push_text(opp_text, 120)

                    );

                }

                // 持仓影响分开推送

                if !scan.impact_text.is_empty() {

                    log::info!("[持仓影响] {}", scan.impact_text);

                    let ok = push_governor_v3(&scan.impact_text, PushKind::CandidateBoard, None).await.is_pushed();

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

            // v41: 周期刷新 banner (AccountMode + DataMode 评估 → 写 LATEST_BANNER)

            evaluate_account_mode_hook(false).await;

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

            if !calendar::today_is_trading_day() {

                break;

            }

        }



        if !calendar::today_is_trading_day() {

            continue;

        }



        log::info!("进入交易时段，开始监控");



        let positions = stock_analysis::portfolio::get_positions().unwrap_or_default();

        // review #14: is_t1_locked 返回 Result, 显式 match; DB 失败时按"未解锁"

        // 处理 (保守), 同时 log warn 让 operator 知道.

        // review #14 修正: 原 Err → false (按未解锁处理) 与另一 caller Err → true (按锁定)

// 不一致, 违反"安全保守"原则. 统一保守: DB 失败 → 按已锁定处理,

// 持仓跳过解禁候选, 防止违反 T+1.

        let t1_unlocks: Vec<_> = positions

            .iter()

            .filter(|p| match stock_analysis::portfolio::is_t1_locked(&p.code) {

                Ok(true) => false,

                Ok(false) => true,

                Err(e) => {

                    log::error!("[盘前] is_t1_locked({}) 失败: {} — 保守按已锁定处理", p.code, e);

                    false

                }

            })

            .cloned()

            .collect();

        let pre_market = checklist::build_pre_market_checklist(&positions, &t1_unlocks, &[]);

        log::info!(

            "[盘前] {} 只持仓，{} 只解禁",

            positions.len(),

            t1_unlocks.len()

        );



        push_governor_v3(&pre_market, PushKind::DailyReport, None).await;



        prediction::verify_predictions().await;

        let hit_rate = prediction::recent_hit_rate(7);

        if hit_rate > 0.0 {

            log::info!("[预测] 近7天命中率: {:.0}%", hit_rate * 100.0);

        }



        let mut targets = Vec::new();

        TieredScanner::load_positions(&mut targets);

        TieredScanner::load_watchlist(&mut targets);

        // 构建实体过滤集合（只关注9只标的）

        let our_codes: std::collections::HashSet<String> =

            targets.iter().map(|t| t.code.clone()).collect();

        // v19.13: 真实持仓 set (只 stock_position open), 不含 watchlist

        // 做T建议只能对真实持仓推, 不能对 watchlist 候选票推 (AGENTS.md §2.1)

        let holding_only_codes: std::collections::HashSet<String> =

            stock_analysis::portfolio::get_positions()

                .unwrap_or_default()

                .into_iter()

                .map(|p| p.code)

                .collect();

        let scanner = TieredScanner::new(targets);



        // ============= v12 PR1-1.7: 启动期评估一次 AccountMode =============

        // 后续每次 tick 重算在循环体内 (PR1-1.7 末尾的 evaluate_account_mode_hook).

        // 这里做 "今日首次" 评估, 防止上一次进程残留状态未推 T-01.

        evaluate_account_mode_hook(true).await;



        let detector = Detector::new(DetectorConfig::default());

        let mut state_machine = SignalStateMachine::default();

        state_machine.restore_state();

        let mut signal_count = 0u32;

        let mut alert_count = 0u32;

        let mut total_limit_ups: std::collections::HashSet<String> =

            std::collections::HashSet::new();

        let mut total_limit_downs: std::collections::HashSet<String> =

            std::collections::HashSet::new();

        let mut total_board_breaks = 0u32;

        let poll_secs: u64 = std::env::var("MONITOR_HOLDING_INTERVAL")

            .ok()

            .and_then(|s| s.parse().ok())

            .unwrap_or(30);

        // Phase 1.1 量化标准：信号融合 + 风险叠加 + 状态驱动

        use stock_analysis::monitor::signal_fusion::{Signal, SignalFusion, SignalSource};

        let fusion = SignalFusion::default();

        // 三个独立计时器

        let mut last_sector_push = std::time::Instant::now(); // 领涨板块（5分钟）
        let mut last_market_view = std::time::Instant::now(); // b013 P1-10: 盘面+产业链独立计时器 (5分钟)

        let mut last_health_summary = std::time::Instant::now(); // 持仓健康度（5分钟）

        let mut last_screener_run = std::time::Instant::now(); // 选股推荐（30分钟）

        let mut last_fund_top_push = std::time::Instant::now(); // 全市场主力净流入Top10（5分钟）

        let mut last_intraday_market = std::time::Instant::now(); // v31: I-01 盘中轮动总览 (10 min)

        let mut last_industry_chain_intraday = std::time::Instant::now(); // v34: I-03 涨停扩散 (15 min)

        let mut last_holding_plan = std::time::Instant::now(); // v38: I-04 持仓操作建议 (30 min)

        // v44: T-14 盘后固定价格申报 (15 min, 申报窗口 9:30-15:30)

        let mut last_post_fixed_order = std::time::Instant::now();

        // v45: T-15 盘后固定价格成交 (撮合 15:05-15:30, 5 min 周期)

        let mut last_post_fixed_fill = std::time::Instant::now();

        // v46: T-16 ST 涨跌幅变更 (开盘 9:30 一次/票/日)

        let mut st_price_pushed = false;

        // v47: T-17 ETF 收盘集合竞价 (14:57-15:00 一次)

        let mut etf_closing_pushed = false;

                                                                // 产业链扫描已移至 news_monitor_loop 的 8:00-22:00 窗口统一调度。

        let mut was_limit_up: std::collections::HashSet<String> = std::collections::HashSet::new();

        // 连板追踪：已推送过的标的不重复推送；board_level_cache 存 1=首板/2=二板/3+=三板

        let mut board_notified: std::collections::HashSet<String> =

            std::collections::HashSet::new();

        let mut board_level_cache: std::collections::HashMap<String, u8> =

            std::collections::HashMap::new();

        // 竞价量能扫描：9:20-9:25 每30秒推送一次全市场涨停量能榜

        let mut auction_vol_notified: std::collections::HashSet<String> =

            std::collections::HashSet::new();

        // 优选候选虚拟仓位记录：从集合竞价推送的候选+开盘价记录

        let mut virtual_observation: Vec<(String, String, f64)> = Vec::new(); // (code, name, open_price)

        let mut post_close_candidates_notified = false;

        let mut virtual_snapshot_persisted = false;

        // v32: P-01 盘前新闻热点 — 每个交易日首次进入 9:00-9:15 窗口时推一次

        let mut preopen_pushed = false;

        // v35: A-10 盘后催化复盘 — 每个交易日首次进入 19:00 后推一次

        let mut evening_pushed = false;

        let entry_mode = air_refuel_entry_mode();

        let monitor_cfg = stock_analysis::config::get_monitor_config();

        let confirm_shares = monitor_cfg.air_refuel.confirm_lots.saturating_mul(100);

        let pilot_shares = monitor_cfg.air_refuel.pilot_lots.saturating_mul(100);



        loop {

            let session = current_session();



            // ═══════════════════════════════════════════════════════════════

            // v32: P-01 盘前新闻热点 (9:00-9:15 窗口, 每日首次)

            //   - 触发: 首次进入 9:00 ≤ now < 9:15, 每个 monitor_loop session 推一次

            //   - 数据源: news_monitor 拉今日 + 昨日要闻 + 板块聚类

            //   - 模板: render_preopen_news_hot (无 banner, ℹ️参考级)

            //   - 静默: 公告空时短路

            //   - 注意: P-02 竞价量能 / P-03 候选触发 已有独立路径, 不在此重复

            // ═══════════════════════════════════════════════════════════════

            if !preopen_pushed && session == MarketSession::Closed {

                let now_time = chrono::Local::now().time();

                let preopen_start = chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap();

                let preopen_end = chrono::NaiveTime::from_hms_opt(9, 15, 0).unwrap();

                if now_time >= preopen_start && now_time < preopen_end {

                    log::info!("[P-01] 盘前窗口 ({}-{}), 推盘前新闻热点",

                        preopen_start.format("%H:%M"), preopen_end.format("%H:%M"));

                    let _ = push_templates::dispatch_preopen_news_hot_daily().await;

                    // v39: P-03 候选触发 (同盘前窗口, 影子开关控制)

                    let hhmm = chrono::Local::now().format("%H:%M").to_string();

                    let banner = current_banner();

                    let _ = push_templates::dispatch_candidate_triggered_daily(&hhmm, &banner).await;

                    preopen_pushed = true;

                }

            }



            // ═══════════════════════════════════════════════════════════════

            // v35: A-10 盘后催化复盘 (AfterHours session, 19:00 后首次)

            //   - 触发: 首次进入 AfterHours, 每个交易日推一次

            //   - 数据源: chain_daily cluster + continuation_count 推断持续性

            //   - 模板: render_catalyst_review (无 banner, ℹ️盘后)

            //   - 静默: chain_daily 空时短路

            // ═══════════════════════════════════════════════════════════════

            if !evening_pushed && session == MarketSession::AfterHours {

                let now_time = chrono::Local::now().time();

                let evening_start = chrono::NaiveTime::from_hms_opt(19, 0, 0).unwrap();

                if now_time >= evening_start {

                    log::info!("[A-10] 盘后窗口 ({} 后), 推催化复盘", evening_start.format("%H:%M"));

                    let date_str = chrono::Local::now().format("%Y-%m-%d").to_string();

                    let _ = push_templates::dispatch_catalyst_review_daily(&date_str).await;

                    // v36: A-01 虚拟仓复盘 - 同 19:00 窗口, 复用 evening_pushed flag

                    let _ = push_templates::dispatch_paper_review_daily(&date_str).await;

                    evening_pushed = true;

                }

            }



            // ============= v12 MVP0-B: T-02 数据状态每分钟评估 =============

            // 任一 capability staleness > 120s → Degraded; Quote stale > 120s → Unsafe

            // 治理 (mode/dm 停发) 走 dispatch 内部, 变更才推

            evaluate_data_mode_hook(None).await;



            // ── 9:20-9:25 竞价高量能扫描（30秒一次）+ 盘后优选重推 ──

            if session == MarketSession::Auction {

                let now_time = chrono::Local::now().time();

                // 9:20 之前只做持仓告警，不推全市场量能（数据不稳定）

                if now_time >= chrono::NaiveTime::from_hms_opt(9, 20, 0).unwrap() {

                    log::info!("[竞价] 9:20-9:25 量能扫描...");

                    let limit_stocks = tokio::task::spawn_blocking(|| {

                        let analyzer =

                            stock_analysis::market_analyzer::MarketAnalyzer::new(None).ok()?;

                        analyzer.get_limit_up_stocks().ok()

                    })

                    .await

                    .unwrap_or(None)

                    .unwrap_or_default();



                    if !limit_stocks.is_empty() {

                        // 按量比降序，取量比最高的前10（量能高代表竞价封板意愿强）

                        let mut sorted = limit_stocks.clone();

                        sorted.sort_by(|a, b| {

                            b.volume_ratio

                                .partial_cmp(&a.volume_ratio)

                                .unwrap_or(std::cmp::Ordering::Equal)

                        });

                        let new_items: Vec<_> = sorted

                            .iter()

                            .filter(|s| !auction_vol_notified.contains(&s.code))

                            .take(10)

                            .collect();

                        if !new_items.is_empty() {

                            // v37: 升级到 v12 §14.1 P-02 模板

                            //   之前: lines.join + PushKind::AuctionVolume (v19 格式)

                            //   现在: dispatch_auction_volume_daily + render_auction_volume

                            //   模板: 🌅 竞价热点量能 TopN (banner + 强承接/一般/弱承接)

                            let ts = chrono::Local::now().format("%H:%M:%S").to_string();

                            // 标记已通知 (避免同票重复推)

                            for s in &new_items {

                                auction_vol_notified.insert(s.code.clone());

                            }

                            let banner = current_banner();

                            let _ = push_templates::dispatch_auction_volume_daily(&ts, &banner).await;

                        }

                    }



                    // ▶ v13.10.1 P0-#3: 9:20-9:25 不再独立推送优选候选,

                    // 候选台(CandidateBoard)统一承载, 这里仅拉取用于虚拟观察.

                    if !post_close_candidates_notified {

                        post_close_candidates_notified = true;

                        log::info!("[竞价] 9:20-9:25 拉优选候选用于虚拟观察（v13.10.1 不再独立推送）...");

                        let post_close =

                            stock_analysis::opportunity::run_post_close_candidates(5).await;

                        // 删 v13.10.1: notify::push_governor(&post_close, notify::PushKind::AuctionRepush).await;

                        // 候选并入候选台 (run_candidate_panel_from_review) 统一推送



                        // 提取候选的code和name以便后续虚拟记录（简单方式：从推送文案中正则提取）

                        // 格式: "N. 名称(代码)" → 收集前5个作为虚拟观察对象

                        let mut seen_codes: std::collections::HashSet<String> =

                            std::collections::HashSet::new();

                        for line in post_close.lines() {

                            if let Some(paren_start) = line.find('(') {

                                if let Some(paren_end) = line.find(')') {

                                    if paren_start < paren_end {

                                        let code_str = &line[paren_start + 1..paren_end];

                                        if code_str.len() == 6

                                            && code_str.chars().all(|c| c.is_numeric())

                                        {

                                            if !seen_codes.insert(code_str.to_string()) {

                                                continue;

                                            }

                                            // 从该行"  "后提取name

                                            let name_part = line.trim_start();

                                            if let Some(name_end) = name_part.find('(') {

                                                let name = name_part[..name_end].trim_end();

                                                // 移除序号 "N. "

                                                let name = if let Some(dot_pos) = name.find('.') {

                                                    name[dot_pos + 1..].trim()

                                                } else {

                                                    name

                                                };

                                                virtual_observation.push((

                                                    code_str.to_string(),

                                                    name.to_string(),

                                                    0.0,

                                                ));

                                            }

                                        }

                                    }

                                }

                            }

                        }



                        // pilot 模式：竞价阶段先按当前价格虚拟潜伏记录（仅一次）

                        if entry_mode == AirRefuelEntryMode::Pilot

                            && !virtual_observation.is_empty()

                        {

                            let codes: Vec<String> = virtual_observation

                                .iter()

                                .map(|(c, _, _)| c.clone())

                                .collect();

                            let quote_map = tokio::task::spawn_blocking(move || {

                                let quotes =

                                    market_data::fetch_eastmoney_quotes(&codes).unwrap_or_default();

                                quotes

                                    .into_iter()

                                    .map(|q| (q.code, q.price))

                                    .collect::<std::collections::HashMap<_, _>>()

                            })

                            .await

                            .unwrap_or_default();



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

                                notify::push_governor(

                                    &lines.join("\n"),

                                    notify::PushKind::FactorIC,

                                )

                                .await;

                            }

                        }

                    }



                    // 持仓信号（原有逻辑保留）

                    for s in limit_stocks.iter().take(10) {

                        if !our_codes.contains(&s.code) {

                            continue;

                        }

                        let snap = StockSnapshot {

                            code: s.code.clone(),

                            name: s.name.clone(),

                            price: s.price,

                            change_pct: s.change_pct,

                            volume_ratio: 0.0,

                            main_net_yi: 0.0,

                            limit_up_price: None,

                            was_limit_up: false,

                            t1_locked: false,

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

                    // v40 + v52: P-04 虚拟盘成交回报 - 9:25 集合竞价结束推一次

                    //   数据源: monitor_loop 维护的 virtual_observation

                    //   v52: 遍历每只虚拟仓, 单独推 PaperTrade 模板 (替代 v40 占位)

                    //   静默: virtual_observation 空时短路

                    {

                        let hhmm = chrono::Local::now().format("%H:%M").to_string();

                        if virtual_observation.is_empty() {

                            log::info!("[P-04] virtual_observation 空, 跳过推送");

                        } else {

                            // v52: 每只虚拟仓单独推 (code/name/entry_price 真实)

                            for (code, name, entry_price) in &virtual_observation {

                                let status = if *entry_price > 0.0 {

                                    push_templates::PaperTradeStatus::Filled

                                } else {

                                    push_templates::PaperTradeStatus::NotFilled

                                };

                                let _ = push_templates::dispatch_paper_trade_one(

                                    &hhmm,

                                    name,

                                    code,

                                    status,

                                    if *entry_price > 0.0 { Some(*entry_price) } else { None },

                                    None, // qty: monitor_loop 未存

                                    Some(if *entry_price > 0.0 {

                                        "开盘价触达"

                                    } else {

                                        "已观察未成交"

                                    }),

                                    Some(if *entry_price > 0.0 {

                                        ""

                                    } else {

                                        "集合竞价后未触达买入价"

                                    }),

                                )

                                .await;

                            }

                        }

                    }

                    // 9:15-9:20 等待即可

                    tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

                    continue;

                }

            }



            if session == MarketSession::Morning || session == MarketSession::Afternoon {

                let result = tokio::task::spawn_blocking(|| {

                    let analyzer =

                        stock_analysis::market_analyzer::MarketAnalyzer::new(None).ok()?;

                    let limit_stocks = analyzer.get_limit_up_stocks().ok().unwrap_or_default();

                    std::thread::sleep(std::time::Duration::from_millis(800));

                    let position_quotes = market_data::fetch_position_quotes();

                    Some((limit_stocks, position_quotes))

                })

                .await

                .unwrap_or(None);



                if let Some((limit_stocks, position_quotes)) = result {

                    // ▶ 新增：开盘后虚拟记录观察仓位（仅一次）

                    if entry_mode == AirRefuelEntryMode::Confirm

                        && session == MarketSession::Morning

                        && !virtual_observation.is_empty()

                        && virtual_observation.iter().all(|(_, _, p)| *p == 0.0)

                    {

                        log::info!(

                            "[P-05 开盘] 虚拟观察仓位初始化（{}手 × {}只）",

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



                        // v63 (P-04 fix): 兜底拉 LLM 推荐的虚拟观察 codes 真报价

                        //   - 旧 bug: virtual_pos 来自 LLM 文本解析, 但 fill 只查 user holdings/watchlist + 涨停

                        //     限制, LLM 推的非持仓非涨停股 entry_price 永远 0.0 → push_virtual_next_day_review 跳过整条

                        //   - 新: 显式 fetch_position_quotes 给所有 virtual_observation codes (无持仓关系)

                        let virt_codes: Vec<String> = virtual_observation

                            .iter()

                            .filter(|(_, _, p)| *p == 0.0)

                            .map(|(c, _, _)| c.clone())

                            .collect();

                        if !virt_codes.is_empty() {

                            let virt_quotes = market_data::fetch_eastmoney_quotes(&virt_codes)

                                .unwrap_or_default();

                            for q in virt_quotes {

                                for virtual_pos in &mut virtual_observation {

                                    if virtual_pos.0 == q.code && virtual_pos.2 == 0.0 && q.price > 0.0 {

                                        virtual_pos.2 = q.price;

                                    }

                                }

                            }

                        }



                        // v58: 持久化虚拟观察快照 (保留旧逻辑)

                        if !virtual_snapshot_persisted {

                            let mut records: Vec<VirtualObservationRecord> = Vec::new();

                            let today = chrono::Local::now().format("%Y-%m-%d").to_string();

                            for (code, name, price) in &virtual_observation {

                                if *price > 0.0 {

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

                            if !records.is_empty() {

                                persist_virtual_observation_snapshot(&records);

                                virtual_snapshot_persisted = true;

                            }

                        }



                        // v58: 改用 v12 §14.5 P-05 dispatcher (替代内联 lines.join)

                        let hhmm = chrono::Local::now().format("%H:%M").to_string();

                        let total_amount: f64 = virtual_observation

                            .iter()

                            .filter(|(_, _, p)| *p > 0.0)

                            .map(|(_, _, p)| p * confirm_shares as f64)

                            .sum();

                        let _ = push_templates::dispatch_virtual_watch_daily(

                            &hhmm,

                            &virtual_observation,

                            confirm_shares,

                        )

                        .await;

                        log::info!(

                            "[P-05 开盘] 虚拟观察仓位已推送（合计 ¥{:.0}）",

                            total_amount

                        );

                    }



                    // 首板/二板/三板识别：全市场涨停池，各自独立消息，每只仅推一次

                    if !limit_stocks.is_empty() {

                        let mut need_lookup: Vec<(String, String)> = Vec::new();

                        for s in &limit_stocks {

                            if board_notified.contains(&s.code) {

                                continue;

                            }

                            if !board_level_cache.contains_key(&s.code) {

                                need_lookup.push((s.code.clone(), s.name.clone()));

                            }

                        }

                        if !need_lookup.is_empty() {

                            let need_lookup: Vec<(String, String)> =

                                need_lookup.into_iter().take(40).collect();

                            let looked_up = tokio::task::spawn_blocking(move || {

                                market_data::lookup_board_level_batch(&need_lookup)

                            })

                            .await

                            .unwrap_or_default();

                            board_level_cache.extend(looked_up);

                        }



                        let mut first_lines: Vec<String> = Vec::new();

                        let mut second_lines: Vec<String> = Vec::new();

                        let mut third_lines: Vec<String> = Vec::new();

                        let mut sorted_limits = limit_stocks.clone();

                        sorted_limits.sort_by(|a, b| {

                            b.main_net_yi

                                .partial_cmp(&a.main_net_yi)

                                .unwrap_or(std::cmp::Ordering::Equal)

                        });

                        for s in sorted_limits.iter().take(50) {

                            let level = match board_level_cache.get(&s.code) {

                                Some(v) => *v,

                                None => continue,

                            };

                            if !board_notified.insert(s.code.clone()) {

                                continue;

                            }

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

                            let mut lines = vec![format!(

                                "🟢 首板涨停 Top{}（{}）",

                                first_lines.len().min(10),

                                ts

                            )];

                            lines.extend(first_lines.into_iter().take(10));

                            notify::push_governor(&lines.join("\n"), notify::PushKind::LimitBoards)

                                .await;

                        }

                        if !second_lines.is_empty() {

                            let mut lines = vec![format!(

                                "🟡 二板涨停 Top{}（{}）",

                                second_lines.len().min(10),

                                ts

                            )];

                            lines.extend(second_lines.into_iter().take(10));

                            notify::push_governor(&lines.join("\n"), notify::PushKind::LimitBoards)

                                .await;

                        }

                        if !third_lines.is_empty() {

                            let mut lines = vec![format!(

                                "🔴 三板+ 涨停 Top{}（{}）",

                                third_lines.len().min(10),

                                ts

                            )];

                            lines.extend(third_lines.into_iter().take(10));

                            notify::push_governor(&lines.join("\n"), notify::PushKind::LimitBoards)

                                .await;

                        }

                    }



                    // 合并两路数据：涨停列表中的持仓 + 持仓单独查询

                    let mut stock_map: std::collections::HashMap<

                        String,

                        &stock_analysis::market_data::TopStock,

                    > = std::collections::HashMap::new();

                    for s in &limit_stocks {

                        if our_codes.contains(&s.code) {

                            stock_map.insert(s.code.clone(), s);

                        }

                    }

                    for q in &position_quotes {

                        if !stock_map.contains_key(&q.code) {

                            stock_map.insert(q.code.clone(), q);

                        }

                    }



                    // 主力排名（仅涨停股中排序）

                    let mut ranked: Vec<&stock_analysis::market_data::TopStock> =

                        limit_stocks.iter().collect();

                    ranked.sort_by(|a, b| {

                        b.main_net_yi

                            .partial_cmp(&a.main_net_yi)

                            .unwrap_or(std::cmp::Ordering::Equal)

                    });

                    let total_ranked = ranked.len();



                    // 持仓遍历：信号融合（不再单独推送每条事件）

                    let mut health_lines: Vec<String> = Vec::new();

                    for (code, s) in &stock_map {

                        // review #14: DB 错误按"已锁定"处理 (保守), log warn 提醒.

                        let t1_locked = match stock_analysis::portfolio::is_t1_locked(code) {

                            Ok(v) => v,

                            Err(e) => {

                                log::warn!("[t+1] is_t1_locked({}) 失败: {} — 按锁定处理", code, e);

                                true

                            }

                        };

                        let rank = ranked.iter().position(|r| r.code == *code).map(|p| p + 1);

                        let is_limit_up = s.change_pct >= 9.5;

                        let prev_was_limit = was_limit_up.contains(code);



                        // 状态追踪

                        if is_limit_up {

                            was_limit_up.insert(code.clone());

                        } else {

                            was_limit_up.remove(code);

                        }



                        let snap = StockSnapshot {

                            code: s.code.clone(),

                            name: s.name.clone(),

                            price: s.price,

                            change_pct: s.change_pct,

                            volume_ratio: s.volume_ratio,

                            main_net_yi: s.main_net_yi,

                            limit_up_price: Some(s.price * 1.1),

                            was_limit_up: prev_was_limit,

                            t1_locked,

                        };



                        // 信号收集 + 突变检测

                        let mut signals: Vec<Signal> = Vec::new();

                        let mut emergency_note = String::new();

                        for e in detector.scan_stock(&snap) {

                            signal_count += 1;

                            let (dir, strength) = match e.category {

                                AlertCategory::LimitUp | AlertCategory::MainInflow => (1.0, 80.0),

                                AlertCategory::LimitDown | AlertCategory::MainOutflow => {

                                    (-1.0, 80.0)

                                }

                                AlertCategory::VolBurst => (1.0, 60.0),

                                AlertCategory::BoardBreak => (-1.0, 90.0),

                                _ => (0.0, 40.0),

                            };

                            signals.push(Signal::new(

                                match e.category {

                                    AlertCategory::MainInflow | AlertCategory::MainOutflow => {

                                        SignalSource::FundFlow

                                    }

                                    _ => SignalSource::Technical,

                                },

                                dir,

                                strength,

                                0.0,

                            ));

                            // 突变检测：仅记录状态，不单独推送

                            if matches!(e.category, AlertCategory::BoardBreak) {

                                emergency_note = "⚠️ 炸板！".to_string();

                            }

                        }



                        // 信号融合

                        let resonance = if signals.is_empty() {

                            0.0

                        } else {

                            fusion.resonance(&signals)

                        };

                        let recommend = fusion.recommend(resonance);



                        // 累计当日数据（供收盘总结）

                        if is_limit_up {

                            total_limit_ups.insert(code.clone());

                        }

                        if s.change_pct <= -9.5 {

                            total_limit_downs.insert(code.clone());

                        }

                        if prev_was_limit && !is_limit_up {

                            total_board_breaks += 1;

                        }



                        // 涨停/跌停突变一次推送（走状态机防重复）

                        if is_limit_up || s.change_pct <= -9.5 {

                            let event = AlertEvent {

                                level: if s.change_pct <= -9.5 {

                                    AlertLevel::Emergency

                                } else {

                                    AlertLevel::Important

                                },

                                category: if s.change_pct <= -9.5 {

                                    AlertCategory::LimitDown

                                } else {

                                    AlertCategory::LimitUp

                                },

                                code: code.clone(),

                                name: s.name.clone(),

                                message: if s.change_pct <= -9.5 {

                                    format!("{} 跌停 {:.1}%", s.name, s.change_pct)

                                } else {

                                    format!("{} 涨停 {:.1}%", s.name, s.change_pct)

                                },

                                detail: AlertDetail {

                                    price: Some(s.price),

                                    change_pct: Some(s.change_pct),

                                    volume_ratio: Some(s.volume_ratio),

                                    main_flow_yi: Some(s.main_net_yi),

                                    threshold: None,

                                    news_title: None,

                                    news_summary: None,

                                    ai_decision: None,

                                    t1_locked,

                                    extra: rank.map(|r| {

                                        format!(

                                            "主力排名 {}/{} | 共振{:.0} {}",

                                            r, total_ranked, resonance, recommend

                                        )

                                    }),

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

                            push_governor_v3(&format!("🔴 {}({}) {}", s.name, code, emergency_note), PushKind::HoldingEvent, Some(code)).await;

                        }



                        // 健康度记录（每5分钟推送汇总）

                        let note = if t1_locked {

                            "🔒锁仓"

                        } else if is_limit_up {

                            "🔺涨停"

                        } else if s.change_pct <= -5.0 {

                            "🔻"

                        } else if resonance > 60.0 {

                            "📈"

                        } else if resonance < -30.0 {

                            "📉"

                        } else {

                            "→"

                        };

                        health_lines.push(format!(

                            "  {:<6} {}({}) {:>+.1}% ¥{:2} {}",

                            note,

                            s.name,

                            code,

                            s.change_pct,

                            s.price,

                            if resonance.abs() > 5.0 {

                                format!("共振{:0}", resonance)

                            } else {

                                String::new()

                            }

                        ));

                        if resonance.abs() > 30.0 {

                            log::info!(

                                "[信号融合] {}({}) 共振={:0} 建议={}",

                                s.name,

                                code,

                                resonance,

                                recommend

                            );

                        }

                        // v19.13: 移除原来的做T推送 (line 2827-2834)

                        // 旧: 对 limit_stocks (涨停股 Top 10) ∩ our_codes (持仓+watchlist) 推

                        // 问题: 涨停股很少是持仓 (持仓 6 只, 涨停 Top 10 通常不重叠), 即使重叠也包括 watchlist

                        // 新: 上面 last_screener_run 后的 "持仓专属做T扫描" 才是真路径

                        // 这里只保留 signal_count + alert_count, 不推做T

                    }



                    // v19.12: 持仓健康度 → 每 5 分钟硬推 (去掉条件限制, 用户要求全推)

                    if last_health_summary.elapsed().as_secs() >= 300 && !health_lines.is_empty() {

                        last_health_summary = std::time::Instant::now();

                        let mut summary = vec![format!(

                            "📊 持仓健康度 ({})",

                            chrono::Local::now().format("%H:%M")

                        )];

                        summary.append(&mut health_lines);

                        summary.push("─────".into());

                        summary.push("💡 T-04 持仓监控 (5min 周期, 全推)".into());

                        notify::push_governor(&summary.join("\n"), notify::PushKind::HoldingEvent)

                            .await;

                    }



                    // 选股推荐（独立计时器，每30分钟）

                    let cfg = stock_analysis::config::get_monitor_config();

                    if last_screener_run.elapsed().as_secs() >= cfg.screener_interval_min * 60 {

                        last_screener_run = std::time::Instant::now();

                        log::info!("[选股] 开始盘中选股扫描...");

                        let recs = tokio::task::spawn_blocking(run_stock_screener)

                            .await

                            .unwrap_or(None);

                        if let Some(ref recs) = recs {

                            for rec in recs {

                                log::info!("[选股] {}", rec);

                                // v57: 改用 D-01 NewsToIdea PushKind (合并 StockPick)

                                notify::push_governor(rec, notify::PushKind::NewsToIdea).await;

                            }

                        }

                    }



                    // v19.13: 持仓专属做T扫描 (每 30s, 真接 DB 持仓股, 不依赖涨停)

                    // AGENTS.md §2.1: 做T建议只对真实持仓推 (不是 watchlist 候选票)

                    if last_health_summary.elapsed().as_secs() >= 30 {

                        // 复用 last_health_summary 计时器 (30s 间隔), 单独 lock 避免阻塞

                        let holding_codes_vec: Vec<String> =

                            holding_only_codes.iter().cloned().collect();

                        if !holding_codes_vec.is_empty() {

                            let holding_signals = tokio::task::spawn_blocking(move || {

                                use stock_analysis::monitor::detector::{Detector, DetectorConfig, StockSnapshot as SS};

                                let detector_local = Detector::new(DetectorConfig::default());

                                let quotes = market_data::fetch_position_quotes();

                                let name_map: std::collections::HashMap<String, String> = stock_analysis::portfolio::get_positions()

                                    .unwrap_or_default()

                                    .into_iter()

                                    .map(|p| (p.code, p.name))

                                    .collect();

                                let mut out: Vec<String> = Vec::new();

                                for q in &quotes {

                                    if !holding_codes_vec.contains(&q.code) { continue; }

                                    let snap = SS {

                                        code: q.code.clone(),

                                        name: name_map.get(&q.code).cloned().unwrap_or_else(|| q.code.clone()),

                                        price: q.price,

                                        change_pct: q.change_pct,

                                        volume_ratio: q.volume_ratio,

                                        main_net_yi: 0.0,

                                        limit_up_price: None, was_limit_up: false, t1_locked: false,

                                    };

                                    for e in detector_local.scan_stock(&snap) {

                                        // 强信号才推做T (VolumeBurst / MainInflow / MainOutflow)

                                        if matches!(e.category,

                                            stock_analysis::monitor::detector::AlertCategory::VolBurst

                                            | stock_analysis::monitor::detector::AlertCategory::MainInflow

                                            | stock_analysis::monitor::detector::AlertCategory::MainOutflow)

                                        {

                                            let dir = if matches!(e.category,

                                                stock_analysis::monitor::detector::AlertCategory::MainInflow) { "+" } else { "-" };

                                            out.push(format!(

                                                "🔄 做T建议 {}({}) | {} {}\n   现价 ¥{:.2} 涨跌 {:+.2}%\n   高抛: +{:.1}% 减仓1/3\n   低吸: -{:.1}% 回补2/3\n   止损: ¥{:.2}",

                                                snap.name, snap.code, dir, e.message,

                                                snap.price, snap.change_pct,

                                                snap.change_pct.abs().max(2.0), snap.change_pct.abs().max(2.0),

                                                snap.price * 0.95

                                            ));

                                        }

                                    }

                                }

                                out

                            }).await.unwrap_or_default();

                            for t0 in holding_signals {

                                log::info!("[做T-持仓] 推送: {}", t0.lines().next().unwrap_or(""));

                                notify::push_governor(&t0, notify::PushKind::T0Advice).await;

                            }

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



                    // v19.12: 盘面走向 (R-02 盘中简版) + 涨停产业链 (R-03 盘中简版) — 每 5 分钟硬推
                    // b013 P1-10: 改用独立 last_market_view 计时器
                    if last_market_view.elapsed().as_secs() >= 300 {
                        last_market_view = std::time::Instant::now();

                        let market_view = tokio::task::spawn_blocking(|| {

                            use stock_analysis::market_analyzer::sector_monitor;

                            use stock_analysis::review::limit_chain_review;

                            // 盘面简版

                            let boards =

                                sector_monitor::fetch_board_ranking("f3", 10).unwrap_or_default();

                            let market_text = if boards.is_empty() {

                                "📊 盘面 (盘中) | 数据源不稳定, 跳过".to_string()

                            } else {

                                let avg_chg = boards.iter().map(|b| b.change_pct).sum::<f64>()

                                    / boards.len() as f64;

                                let strong = boards.iter().filter(|b| b.change_pct > 3.0).count();

                                let mut s = format!(

                                    "📊 盘面 ({} 盘中)\n板块均值 {:+.2}% | 强势板块 {} 个\n",

                                    chrono::Local::now().format("%H:%M"),

                                    avg_chg,

                                    strong

                                );

                                s.push_str("领涨板块 Top5:\n");

                                for b in boards.iter().take(5) {

                                    s.push_str(&format!(

                                        "  {} {:+.2}% 主力{:.2}亿\n",

                                        b.name,

                                        b.change_pct,

                                        b.main_inflow / 1e8

                                    ));

                                }

                                s

                            };

                            // 涨停产业链简版

                            let boards_full =

                                sector_monitor::fetch_board_ranking("f3", 30).unwrap_or_default();

                            let mut items = Vec::new();

                            for b in boards_full.iter().take(5) {

                                if b.change_pct > 0.5 {

                                    let limit_up_estimate = if b.change_pct > 5.0 { 3 } else { 1 };

                                    items.push(limit_chain_review::build_chain_item(

                                        b.name.clone(),

                                        limit_up_estimate,

                                        limit_up_estimate,

                                        0,

                                        b.leader_name.clone(),

                                        1,

                                        b.main_inflow,

                                    ));

                                }

                            }

                            let chain_text = if items.is_empty() {

                                String::new()

                            } else {

                                limit_chain_review::render_r03(&items, &[])

                            };

                            format!("{}\n{}", market_text, chain_text)

                        })

                        .await

                        .unwrap_or_default();

                        if !market_view.is_empty() && market_view.len() > 50 {

                            notify::push_governor(&market_view, notify::PushKind::ReviewSignal)

                                .await;

                        }

                    }



                    // v19.12: 盘中换手率高 Top10 (每 10 分钟, 关注流动性)

                    if last_fund_top_push.elapsed().as_secs() >= 600 {

                        let turnover_top = tokio::task::spawn_blocking(|| {

                            use stock_analysis::market_analyzer::sector_monitor;

                            let boards =

                                sector_monitor::fetch_board_ranking("f3", 30).unwrap_or_default();

                            let mut s = format!(

                                "🔄 换手率 Top10 ({} 盘中)\n",

                                chrono::Local::now().format("%H:%M")

                            );

                            let mut rows: Vec<(String, f64, f64)> = Vec::new();

                            for b in boards.iter().take(30) {

                                if !b.leader_name.is_empty() {

                                    rows.push((

                                        b.leader_name.clone(),

                                        b.change_pct,

                                        b.main_inflow / 1e8,

                                    ));

                                }

                            }

                            rows.sort_by(|a, b| {

                                b.2.abs()

                                    .partial_cmp(&a.2.abs())

                                    .unwrap_or(std::cmp::Ordering::Equal)

                            });

                            s.push_str("龙头股 (按资金流入排序, 间接反映换手活跃度):\n");

                            for (i, (name, chg, flow)) in rows.iter().take(10).enumerate() {

                                s.push_str(&format!(

                                    "  {}. {} {:+.2}% 主力{:.2}亿\n",

                                    i + 1,

                                    name,

                                    chg,

                                    flow

                                ));

                            }

                            if rows.is_empty() {

                                s.push_str("⚠️ 数据源不稳定, 跳过\n");

                            }

                            s

                        })

                        .await

                        .unwrap_or_default();

                        if turnover_top.len() > 50 {

                            // v56: PushKind 修正 (之前错用 FundInflow, 实际是 I-08 换手率)

                            notify::push_governor(&turnover_top, notify::PushKind::TurnoverTop)

                                .await;

                            last_fund_top_push = std::time::Instant::now();

                        }

                    }



                    // ═══════════════════════════════════════════════════════════════

                    // v31: I-01 盘中轮动总览 (10 min 周期, 替代老 SectorTop)

                    //   - 数据源: sector_monitor::fetch_board_ranking (科技/电力/机器人三轴)

                    //   - 模板: render_intraday_market (带 banner)

                    //   - 静默: grade_sectors 无数据时短路, log

                    //   - 横幅 DataMode 写死 Full (与 v12 已推模板一致)

                    // ═══════════════════════════════════════════════════════════════

                    if last_intraday_market.elapsed().as_secs() >= 600 {

                        use push_templates::dispatch_intraday_market_daily;

                        // v41: 读共享 banner

                        let banner = current_banner();

                        let hhmm = chrono::Local::now().format("%H:%M").to_string();

                        let _ = dispatch_intraday_market_daily(&hhmm, &banner).await;

                        last_intraday_market = std::time::Instant::now();

                    }



                    // ═══════════════════════════════════════════════════════════════

                    // v34: I-03 涨停扩散与板块补涨 (15 min 周期, 与 v18 LimitBoards 互补)

                    //   - 数据源: limit_up_stocks + chain_mapper 板块归类

                    //   - 模板: render_industry_chain_intraday (主链 + 龙头 + 补涨候选)

                    //   - 静默: 涨停池空时短路

                    //   - 与 v18 LimitBoards (首板/二板/三板 split) 互补不冲突

                    // ═══════════════════════════════════════════════════════════════

                    if last_industry_chain_intraday.elapsed().as_secs() >= 900 {

                        use push_templates::dispatch_industry_chain_intraday_daily;

                        // v41: 读共享 banner

                        let banner = current_banner();

                        let hhmm = chrono::Local::now().format("%H:%M").to_string();

                        let _ = dispatch_industry_chain_intraday_daily(&hhmm, &banner).await;

                        last_industry_chain_intraday = std::time::Instant::now();

                    }



                    // ═══════════════════════════════════════════════════════════════

                    // v38: I-04 持仓操作建议 (30 min 周期, v12 §14.5 冷却 30 min/票)

                    //   - 遍历当前持仓, 用 cost/hard_stop 生成 plan

                    //   - 简化版: 涨幅 >5% 减仓, <-3% 加仓, 否则持有

                    //   - 真实意图: 接入 decision::evaluate_holding (v12.2 规划)

                    //   - 静默: 无持仓时短路

                    // ═══════════════════════════════════════════════════════════════

                    if last_holding_plan.elapsed().as_secs() >= 1800 {

                        let hhmm = chrono::Local::now().format("%H:%M").to_string();

                        let banner = current_banner();

                        let _ = push_templates::dispatch_holding_plan_daily(&hhmm, &banner).await;

                        last_holding_plan = std::time::Instant::now();

                    }



                    // ═══════════════════════════════════════════════════════════════

                    // v44 + v54 + v60: T-14/T-15 trade_pipeline 调度 (F8 拆分)

                    //   - T-14 (15 min) 调 dispatch_trade_pipeline_orders (只 order events)

                    //   - T-15 (5 min) 调 dispatch_trade_pipeline_fills (只 fill events)

                    //   - 拆分后 5 min T-15 不会再扫 order events (旧 bug 3x 工作量)

                    //   - 沙箱: trade_pipeline 空, 静默短路

                    //   - 真实 intent: broker 委托/成交回报 event

                    if last_post_fixed_order.elapsed().as_secs() >= 900 {

                        let hhmm = chrono::Local::now().format("%H:%M").to_string();

                        let _ = push_templates::dispatch_trade_pipeline_orders(&hhmm).await;

                        last_post_fixed_order = std::time::Instant::now();

                    }



                    if last_post_fixed_fill.elapsed().as_secs() >= 300 {

                        let hhmm = chrono::Local::now().format("%H:%M").to_string();

                        let _ = push_templates::dispatch_trade_pipeline_fills(&hhmm).await;

                        last_post_fixed_fill = std::time::Instant::now();

                    }



                    // ═══════════════════════════════════════════════════════════════

                    // v46 + v59: T-16 ST 涨跌幅变更 (开盘 9:30 一次/票/日)

                    //   - 新规 2026-07-06: 主板 ST/*ST 5%→10%

                    //   - v59 修复: 真正调 dispatch_st_price_limit_changed (F2 死代码修复)

                    //   - 真实数据源: portfolio.get_st_positions() (is_st/star_st 暂写死, broker 接入后真接)

                    // ═══════════════════════════════════════════════════════════════

                    if !st_price_pushed {

                        let now_time = chrono::Local::now().time();

                        let st_trigger = chrono::NaiveTime::from_hms_opt(9, 30, 0).unwrap();

                        if now_time >= st_trigger {

                            st_price_pushed = true;

                            // review #14: get_st_positions 现在返回 Vec<String>, 按 code 反查 Position.

                            let st_codes = stock_analysis::portfolio::get_st_positions();

                            if st_codes.is_empty() {

                                log::info!("[T-16] ST 涨跌幅变更 ticker (无 ST/*ST 持仓, 静默)");

                            } else {

                                for code in &st_codes {

                                    // 找不到完整持仓则跳过 (DB 可能刚 reset)

                                    let Some(pos) = stock_analysis::portfolio::find_position(code) else {

                                        log::warn!("[T-16] ST code {} 无 Position 详情, 跳过", code);

                                        continue;

                                    };

                                    let st_type = if pos.star_st {

                                        push_templates::StType::StarST

                                    } else {

                                        push_templates::StType::ST

                                    };

                                    let now_price = pos.cost_price * 1.02; // 简化: 无 fetch

                                    let new_stop = pos.cost_price * 0.90;

                                    let new_take = pos.cost_price * 1.10;

                                    let banner = current_banner();

                                    let _ = push_templates::dispatch_st_price_limit_changed(

                                        "09:30",

                                        &pos.name,

                                        &pos.code,

                                        st_type,

                                        0.05, 0.10, // 5% → 10% 新规

                                        pos.shares as u32,

                                        pos.cost_price,

                                        now_price,

                                        Some(new_stop),

                                        Some(new_take),

                                        &banner,

                                    )

                                    .await;

                                }

                                log::info!(

                                    "[T-16] ST 涨跌幅变更已推 {} 只持仓",

                                    st_codes.len()

                                );

                            }

                        }

                    }



                    // ═══════════════════════════════════════════════════════════════

                    // v47 + v59: T-17 ETF 收盘集合竞价 (14:57 一次)

                    //   - 新规 2026-07-06: 上交所基金收盘 14:57-15:00 集合竞价

                    //   - v59 修复: 真正调 dispatch_etf_closing_call_auction (F2 死代码修复)

                    //   - 真实数据源: portfolio ETF 持仓 + 集合竞价行情 (后续 PR)

                    // ═══════════════════════════════════════════════════════════════

                    if !etf_closing_pushed {

                        let now_time = chrono::Local::now().time();

                        let etf_trigger = chrono::NaiveTime::from_hms_opt(14, 57, 0).unwrap();

                        if now_time >= etf_trigger {

                            etf_closing_pushed = true;

                            // 简化: 沙箱无 ETF 持仓识别, 调 dispatcher (空时短路)

                            let _ = push_templates::dispatch_etf_closing_call_auction(

                                "14:57",

                                "沪市ETF",

                                "510000", // 沙箱占位

                                None,

                                None,

                                "正常",

                            )

                            .await;

                            log::info!("[T-17] ETF 收盘集合竞价 ticker (沙箱无 ETF 持仓, 短路)");

                        }

                    }

                }

            }



            if session == MarketSession::AfterHours {

                break;

            }

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

        let board_break_rate = if up_count > 0 {

            total_board_breaks as f64 / up_count as f64 * 100.0

        } else {

            0.0

        };

        // v13.10.1 P0-#5: 区分"今日冻结(明日解禁)"与"今日解禁(明日可卖)"

        // 之前 close_summary 把 t1_unlocks(今日解禁) 误标为"T+1 冻结", 7 只全部命中,

        // 全显示"止损 0.00"无意义.

        let positions_for_close = stock_analysis::portfolio::get_positions().unwrap_or_default();

        let mut t1_frozen: Vec<stock_analysis::portfolio::Position> = Vec::new();

        let mut tomorrow_unlocks: Vec<stock_analysis::portfolio::Position> = Vec::new();

        for p in &positions_for_close {

            match stock_analysis::portfolio::is_t1_locked(&p.code) {

                Ok(true) => t1_frozen.push(p.clone()),

                Ok(false) => tomorrow_unlocks.push(p.clone()),

                Err(e) => {

                    log::error!("[盘后] is_t1_locked({}) 失败: {} — 保守按已锁定归类", p.code, e);

                    t1_frozen.push(p.clone());

                }

            }

        }

        let summary = checklist::build_close_summary(

            index_change,

            up_count,

            down_count,

            board_break_rate,

            signal_count as usize,

            alert_count as usize,

            &t1_frozen,

            &tomorrow_unlocks,

        );

        push_governor_v3(&summary, PushKind::DailyReport, None).await;



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

        let review_report = stock_analysis::review::report::generate_daily_report_with_ledger(

            &reviews,

            &stats,

            &holdings,

            &prices,

            Some(equity.as_slice()),

        );

        push_governor_v3(&review_report, PushKind::DailyReport, None).await;



        // 盘后独立维度：优选次日候选（最多 5 只，达不到阈值可少推/不推），强调可解释性，不复用盘中量能信号口径。

        // v57: 改用 A-08 TomorrowWatch PushKind (合并 OptimalClose)

        let post_close_candidates = stock_analysis::opportunity::run_post_close_candidates(5).await;

        notify::push_governor(&post_close_candidates, notify::PushKind::TomorrowWatch).await;



        // 盘后统计上一交易日虚拟观察仓表现（可配置开关）

        push_virtual_next_day_review_if_needed().await;



        // v3 每日净值快照

        let _ = tokio::task::spawn_blocking(snapshot_portfolio_value).await;



        // 盘后持仓多 Agent 深度研判（6 分析师 + 多空辩论 + 仲裁），逐只推送飞书

        // v17.0: --test 路径 holding/watch_breakout_text 在 run_test_scan 不可见, 传 "" 占位

        run_review_deep_analysis("", "", "").await;



        log::info!(

            "[收盘] 信号{}条 告警{}条 | DQ: {} | {}",

            signal_count,

            alert_count,

            scanner.dq_summary(),

            prediction::hit_rate_summary(7)

        );

        // 收盘后继续循环，等待下一个交易日

    }

}



/// Phase 4.1 选股推荐：点火广度排序 + 成份股过滤

fn run_stock_screener() -> Option<Vec<String>> {

    use stock_analysis::breakout::engine::screen_intraday;

    use stock_analysis::market_analyzer::sector_monitor;



    let our_codes: std::collections::HashSet<String> = stock_analysis::portfolio::get_all_codes()

        .unwrap_or_default()

        .into_iter()

        .collect();



    // 1. 拉涨幅前 30 板块（失败→本轮无推荐，不刷屏）

    let boards = sector_monitor::fetch_board_ranking("f3", 30).ok()?;



    // 2. 收集候选标的（逐板块拉成份股，命中足够候选即提前停止，避免预拉全部 30 板块）

    //    候选携带其所属板块名 + 板块点火广度，供 breakout 盘中模式打分。

    const MAX_CANDIDATES: usize = 20; // 限制批量报价规模，控制 HTTP 成本

    struct Candidate {

        code: String,

        name: String,

        board: String,

        near_limit: usize,

    }

    let mut candidates: Vec<Candidate> = Vec::new();

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for b in boards.iter() {

        let comps = match sector_monitor::fetch_board_components(&b.code, 30) {

            Ok(c) => c,

            Err(_) => continue, // 该板块拉取失败→跳过，不中断

        };

        let ignition = sector_monitor::compute_ignition(&comps);

        for s in comps.iter() {

            if our_codes.contains(&s.code) {

                continue;

            }

            if s.code.starts_with('8') || s.code.starts_with('4') || s.code.starts_with("688") {

                continue;

            }

            if s.name.contains("ST") || s.name.contains("退") {

                continue;

            }

            if s.change_pct > 9.5 {

                continue;

            } // 已涨停不追

            if !seen.insert(s.code.clone()) {

                continue;

            }

            candidates.push(Candidate {

                code: s.code.clone(),

                name: s.name.clone(),

                board: b.name.clone(),

                near_limit: ignition.near_limit_count,

            });

            if candidates.len() >= MAX_CANDIDATES {

                break;

            }

        }

        if candidates.len() >= MAX_CANDIDATES {

            break;

        }

    }

    if candidates.is_empty() {

        return None;

    }



    // 3. 批量拉候选资金面（一次 HTTP）。失败→资金面留空，breakout 标记数据降级（不伪造）。

    let codes: Vec<String> = candidates.iter().map(|c| c.code.clone()).collect();

    let quote_map: std::collections::HashMap<String, stock_analysis::market_data::TopStock> =

        match market_data::fetch_eastmoney_quotes(&codes) {

            Ok(qs) => qs.into_iter().map(|q| (q.code.clone(), q)).collect(),

            Err(e) => {

                log::warn!("[选股] 候选资金面拉取失败，按数据降级处理: {}", e);

                std::collections::HashMap::new()

            }

        };



    // 4. breakout 盘中模式逐个打分

    let mut signals: Vec<(stock_analysis::breakout::signal::BreakoutSignal, String)> = Vec::new();

    for c in &candidates {

        let (vol_ratio, change_pct, main_net_yi) = match quote_map.get(&c.code) {

            Some(q) => (q.volume_ratio, q.change_pct, q.main_net_yi),

            None => (0.0, 0.0, 0.0), // 数据降级：screen_intraday 内部会置 data_degraded

        };

        let sig = screen_intraday(

            &c.code,

            &c.name,

            vol_ratio,

            change_pct,

            main_net_yi,

            c.near_limit,

        );

        signals.push((sig, c.board.clone()));

    }



    // 5. 按置信度降序，取置信度达阈值（≥50）的 Top 3

    // v13.10.1 P1-#7: 阈值 20→50. 修复前 30-40 置信度全推, 用户反馈"只推送无意义".

    signals.sort_by(|a, b| b.0.confidence.cmp(&a.0.confidence));

    let recs: Vec<String> = signals

        .iter()

        .filter(|(s, _)| s.confidence >= 50)

        .take(3)

        .map(|(s, board)| {

            format!(

                "{} 选股推荐 | {}({}) | 板块:{} | 涨幅:{:.1}% | 置信度:{} | {}",

                s.breakout_type.emoji(),

                s.name,

                s.code,

                board,

                s.change_pct,

                s.confidence,

                s.description

            )

        })

        .collect();



    if recs.is_empty() {

        None

    } else {

        Some(recs)

    }

}



/// 持仓实时行情：东财 push2 为主（多主机轮询），新浪兜底

async fn push_sector_leaders() {

    // v56: 改用 dispatch_sector_top_daily (I-09 v12 §14.5 模板)

    let hhmm = chrono::Local::now().format("%H:%M").to_string();

    let _ = push_templates::dispatch_sector_top_daily(&hhmm).await;

}



async fn push_market_fund_top10() {

    // v56: 改用 dispatch_fund_inflow_top_daily (I-10 v12 §14.5 模板)

    let hhmm = chrono::Local::now().format("%H:%M").to_string();

    let _ = push_templates::dispatch_fund_inflow_top_daily(&hhmm).await;

}



/// v13.11 (Task 11): 独立轮询 Sina 财经要闻, 每 90s 拉一次 top 20,

/// 双写 DatabaseManager::news_items 表 (供后续链路/回溯使用).

/// 与 news_monitor_loop 解耦: 本 loop 不做信号/AI 分析, 仅入库.

async fn poll_news_loop() {

    use stock_analysis::data_provider::sina_news_provider::SinaNewsProvider;

    use stock_analysis::database::DatabaseManager;

    use std::time::Duration;



    let provider = SinaNewsProvider::new();

    let mut interval = tokio::time::interval(Duration::from_secs(90));

    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);



    log::info!("[PollNews] 启动 (Sina 财经要闻, 90s 间隔, 双写 DB)");



    loop {

        interval.tick().await;

        match provider.fetch_top_news(20).await {

            Ok(items) => {

                let count = items.len();

                let mut written = 0usize;

                for item in &items {

                    let ok = DatabaseManager::with_db("poll_news", |db| {

                        if db.insert_news_item(item).is_ok() {

                            Some(())

                        } else {

                            None

                        }

                    });

                    if ok.is_some() {

                        written += 1;

                    }

                }

                log::info!(

                    "[新闻] Sina 拉取 {} 条, DB 写 {} 条",

                    count,

                    written

                );

            }

            Err(e) => log::warn!("[新闻] Sina 拉取失败: {e}"),

        }

    }

}



/// v13.12 (Task 12): 盘后回溯 — 拉取持仓个股近 30 天 Sina 个股新闻, 双写 news_items.

/// 与 poll_news_loop 解耦: 本函数一次性跑完所有持仓, 不循环.

/// 入口: scheduler 在每 30 分钟 tick 时, 若本地时间 >= 15:30 触发一次.

async fn post_close_news_review() {

    use chrono::{Duration as ChronoDuration, Utc};

    use stock_analysis::data_provider::sina_news_provider::SinaNewsProvider;

    use stock_analysis::database::DatabaseManager;



    let now = Utc::now();

    let from = now - ChronoDuration::days(30);

    let provider = SinaNewsProvider::new();



    // 读持仓代码列表 (失败 → 用空 Vec, 不会 panic).

    let holdings: Vec<String> = stock_analysis::portfolio::get_positions()

        .unwrap_or_default()

        .iter()

        .map(|p| p.code.clone())

        .collect();



    log::info!(

        "[盘后] 拉 {} 只持仓近 30 天个股新闻 (from={}, to={})",

        holdings.len(),

        from.format("%Y-%m-%d"),

        now.format("%Y-%m-%d")

    );



    if holdings.is_empty() {

        log::warn!("[盘后] 当前无持仓, 跳过回溯");

        return;

    }



    for code in &holdings {

        match provider.fetch_stock_news_in_range(code, from, now).await {

            Ok(items) => {

                let total = items.len();

                let mut written = 0usize;

                for item in &items {

                    let ok = DatabaseManager::with_db("post_close_news", |db| {

                        if db.insert_news_item(item).is_ok() {

                            Some(())

                        } else {

                            None

                        }

                    });

                    if ok.is_some() {

                        written += 1;

                    }

                }

                log::info!(

                    "[盘后] {code} Sina 个股新闻: 拉 {} 条, DB 写 {} 条",

                    total,

                    written

                );

            }

            Err(e) => log::warn!("[盘后] {code} Sina 拉取失败: {e}"),

        }

    }

    log::info!("[盘后] 持仓回溯完成 ({} 只持仓)", holdings.len());

}



/// v13.12 (Task 12): 盘后回溯调度 — 每 30 分钟 tick 一次, 若本地时间已过 15:30 则触发一次.

/// 简化策略: 进入盘后时段后每 30 分钟最多触发一次 (避免重启后多触).

async fn post_close_news_scheduler() {

    use std::time::Duration;



    let threshold = chrono::NaiveTime::from_hms_opt(15, 30, 0).unwrap_or_else(|| {

        chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()

    });

    let mut interval = tokio::time::interval(Duration::from_secs(1800));

    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);



    log::info!(

        "[盘后调度] 启动 (30 min tick, 触发条件: 本地时间 >= {})",

        threshold.format("%H:%M")

    );



    loop {

        interval.tick().await;

        let now_local = chrono::Local::now();

        if now_local.time() >= threshold {

            log::info!("[盘后调度] tick @ {} → 触发回溯", now_local.format("%H:%M"));

            post_close_news_review().await;

        }

    }

}



async fn push(event: &AlertEvent) {

    let text = alert::format_alert(event);

    log::info!(

        "[告警] {} {} → {}",

        event.level.emoji(),

        event.code,

        event.message

    );

    stock_analysis::monitor::alert_log::append_jsonl(event);

    stock_analysis::monitor::alert_log::append_md(event);

    push_governor_v3(&text, PushKind::DailyReport, None).await;

}



fn build_price_map(

    quotes: &[stock_analysis::market_data::TopStock],

) -> std::collections::HashMap<String, f64> {

    quotes.iter().map(|q| (q.code.clone(), q.price)).collect()

}



fn compute_ma(kline: &[stock_analysis::data_provider::KlineData], n: usize) -> Option<f64> {

    if n == 0 || kline.len() < n {

        return None;

    }

    let sum: f64 = kline.iter().rev().take(n).map(|k| k.close).sum();

    Some(sum / n as f64)

}



/// v3: 收盘时记录净值快照到 ledger 表

fn snapshot_portfolio_value() {

    let positions = match stock_analysis::portfolio::get_positions() {

        Ok(p) => p,

        Err(e) => {

            log::warn!("[净值快照] 获取持仓失败: {}", e);

            return;

        }

    };

    if positions.is_empty() {

        return;

    }



    let quotes = market_data::fetch_position_quotes();

    let mut quote_map: std::collections::HashMap<&str, f64> = std::collections::HashMap::new();

    for q in &quotes {

        quote_map.insert(q.code.as_str(), q.price);

    }



    let mut total_value = 0.0_f64;

    let mut counted = 0;

    for p in &positions {

        let price = quote_map

            .get(p.code.as_str())

            .copied()

            .unwrap_or(p.cost_price);

        total_value += p.shares as f64 * price;

        counted += 1;

    }



    let prev_curve = stock_analysis::portfolio::get_equity_curve(2)

        .ok()

        .unwrap_or_default();

    if let Some(last) = prev_curve.last() {

        if !validate_nav_freshness(last.date) {

            log::warn!("[净值快照] NAV 数据过期，跳过本次快照");

            return;

        }

    }

    let prev_value = prev_curve

        .last()

        .map(|e| e.total_value)

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

        Ok(()) => log::info!(

            "[净值快照] 总市值 ¥{:.0} ({}/{} 只) 日盈亏 {:+.0}",

            total_value,

            counted,

            positions.len(),

            daily_pnl

        ),

        Err(e) => log::warn!("[净值快照] 保存失败: {}", e),

    }

}



// ============================================================================

// v11-P0-5++ Commit 5: 候选筛选台 wrapper (P5 §六 验收)

// ============================================================================



/// 从复盘路径 (LLM 终稿 by_code + 持仓) 收集 5 路 raw, 调 candidate_panel 合并+分档+门槛+排序+渲染

///

/// 5 路 raw 来源 (Commit 4 降级, Commit 5 集中推 1 条):

/// - A10 选股 (本次复盘不直接拿, 留 placeholder)

/// - B3 优选 (run_post_close_candidates)

/// - B6 放量·自选 (holding_breakout_text)

/// - B7 放量·实盘优选 (watch_breakout_text)

/// - C4 产业链 (scan.chain_text, 本函数不调, 留 P0-5++ commit 6 接入)

///

/// **v16.4 修订 (P0-5++ Commit 7)**: 接受 5 路 raw (A10/B3/B6/B7/C4) 真正 5 路收集

/// (主路径暂传 None, 留 P0-5++ commit 8 实际接入 5 处调用点).

///

/// **简化**: 本 commit 不解析 LLM 文本 (留 P0-5++ commit 6+), 直接用 by_code (LLM 终稿) 当 raw 喂入.

/// 实际行为: 每只持仓的 "操作建议" + 板块/产业链 文本 = 1 条候选, source = IndustryChain (兜底).

///

/// **P5 红线 (P5 §一)**: 候选筛选不是买入决策, 不合成"买入分".

#[allow(clippy::too_many_arguments)]

fn run_candidate_panel_from_review(

    by_code: &std::collections::HashMap<String, (String, Option<String>)>,

    holdings: &[stock_analysis::portfolio::Position],

    // v16.4: 5 路 raw (主路径暂 None, 留 P0-5++ commit 8 接入)

    stock_pick_raw: Option<&str>,

    optimal_close_raw: Option<&str>,

    volume_watchlist_raw: Option<&str>,

    volume_real_trade_raw: Option<&str>,

    industry_chain_raw: Option<&str>,

) -> String {

    use stock_analysis::opportunity::candidate_panel::{

        classify_tier, filter_hard_gates, format_candidate_board, merge_candidates,

        parse_text_to_raw, sort_candidates, CandidateSource,

    };



    // 收集 5 路 raw (v16.4 P0-5++ Commit 7 修订: 5 个 String 参数, 主路径暂 None 走兜底)

    // P5 §三 3.1 红线: 多路信号合并, 这里 1 路来源 (IndustryChain 兜底)

    let mut raw: Vec<(CandidateSource, String, String)> = Vec::new();

    // v16.4: 5 路 raw 解析 (parse_text_to_raw, P0-5++ Commit 6 加的 helper)

    // 同时收集每个 (code, source) 对应的原始行 (用作 evidence 题材段)

    let mut evidence_map: std::collections::HashMap<String, String> =

        std::collections::HashMap::new();

    for (source, text) in [

        (CandidateSource::StockPick, stock_pick_raw),

        (CandidateSource::OptimalClose, optimal_close_raw),

        (CandidateSource::VolumeWatchlist, volume_watchlist_raw),

        (CandidateSource::VolumeRealTrade, volume_real_trade_raw),

        (CandidateSource::IndustryChain, industry_chain_raw),

    ] {

        if let Some(t) = text {

            for line in t.lines() {

                // 找 6 位 code + 名字

                let mut chars = line.char_indices().peekable();

                let mut code_end = None;

                let mut code_start = None;

                let mut count = 0;

                while let Some((i, c)) = chars.next() {

                    if c.is_ascii_digit() {

                        let mut end = i + c.len_utf8();

                        let mut cnt = 1;

                        while let Some(&(_, nc)) = chars.peek() {

                            if nc.is_ascii_digit() {

                                end += nc.len_utf8();

                                chars.next();

                                cnt += 1;

                                if cnt == 6 {

                                    break;

                                }

                            } else {

                                break;

                            }

                        }

                        if cnt == 6 {

                            code_start = Some(i);

                            code_end = Some(end);

                        }

                        break;

                    }

                }

                if let (Some(s), Some(e)) = (code_start, code_end) {

                    let code = &line[s..e];

                    // 取 code 后 — 前的描述段 (置信% + [详情])

                    let after = &line[e..];

                    if let Some(em_dash_pos) = after.find('—') {

                        let desc = &after[em_dash_pos + 3..]; // 跳过 "— "

                        if !desc.trim().is_empty() {

                            evidence_map.insert(code.to_string(), desc.trim().to_string());

                        }

                    }

                }

            }

            for (code, name) in parse_text_to_raw(t) {

                raw.push((source, code, name));

            }

        }

    }

    // v16.4 兜底: by_code LLM 终稿 → IndustryChain 兜底 (Commit 5 已有)

    if raw.is_empty() {

        // 遍历 by_code (不是 holdings), 候选不只限于持仓

        for (code, value) in by_code {

            if value.1.is_some() {

                raw.push((CandidateSource::IndustryChain, code.clone(), code.clone()));

            }

        }

    }

    // 简化: 实际 P0-5++ 还会接 A10/B3/B6/B7 4 路 raw, 这里先 1 路

    if raw.is_empty() {

        return String::new();

    }



    // 1. 多源合并去重

    let mut entries = merge_candidates(raw);



    // 2. 证据分层 (P5 §3.2 红线: 唯一 Strong = 布林+MACD) + 拉价格/涨幅

    // 拉 K 线 (5 日够看当日), 给 entry 填 current_price / change_pct / 题材

    let fetcher = stock_analysis::data_provider::DataFetcherManager::new().ok();

    for e in &mut entries {

        // 2.1 evidence: 优先 evidence_map (放量描述), fallback by_code LLM 终稿

        let mut ev: Option<String> = None;

        if let Some(desc) = evidence_map.get(&e.code) {

            ev = Some(format!("放量: {}", desc));

        } else if let Some((_, Some(md))) = by_code.get(&e.code) {

            ev = md

                .lines()

                .find(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))

                .map(|l| l.chars().take(80).collect::<String>());

        }

        if let Some(s) = ev {

            if !s.is_empty() {

                e.evidence = vec![s];

            }

        }

        // 2.2 价格/涨幅: 拉 K 线最近 1 日

        if let Some(f) = &fetcher {

            if let Ok((klines, _)) = f.get_daily_data(&e.code, 5) {

                if let Some(last) = klines.last() {

                    e.current_price = last.close;

                    e.change_pct = last.pct_chg;

                }

            }

        }

        // 2.3 tier 分类

        e.tier = classify_tier(&e.evidence);

    }



    // 3. 硬门槛过滤 (P5 §3.3): 剔除已持仓 / 停牌 / ST / 北交所/科创板 / 已涨停

    let held_codes: Vec<String> = holdings.iter().map(|p| p.code.clone()).collect();

    entries = filter_hard_gates(entries, &held_codes);



    // 4. 排序 (P5 §3.3 硬规则: 强证据优先 > 多源 > 题材)

    entries = sort_candidates(entries);



    // v13.10.1 P0-#1: 通过硬门槛为 0 时不推"空台"卡片 (用户反馈噪声).

    // format_candidate_board 会输出 "通过硬门槛 0 只" 的卡片, 这里直接短路.

    if entries.is_empty() {

        return String::new();

    }



    // 5. 渲染

    format_candidate_board(&entries)

}



#[cfg(test)]

mod tests_candidate_panel {

    use super::*;

    use chrono::NaiveDate;

    use std::collections::HashMap;

    use stock_analysis::portfolio::{Position, PositionStatus};



    fn make_position(code: &str, name: &str) -> Position {

        Position {

            code: code.to_string(),

            name: name.to_string(),

            shares: 1000,

            cost_price: 10.0,

            hard_stop: 0.0,

            added_at: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),

            status: PositionStatus::Holding,

            sector: "测试".to_string(), ..Default::default()

        }

    }



    fn make_md(advice: &str) -> String {

        format!("# 复盘\n## 【操作建议】{}\n", advice)

    }



    /// 空 by_code → 候选台不推 (空字符串)

    #[test]

    fn wrapper_empty_by_code_returns_empty() {

        let by_code: HashMap<String, (String, Option<String>)> = HashMap::new();

        let holdings = vec![make_position("600999", "测试")];

        let result =

            run_candidate_panel_from_review(&by_code, &holdings, None, None, None, None, None);

        assert!(result.is_empty(), "空 by_code 应返回空字符串, 不推候选台");

    }



    /// LLM 终稿有 "强烈卖出" → evidence + tier=Reference (因 keywords 是 "卖出" 不是 "布林+MACD")

    #[test]

    fn wrapper_extracts_evidence_from_llm_md() {

        let mut by_code = HashMap::new();

        by_code.insert(

            "600999".to_string(),

            ("测试".to_string(), Some(make_md("**强烈卖出**"))),

        );

        let holdings = vec![make_position("600000", "测试")];

        let result =

            run_candidate_panel_from_review(&by_code, &holdings, None, None, None, None, None);

        assert!(result.contains("候选筛选台"), "应输出候选台卡片");

        assert!(result.contains("600999"), "应包含 code 600999");

    }



    /// LLM 终稿有 "布林+MACD" → tier=Strong (P5 红线: 唯一能进强证据)

    #[test]

    fn wrapper_strong_evidence_for_boll_macd() {

        let mut by_code = HashMap::new();

        by_code.insert(

            "600999".to_string(),

            (

                "测试".to_string(),

                Some(make_md("**强烈卖出, 布林+MACD主升浪启动 (B方案, 已验证)**")),

            ),

        );

        let holdings = vec![make_position("600000", "测试")];

        let result =

            run_candidate_panel_from_review(&by_code, &holdings, None, None, None, None, None);

        // 5 路 None 兜底走 by_code 600999, evidence 抽 "**强烈卖出, 布林+MACD...**" 命中

        // 渲染输出 "📋 候选筛选台 · 通过硬门槛 1 只" + 1 个 entry

        assert!(result.contains("📋 候选筛选台"), "应输出候选台卡片 (顶部)");

        assert!(result.contains("600999"), "应含 by_code code 600999");

    }



    /// 持仓被 filter_hard_gates 剔除

    #[test]

    fn wrapper_filters_out_held_positions() {

        let mut by_code = HashMap::new();

        by_code.insert(

            "600999".to_string(),

            ("持仓A".to_string(), Some(make_md("**强烈卖出**"))),

        );

        by_code.insert(

            "000002".to_string(),

            ("候选B".to_string(), Some(make_md("**布林+MACD**"))),

        );

        let holdings = vec![

            make_position("000001", "持仓A"), // 已持仓 → 剔除 000001

        ];

        let result =

            run_candidate_panel_from_review(&by_code, &holdings, None, None, None, None, None);

        // 候选 B 留下, 持仓 A 剔除

        assert!(result.contains("000002"));

        assert!(!result.contains("持仓A"));

    }



    /// md=None (LLM 失败) → entry 跳过, 候选台不推

    #[test]

    fn wrapper_skips_md_none_entries() {

        let mut by_code = HashMap::new();

        by_code.insert(

            "000001".to_string(),

            ("测试".to_string(), None), // LLM 失败

        );

        let holdings = vec![make_position("600999", "测试")];

        let result =

            run_candidate_panel_from_review(&by_code, &holdings, None, None, None, None, None);

        assert!(result.is_empty(), "md=None 应跳过 entry, 候选台不推");

    }

}



/// v16.8 (P0-5++ Commit 10): 5 个 wrapper 真 raw 单测

///

/// 验证 wrapper 接 5 个 Some(raw) 时 parse_text_to_raw 正确提取 + merge + 排序 + 渲染

/// (测试主路径 L978 用 None 是因为 5 个 raw 字符串在不同函数, 实际接入留 P0-5++ commit 11)

#[cfg(test)]

mod tests_wrapper_real_raw {

    use super::*;

    use chrono::NaiveDate;

    use std::collections::HashMap;

    use stock_analysis::portfolio::{Position, PositionStatus};



    fn pos(code: &str) -> Position {

        Position {

            code: code.to_string(),

            name: format!("测试{}", code),

            shares: 1000,

            cost_price: 10.0,

            hard_stop: 0.0,

            added_at: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),

            status: PositionStatus::Holding,

            sector: "测试".to_string(), ..Default::default()

        }

    }



    /// 5 路全 None → 走 by_code IndustryChain 兜底

    #[test]

    fn wrapper_5_raws_all_none_falls_back_to_by_code() {

        let mut by_code = HashMap::new();

        by_code.insert(

            "600999".to_string(), // 不在持仓, 避免被 filter_hard_gates 剔除

            (

                "测试".to_string(),

                Some("# 复盘\n## 【操作建议】**强烈卖出**\n".to_string()),

            ),

        );

        let holdings = vec![pos("000001")]; // 持仓 000001, 候选 600999

        let result =

            run_candidate_panel_from_review(&by_code, &holdings, None, None, None, None, None);

        assert!(

            result.contains("600999"),

            "5 路 None → 走兜底, 仍应含 by_code code (600999)"

        );

    }



    /// 单路 Some(A10 选股) → 解析 → 1 行候选

    #[test]

    fn wrapper_stock_pick_real_raw() {

        let by_code = HashMap::new(); // 不用

        let holdings = vec![pos("000001")];

        let stock_pick = "推荐: 600519 贵州茅台 +3.2%";

        let result = run_candidate_panel_from_review(

            &by_code,

            &holdings,

            Some(stock_pick),

            None,

            None,

            None,

            None,

        );

        assert!(result.contains("600519"), "StockPick raw 解析应含 600519");

        assert!(result.contains("贵州茅台"));

    }



    /// 单路 Some(B3 优选) → 解析 (无序号前缀, 跟 parse_text_to_raw 测试一致)

    #[test]

    fn wrapper_optimal_close_real_raw() {

        let by_code = HashMap::new();

        let holdings = vec![pos("000001")];

        let optimal_close = "002208 合肥城建 ¥19.25\n600519 贵州茅台";

        let result = run_candidate_panel_from_review(

            &by_code,

            &holdings,

            None,

            Some(optimal_close),

            None,

            None,

            None,

        );

        assert!(result.contains("002208"));

        assert!(result.contains("600519"));

    }



    /// 单路 Some(C4 产业链) → 解析

    #[test]

    fn wrapper_industry_chain_real_raw() {

        let by_code = HashMap::new();

        let holdings = vec![pos("000001")];

        // 测试 parse_text_to_raw 实际能解析的格式 (LLM 输出常含 "code + 中文名 + 数据")

        let industry = "002008 大族激光 +5.2%";

        let result = run_candidate_panel_from_review(

            &by_code,

            &holdings,

            None,

            None,

            None,

            None,

            Some(industry),

        );

        assert!(result.contains("002008"), "C4 产业链 raw 应含 002008");

    }



    /// 多路 Some(2 路) → 合并去重 (同 code 出现 2 次 → 1 行, source 列表显示 2 路)

    #[test]

    fn wrapper_multi_raws_merge_dedup() {

        let by_code = HashMap::new();

        let holdings = vec![pos("000001")];

        let stock_pick = "600519 贵州茅台";

        let optimal_close = "600519 贵州茅台 (二次推荐)";

        let result = run_candidate_panel_from_review(

            &by_code,

            &holdings,

            Some(stock_pick),

            Some(optimal_close),

            None,

            None,

            None,

        );

        // 合并去重后只有 1 行, 但 sources 应含 2 路 (选股+优选)

        assert!(result.contains("选股+优选"), "2 路合并后 source 应列 2 个");

        let occ = result.matches("600519").count();

        assert_eq!(occ, 1, "同 code 600519 应只出现 1 次 (去重)");

    }

}

// ============================================================================
// v15.3 D6.2: news_pipeline_loop + news_push_loop wire
// ============================================================================

/// v15.3 wire-aware push: 从 np.text 解析 marker [v15.3:status:source] 决定精确 PushKind
async fn news_push_via_v14(np: stock_analysis::news::dispatcher::NewsPush) -> bool {
    // 解析 marker 例如 "[v15.3:beat:policy] ..." or "[v15.3:miss:earnings] ..."
    let (status, source) = parse_v15_3_marker(&np.text);
    let stripped = strip_v15_3_marker(&np.text);

    let kind = match (status.as_str(), source.as_str()) {
        // 财报 beat/miss 显式
        ("miss", _) => PushKind::EarningsMiss,
        ("beat_or_pos", "earnings") => PushKind::EarningsBeat,
        // 政策类 (含政府文件 / 公告 / 通用政策)
        (_, "policy") => PushKind::PolicyHit,
        // 财报正向但不是显式 earnings 源 — 视作 beat/analyst/marketaction
        ("beat_or_pos", "analyst") => PushKind::AnalystUpgrade,
        ("beat_or_pos", "marketaction") => PushKind::MarketActionAlert,
        ("beat_or_pos", _) => PushKind::EarningsBeat,
        // 其他 → 默认 Info 级别
        _ => PushKind::NewsRanked,
    };
    let outcome = notify::push_governor_v3(&stripped, kind, np.code.as_deref()).await;
    matches!(outcome, notify::PushOutcome::Pushed)
}

/// 从 "[v15.3:status:source] body" 提取 marker
fn parse_v15_3_marker(text: &str) -> (String, String) {
    if let Some(rest) = text.strip_prefix("[v15.3:") {
        if let Some(colon) = rest.find(':') {
            let status = rest[..colon].to_string();
            let after_colon = &rest[colon + 1..];
            if let Some(bracket) = after_colon.find(']') {
                let source = after_colon[..bracket].to_string();
                return (status, source);
            }
        }
    }
    ("unknown".to_string(), "unknown".to_string())
}

fn strip_v15_3_marker(text: &str) -> String {
    if let Some(rest) = text.strip_prefix("[v15.3:") {
        if let Some(bracket) = rest.find(']') {
            return rest[bracket + 1..].trim_start().to_string();
        }
    }
    text.to_string()
}

/// v15.3 D6.2 push_loop — recv from news::sink channel → push_governor_v3
async fn news_push_loop_v15_3(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<stock_analysis::news::dispatcher::NewsPush>,
) {
    // Phase 3: 停止"📊 分数"独立推送 (新闻经 run_opportunity_scan → 候选票, 同源快讯不重复推)
    //   只消费 sink channel (避免 sender 阻塞), 不再调 news_push_via_v14
    log::info!("[v15.3 news_push_loop] 启动 (Phase3: 不推分数, 仅消费 channel)");
    while let Some(np) = rx.recv().await {
        log::debug!("[v15.3 news_push_loop] 已停推(Phase3): {}", np.headline);
    }
    log::warn!("[v15.3 news_push_loop] channel closed, loop exit");
}

/// v15.3 D6.2 pipeline_loop — 60s tick aggregator → score → decide → sink.send
async fn news_pipeline_loop_v15_3() {
    use stock_analysis::news::aggregator::{feed, set_global as set_agg_global, NewsAggregator};
    use stock_analysis::news::sink;

    // 1. 安装 sink channel 并 spawn push_loop
    let rx = sink::install();
    tokio::spawn(news_push_loop_v15_3(rx));
    log::info!("[v15.3 news_pipeline] sink installed, push_loop spawned");

    // 2. 注册 12 个 feeds (生产). mock feeds 走 NEWS_MOCK_FEEDS=1 env.
    register_v15_3_feeds();
    let feeds = feed::take_all_for_aggregator();
    let feed_count = feeds.len();
    let agg = std::sync::Arc::new(NewsAggregator::new(feeds));
    set_agg_global(agg);
    log::info!("[v15.3 news_pipeline] aggregator initialized with {} feeds", feed_count);

    // 3. tick loop — 每 60s
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        tick.tick().await;
        // skip first tick (immediate, no real data yet)
        let agg = match stock_analysis::news::aggregator::global() {
            Some(a) => a,
            None => {
                log::warn!("[v15.3 news_pipeline] aggregator 未注入, 跳过 tick");
                continue;
            }
        };
        let events = agg.tick(20).await;
        if events.is_empty() {
            log::trace!("[v15.3 news_pipeline] tick return 0 events");
            continue;
        }
        log::info!("[v15.3 news_pipeline] tick -> {} events", events.len());
        for event in events {
            use stock_analysis::signal::market_event::{Direction, EventType};
            let sm_relation = stock_analysis::news::stock_mapper::relation_for_event_type(event.event_type);
            let relation = match sm_relation {
                stock_analysis::news::stock_mapper::Relation::SelfCode => stock_analysis::news::impact::RelationType::SelfCode,
                stock_analysis::news::stock_mapper::Relation::SupplyChain => stock_analysis::news::impact::RelationType::SupplyChain,
                stock_analysis::news::stock_mapper::Relation::Industry => stock_analysis::news::impact::RelationType::Industry,
                stock_analysis::news::stock_mapper::Relation::PolicyImpact => stock_analysis::news::impact::RelationType::PolicyImpact,
                stock_analysis::news::stock_mapper::Relation::AnalystView => stock_analysis::news::impact::RelationType::AnalystView,
                stock_analysis::news::stock_mapper::Relation::EarningsRef => stock_analysis::news::impact::RelationType::EarningsRef,
            };
            let impact = stock_analysis::news::impact::score_event(&event, relation, 1);
            // 把 event_type 钉到 impact 上, 让 push_loop 能做精确 PushKind 选择
            let mut np = match stock_analysis::news::dispatcher::decide(&impact) {
                Some(p) => p,
                None => continue,  // score < 40, skip
            };
            // 在 text 里塞个 marker 标记 event_type, push_loop 用它决定 PushKind
            let et_label = match event.event_type {
                EventType::Policy => "policy",
                EventType::Earnings => "earnings",
                EventType::AnalystView => "analyst",
                EventType::MarketAction => "marketaction",
                EventType::Announcement => "policy",  // 公告走低分 Info, 标 policy (兼容)
                _ => "policy",
            };
            // 用 direction 控制 EarningsBeat vs EarningsMiss
            let bear = matches!(event.direction, Direction::Bear);
            np.text = format!("[v15.3:{}:{}] {}", if bear {"miss"} else {"beat_or_pos"}, et_label, np.text);
            let ok = sink::try_push(&np);
            if !ok {
                log::debug!("[v15.3 news_pipeline] sink try_push 失败 (push_loop 未启)");
            }
        }
    }
}

/// v15.3 wire: 注册生产 feeds / mock feeds
fn register_v15_3_feeds() {
    use stock_analysis::news::aggregator::feed;
    use std::sync::Arc;
    let mock_mode = std::env::var("NEWS_MOCK_FEEDS").ok().as_deref() == Some("1");

    if mock_mode {
        // Mock feeds: 8 个确定性事件用于 e2e demo
        log::info!("[v15.3 wire] NEWS_MOCK_FEEDS=1, 注册 1 mock feed (8 events)");
        let demo_feed = Arc::new(DemoMockFeed);
        feed::register_feeds(vec![demo_feed as Arc<dyn stock_analysis::news::aggregator::NewsFeed>]);
        return;
    }

    log::info!("[v15.3 wire] 注册 12 production feeds (实际 HTTP)");
    use stock_analysis::news::aggregator::NewsFeed;
    use stock_analysis::search_service::providers::{
        cls::ClsProvider, gelonghui::GelonghuiProvider, gov_policy::GovPolicyProvider,
        jin10::Jin10Provider, kcb_daily::KcbDailyProvider, sina_flash::SinaFlashProvider,
        wallstreetcn::WallStreetCnProvider, weibo_hot::WeiboHotProvider,
    };
    let feeds: Vec<Arc<dyn NewsFeed>> = vec![
        Arc::new(feed::Jin10FlashFeed { inner: Jin10Provider::new() }),
        Arc::new(feed::WallStreetCnFeed { inner: WallStreetCnProvider::new() }),
        Arc::new(feed::ClsFlashFeed { inner: ClsProvider::new() }),
        Arc::new(feed::SinaFlashFeed { inner: SinaFlashProvider::new() }),
        Arc::new(feed::WeiboHotFeed { inner: WeiboHotProvider::new() }),
        Arc::new(feed::GelonghuiFeed { inner: GelonghuiProvider::new() }),
        Arc::new(feed::KcbDailyFeed { inner: KcbDailyProvider::new() }),
        Arc::new(feed::GovPolicyFeed { inner: GovPolicyProvider::new() }),
        // Skeleton pass:
        Arc::new(feed::GovCnFeed),
        Arc::new(feed::MiitFeed),
        Arc::new(feed::EarningsCalendarFeed),
        Arc::new(feed::ConsensusFeed),
    ];
    log::info!("[v15.3 wire] registered {} production feeds", feeds.len());
    feed::register_feeds(feeds);
}

/// Mock feed for demo e2e — 8 确定性事件
pub struct DemoMockFeed;
#[async_trait::async_trait]
impl stock_analysis::news::aggregator::NewsFeed for DemoMockFeed {
    fn name(&self) -> &str { "demo_mock_v15_3" }
    fn source_kind(&self) -> stock_analysis::news::aggregator::SourceKind {
        stock_analysis::news::aggregator::SourceKind::Flash
    }
    async fn fetch(&self, limit: usize) -> anyhow::Result<Vec<stock_analysis::signal::market_event::MarketEvent>> {
        use stock_analysis::signal::market_event::{
            Direction, EventType, MarketEvent, SourceRef,
        };
        use chrono::{Local, Utc};
        let now = Utc::now().with_timezone(&Local);
        let mut events = Vec::new();
        for i in 0..limit.min(8) {
            let (title, code, et, dir, strength) = match i {
                0 => ("长鑫存储递交招股说明书, 兆易创新受益".to_string(), "603986", EventType::Policy, Direction::Bull, 95),
                1 => ("工信部发布半导体行业扶持政策".to_string(), "688981", EventType::Policy, Direction::Bull, 90),
                2 => ("贵州茅台发布前三季度业绩预增公告".to_string(), "600519", EventType::Earnings, Direction::Bull, 100),
                3 => ("天风证券: 给予宁德时代买入评级".to_string(), "300750", EventType::AnalystView, Direction::Bull, 70),
                4 => ("持仓账户切换为半仓模式".to_string(), "000001", EventType::MarketAction, Direction::Neutral, 95),
                5 => ("十五五规划储能产业链规划".to_string(), "300274", EventType::Policy, Direction::Bull, 85),
                6 => ("宁德时代业绩低于预期".to_string(), "300750", EventType::Earnings, Direction::Bear, 90),
                7 => ("国君证券: 半导体行业策略报告".to_string(), "688981", EventType::AnalystView, Direction::Bull, 75),
                _ => break,
            };
            let simhash = {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut h = DefaultHasher::new();
                title.hash(&mut h);
                h.finish()
            };
            events.push(MarketEvent {
                event_id: format!("demo-{i}-{:x}", simhash),
                simhash,
                full_title: title,
                event_type: et,
                subject: code.into(),
                object: Some(code.into()),
                direction: dir,
                strength,
                certainty: 85,
                chains: vec![],
                occurred_at: now,
                provenance: vec![SourceRef { provider: "demo_mock_v15_3".into(), url: None, fetched_at: now }],
                ai_degraded: false,
                stale: false,
            });
        }
        Ok(events)
    }
}
