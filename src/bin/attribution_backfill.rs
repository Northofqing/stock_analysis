//! 归因补跑工具 (BR-255 2026-09-01): 15:05-15:20 窗口外的当日归因手动补跑。
//!
//! 生产 monitor 的归因只在 15:05-15:20 窗口内自动执行, 窗口关闭后当天永久缺失
//! (2026-09-01 实测: 15:05 窗口期间上游服务器崩溃循环, 13 次重试耗尽)。本工具在
//! 事后补算当日归因: 持仓 → 日线收盘价 → compute → 报告落盘。
//!
//! 路径选择:
//! - 活跃 epoch 的 effective_date <= 目标日 → epoch 完整路径 (compute + persist +
//!   报告落盘), 与生产 15:05 tick 同构 (2026-09-01 起 epoch 已激活, effective=9/2,
//!   9/2 之后的补跑走此路径)。
//! - 目标日早于 effective_date → 只读重建 (reconstruct_epoch_daily): 以目标日前
//!   一交易日的投影 + 同一纯引擎重建报告, 零 DB 写入、无 epoch 绑定 (epoch 一次性
//!   激活 + 链表不可变, 既往日期无法经 epoch 路径补算)。2026-09-01 的激活发生在
//!   当日 19:48 (invoked_at=15:40) → completed=9/1, effective=9/2, 9/1 归因只能
//!   经重建得到。
//!
//! 本工具不激活 epoch: 激活归生产 monitor 15:35-15:50 tick 所有 (一次性), 补跑
//! 工具若自行激活会以错误冻结点占用唯一激活额度 (2026-09-01 实测教训)。
//!
//! 行情来源说明 (BR-217/218 五秒新鲜度门): 实时行情准入要求 source_at 与
//! observed_at 相差 ≤5s (admit_quote_batch), 数据源冻结后 (收盘后/周末) 全部
//! 报价年龄必超 5s → RealtimeQuotes 100% fail-closed (2026-09-01 实锤: 数据源
//! 16:14:53 北京冻结后全 fail, 8/22 周六 23257 次失败 0 成功)。补跑在窗口外,
//! 故走 HistoricalDailyBars (tdx-smart): 无新鲜度门, 收盘价已落库, 24/7 可用
//! (与 R-13 复盘同源, 收盘后 10:15:32Z 实测 accepted)。
//!
//! 用法: attribution_backfill [YYYY-MM-DD]   (缺省 = 今天)

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{Local, NaiveDate};
use stock_analysis::data_gateway::HistoricalBarsGateway;
use stock_analysis::database::attribution_epochs::{reconstruct_epoch_daily, AttributionEpochStore};
use stock_analysis::database::user_position_snapshot::latest_user_position_snapshot;
use stock_analysis::database::DatabaseManager;
use stock_analysis::performance::attribution::{
    compute_epoch_daily, compute_epoch_window, persist_epoch_daily,
};
use stock_analysis::performance::report::{render_full_markdown, render_summary};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();
    let date: NaiveDate = match std::env::args().nth(1) {
        Some(arg) => match NaiveDate::parse_from_str(&arg, "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => {
                eprintln!("用法: attribution_backfill [YYYY-MM-DD]   (缺省 = 今天)");
                std::process::exit(2);
            }
        },
        None => Local::now().date_naive(),
    };

    let database_path = std::env::var("DATABASE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./data/stock_analysis.db"));
    eprintln!("[backfill] 阶段1: 初始化数据库 {database_path:?}");
    DatabaseManager::init(Some(database_path)).expect("数据库初始化失败");
    eprintln!("[backfill] 阶段2: 数据库就绪, 查持仓快照");

    // 持仓代码: BR-226 用户快照 (24h 新鲜) 优先, 否则本地持仓 (与 monitor 同规则)。
    let mut codes: Vec<String> = match latest_user_position_snapshot() {
        Ok(Some(snapshot)) if !snapshot.confirm_empty => {
            let fresh = Local::now()
                .signed_duration_since(snapshot.effective_at.with_timezone(&Local))
                .num_hours()
                <= 24;
            if fresh {
                snapshot.items.iter().map(|item| item.code.clone()).collect()
            } else {
                fallback_positions()
            }
        }
        _ => fallback_positions(),
    };
    if codes.is_empty() {
        eprintln!("[backfill] 持仓列表为空, 无法拉取行情");
        std::process::exit(1);
    }
    // 目标日全部成交代码并入行情覆盖: 未估值 lot 全部来自当日新开仓 (快照只
    // 覆盖持仓代码, 覆盖不到当日新开仓), 补跑重建应覆盖当日全部成交代码才能
    // 完整估值浮盈 (2026-09-01 实测: 快照 7 只 → 67 lot 未估值)。只读查询。
    {
        use diesel::prelude::*;
        #[derive(diesel::QueryableByName)]
        struct CodeRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            code: String,
        }
        let mut conn = DatabaseManager::get()
            .get_conn()
            .expect("数据库连接失败");
        let window_start = format!("{} 00:00:00", date.format("%Y-%m-%d"));
        let window_end = format!(
            "{} 00:00:00",
            date.succ_opt().expect("日期上溢").format("%Y-%m-%d")
        );
        let trade_codes: Vec<String> = diesel::sql_query(
            "SELECT DISTINCT code FROM paper_trades WHERE status = 'Filled' AND ts >= ? AND ts < ?",
        )
        .bind::<diesel::sql_types::Text, _>(&window_start)
        .bind::<diesel::sql_types::Text, _>(&window_end)
        .load::<CodeRow>(&mut conn)
        .expect("当日交易代码查询失败")
        .into_iter()
        .map(|row| row.code)
        .collect();
        eprintln!(
            "[backfill] 目标日 {date} 成交代码 {} 只并入行情覆盖",
            trade_codes.len()
        );
        codes.extend(trade_codes);
        codes.sort();
        codes.dedup();
    }
    eprintln!(
        "[backfill] 阶段3: 行情覆盖 {} 只 (快照 + 当日成交), 拉日线收盘价",
        codes.len()
    );

    // 统一网关 HistoricalDailyBars (fail-closed, 自带审计证据)。收盘后实时行情
    // 因 BR-217/218 五秒新鲜度门必挂, 故用日线收盘价 (与 R-13 复盘同源)。
    let bars_gateway = HistoricalBarsGateway;
    let mut prices: HashMap<String, f64> = HashMap::new();
    for code in &codes {
        // 回拉窗口: 目标日距今 + 5 天缓冲 (保证目标日 bar 在返回区间内)。
        let days = (Local::now().date_naive() - date).num_days().max(0) as usize + 5;
        let admitted = bars_gateway
            .required_daily_bars(code, days)
            .expect("统一行情网关日线不可用 (上游 gRPC 失败)");
        let bar = admitted
            .records()
            .iter()
            .find(|k| k.date == date)
            .unwrap_or_else(|| {
                let dates: Vec<String> = admitted
                    .records()
                    .iter()
                    .map(|k| k.date.to_string())
                    .collect();
                panic!("{code}: 目标日 {date} 无日线记录 (可用: {})", dates.join(","));
            });
        prices.insert(code.clone(), bar.close);
        println!("[backfill] {code}: close={} ({})", bar.close, bar.date);
    }
    println!("[backfill] 行情就绪: {} 只持仓收盘价 (日线)", prices.len());

    let database = DatabaseManager::get();

    // 路径选择: 活跃 epoch 的 effective_date <= 目标日 → epoch 完整路径
    // (compute + persist + 落盘, 与生产 15:05 tick 同构); 否则只读重建
    // (零 DB 写入, 见模块注释)。
    let epoch_usable = match AttributionEpochStore::new(database).verify_active() {
        Ok(receipt) => {
            eprintln!(
                "[backfill] 活跃 epoch: effective={} (epoch_id={})",
                receipt.effective_trading_date, receipt.epoch_id
            );
            receipt.effective_trading_date <= date
        }
        Err(error) => {
            eprintln!("[backfill] 无活跃 epoch ({error:?}), 走只读重建");
            false
        }
    };

    if epoch_usable {
        let daily =
            compute_epoch_daily(&database, date, &prices).expect("compute_epoch_daily 失败");
        let window =
            compute_epoch_window(&database, date, 30, &prices).expect("compute_epoch_window 失败");
        persist_epoch_daily(&database, &daily).expect("persist_epoch_daily 失败");
        let md = render_full_markdown(daily.daily(), window.window());
        write_report(date, &md);
        println!("{}", render_summary(daily.daily(), window.window()));
        println!("[backfill] epoch 路径完成: {date}");
    } else {
        // 只读重建: 以目标日前一交易日投影 (8/31 epoch 若成功激活本应服务 9/1)。
        let mut completed = date
            .checked_sub_signed(chrono::Duration::days(1))
            .expect("日期下溢");
        while !stock_analysis::calendar::verified_a_share_trading_day(completed)
            .expect("日历覆盖")
        {
            completed = completed
                .checked_sub_signed(chrono::Duration::days(1))
                .expect("日期下溢");
        }
        eprintln!("[backfill] 只读重建: completed={completed} target={date}");
        let (daily, window) = reconstruct_epoch_daily(&database, completed, date, &prices)
            .expect("reconstruct_epoch_daily 失败");
        let md = render_full_markdown(&daily, &window);
        write_report(date, &md);
        println!("{}", render_summary(&daily, &window));
        println!("[backfill] 只读重建完成 (无 epoch 绑定, 零 DB 写入): {date}");
    }
}

fn write_report(date: NaiveDate, md: &str) {
    let report_path = format!("data/attribution/{}.md", date.format("%Y-%m-%d"));
    std::fs::create_dir_all("data/attribution").expect("创建 data/attribution 失败");
    std::fs::write(&report_path, md).expect("写归因报告失败");
    println!("[backfill] 报告已落盘: {report_path}");
}

fn fallback_positions() -> Vec<String> {
    stock_analysis::portfolio::get_positions()
        .expect("持仓查询失败")
        .into_iter()
        .map(|position| position.code)
        .collect()
}
