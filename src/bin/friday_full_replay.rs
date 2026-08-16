//! 周五全流程回放 — 用 2026-08-07 真实数据模拟盘中各环节。
//!
//! 环节:
//!   A. 板块样本 (15:00 视角) — Eastmoney 板块资金流排行 (周六返回
//!      周五收盘后最新数据, 真实), 走生产同款 fetch_board_ranking + render
//!   B. 做T 数据链 (收盘时刻 15:29:56 视角) — TDX T0 全链 with_clock 注入,
//!      周五报价快照 + 5min 线 (真实 TDX 数据)
//!   C. T-12 尾盘跳水判定 (14:55 视角) — 快照成本 vs 周五收盘价, pnl ≤ -3%
//!      的持仓 = 若当日 14:55 会推送的跳水票 (判定逻辑同 prepare_close_call)
//!
//! 只读网络请求 + DB 只读, 无写入、无推送。

use chrono::{FixedOffset, TimeZone, Utc};
use stock_analysis::data_gateway::magic_tdx_t0::fetch_magic_tdx_t0_batch_with_clock;

fn main() {
    stock_analysis::database::DatabaseManager::init(Some(
        "/Users/zhangzhen/Desktop/Quant/stock_analysis/data/stock_analysis.db".into(),
    ))
    .expect("production db init");

    let tz = FixedOffset::east_opt(8 * 3600).unwrap();
    let friday = |h: u32, m: u32, s: u32| {
        tz.with_ymd_and_hms(2026, 8, 7, h, m, s)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    };

    println!("══════════════ A. 板块样本 (周五收盘数据) ══════════════");
    match stock_analysis::market_analyzer::sector_monitor::fetch_board_ranking("f3", 5) {
        Ok(boards) if !boards.is_empty() => {
            for b in &boards {
                println!(
                    "  {} {:+.2}% 主力{:+.2}亿",
                    b.name, b.change_pct, b.main_inflow / 1e8
                );
            }
        }
        Ok(_) => println!("板块数据空 (Eastmoney 周六无数据)"),
        Err(e) => println!("板块数据失败: {e}"),
    }

    println!("\n══════════════ B. 做T 数据链 (周五 15:29:56 收盘视角) ══════════════");
    let codes: Vec<String> = std::env::var("STOCK_LIST")
        .ok()
        .map(|s| s.split(',').take(10).map(str::to_owned).collect())
        .unwrap_or_else(|| vec!["605178".to_string()]);
    let replay_at = friday(15, 29, 56);
    match fetch_magic_tdx_t0_batch_with_clock(&codes, replay_at, Some(replay_at)) {
        Ok(batch) => {
            println!(
                "BATCH: requested_at={} source_at={} observed_at={}",
                batch.requested_at, batch.source_at, batch.observed_at
            );
            println!(
                "  fresh={} rejected={}",
                batch.records.len(),
                batch.rejections.len()
            );
            for r in &batch.records {
                println!(
                    "  [fresh] {} quote={} source_at={} settled_daily={} 5min={} avg={:.2}",
                    r.code,
                    r.quote.price,
                    r.source_at,
                    r.settled_daily.len(),
                    r.completed_five_minute.len(),
                    r.intraday_average_price
                );
            }
            for rej in &batch.rejections {
                println!(
                    "  [rejected] {} reason={} detail={}",
                    rej.code, rej.reason_code, rej.detail
                );
            }
        }
        Err(e) => println!("做T 链失败: {e:#}"),
    }

    println!("\n══════════════ C. T-12 尾盘跳水判定 (14:55 视角) ══════════════");
    use diesel::query_dsl::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct PosRow {
        #[diesel(sql_type = diesel::sql_types::Text)]
        code: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        name: String,
        #[diesel(sql_type = diesel::sql_types::Double)]
        cost_price: f64,
    }
    let db = stock_analysis::database::DatabaseManager::get();
    let mut conn = db.get_conn().expect("conn");
    let rows: Vec<PosRow> = diesel::sql_query(
        "SELECT code, name, cost_price FROM user_position_snapshot_item ORDER BY code",
    )
    .load::<PosRow>(&mut conn)
    .expect("position snapshot read");
    // 周五收盘价: 用 5min 线 15:00 bar (真实 TDX 历史, 不依赖 5s 报价门)
    let client = magic_tdx_rs::TdxHqClient::new();
    client.connect_to_any(Some(5.0)).expect("tdx connect");
    let mut closes: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for row in &rows {
        let market = if row.code.starts_with('6') { 1u8 } else { 0u8 };
        if let Ok(bars) = client.get_security_bars(
            magic_tdx_rs::protocol::constants::KLINE_5MIN,
            market,
            &row.code,
            0,
            400,
            magic_tdx_rs::protocol::constants::fq_type::NONE,
        ) {
            if let Some(last) = bars.last() {
                if last.year == 2026 && last.month == 8 && last.day == 7 {
                    closes.insert(row.code.clone(), last.close);
                }
            }
        }
    }
    println!("收盘价获取: {} 只持仓 {} 只有效收盘价", rows.len(), closes.len());
    let mut dump: Vec<(&str, &str, f64, f64)> = Vec::new();
    for r in &rows {
        if let Some(close) = closes.get(&r.code) {
            if r.cost_price > 0.0 {
                dump.push((&r.code, &r.name, r.cost_price, *close));
            }
        }
    }
    dump.sort_by(|a, b| (a.3 / a.2).partial_cmp(&(b.3 / b.2)).unwrap());
    println!("\n  # 代码  名称        成本    周五收盘  盈亏%   T-12判定");
    for (i, (code, name, cost, close)) in dump.iter().enumerate() {
        let pnl = (close / cost - 1.0) * 100.0;
        let verdict = if pnl <= -3.0 {
            "🔴 跳水-建议处理"
        } else {
            "正常"
        };
        println!(
            "  {:2} {} {} {:8.2} {:8.2} {:+6.2}%  {}",
            i + 1, code, name, cost, close, pnl, verdict
        );
    }
}
