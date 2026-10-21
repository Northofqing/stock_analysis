//! 手工开票 / 改状态 / 记事件 (运维工具).
//!
//! 用法:
//!   STOCK_DB=<path> cargo run --release --bin ticket_open -- 600396
//!   STOCK_DB=<path> cargo run --release --bin ticket_open -- 600396 --date 2026-10-19
//!   STOCK_DB=<path> cargo run --release --bin ticket_open -- 600396 --stage holding
//!   STOCK_DB=<path> cargo run --release --bin ticket_open -- 600396 --note "板块热度第2"
//!   STOCK_DB=<path> cargo run --release --bin ticket_open -- 600396 --close 2026-10-23
//!   STOCK_DB=<path> cargo run --release --bin ticket_open -- 600396 --event "放量突破 12.60"
//!
//! 说明: 这是"观察池"的手工入口 —— 看到好票直接加进池子, 不依赖任何自动
//!       选股逻辑. 自动入口(从候选台导入)等选股口径定下来再接.
//!
//! 注意: 独立启动走 `init_retained_monitor`, 对**生产大库**会触发 BR-159 全量
//!       验链 (10+ 分钟).

use chrono::{Local, NaiveDate};
use stock_analysis::database::{ticket_lifecycle, DatabaseManager};

fn arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .map(|value| value.as_str())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let Some(code) = args.iter().skip(1).find(|arg| !arg.starts_with("--")) else {
        return Err(
            "用法: ticket_open <code> [--date YYYY-MM-DD] [--stage S] [--note T] \
                    [--close YYYY-MM-DD] [--event T]"
                .into(),
        );
    };

    let path = std::env::var("STOCK_DB").ok().map(std::path::PathBuf::from);
    DatabaseManager::init_retained_monitor(path)?;
    let db = DatabaseManager::get();
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;

    let today = Local::now().date_naive();
    let date = match arg_value(&args, "--date") {
        Some(raw) => NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .map_err(|error| format!("--date 非法 ({raw}): {error}"))?,
        None => today,
    };

    // --close：结束既有 ticket
    if let Some(raw) = arg_value(&args, "--close") {
        let close_date = NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .map_err(|error| format!("--close 非法 ({raw}): {error}"))?;
        let Some(ticket) = ticket_lifecycle::find_active_ticket(&mut *conn, code)? else {
            return Err(format!("{code} 没有活跃 ticket，无法关闭").into());
        };
        let stage = arg_value(&args, "--stage").unwrap_or("exited");
        let changed = ticket_lifecycle::close_ticket(&mut *conn, ticket.id, &raw, stage)?;
        println!(
            "已关闭 ticket#{} ({code}) close_date={close_date} stage={stage} rows={changed}",
            ticket.id
        );
        return Ok(());
    }

    // 开票（幂等）
    let source = arg_value(&args, "--source").unwrap_or("manual");
    let stage = arg_value(&args, "--stage").unwrap_or("observing");
    let note = arg_value(&args, "--note");
    let date_text = date.format("%Y-%m-%d").to_string();
    let ticket_id =
        ticket_lifecycle::open_ticket(&mut *conn, code, &date_text, stage, source, note)?;

    // 可选：同步改 stage（open_ticket 对已存在的票不改 stage）
    if let Some(new_stage) = arg_value(&args, "--stage") {
        let changed = ticket_lifecycle::set_stage(&mut *conn, ticket_id, new_stage)?;
        if changed == 0 {
            println!("stage 未变更（ticket 可能已关闭）");
        }
    }

    // 打印**数据库里的实际状态**，而不是请求值：open_ticket 是幂等的，
    // 已存在的票不会被覆盖，打印请求值会误导（2026-10-17 验证时踩到）。
    match ticket_lifecycle::find_active_ticket(&mut *conn, code)? {
        Some(actual) => println!(
            "ticket#{} {code} open_date={} stage={} source={}",
            actual.id, actual.open_date, actual.stage, actual.source
        ),
        None => println!("ticket#{ticket_id} {code} 已关闭（非活跃）"),
    }

    // 可选：记一条事件
    if let Some(summary) = arg_value(&args, "--event") {
        let event_at = Local::now().to_rfc3339();
        ticket_lifecycle::append_event(
            &mut *conn,
            ticket_id,
            &event_at,
            "note",
            None,
            None,
            Some(summary),
        )?;
        println!("event @ {event_at} | {summary}");
    }
    Ok(())
}
