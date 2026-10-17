//! 打印一只票的完整生命周期与事件时间轴 (运维 / 验证工具).
//!
//! 用法:
//!   STOCK_DB=<path> cargo run --release --bin ticket_show -- 600396
//!   STOCK_DB=<path> cargo run --release --bin ticket_show -- --active
//!
//! 说明: 这是"票的一等公民"最小切片的可见产出 —— 跑一次就能确认表结构
//!       与聚合是否符合预期, 再决定要不要把更多 push_kind 挂上去.
//!
//! 注意: 独立启动走 `init_retained_monitor`, 对**生产大库**会触发 BR-159 全量
//!       验链 (10+ 分钟). 只读查看看历史可用 `sqlite3 "file:...?mode=ro"` 直接查.

use stock_analysis::database::{ticket_lifecycle, DatabaseManager};

fn print_timeline(db: &DatabaseManager, code: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let tickets = ticket_lifecycle::load_tickets(&mut *conn, code)?;
    if tickets.is_empty() {
        println!("{code}: 没有 ticket 记录");
        return Ok(());
    }
    for ticket in &tickets {
        let span = match &ticket.close_date {
            Some(close) => format!("{} → {}", ticket.open_date, close),
            None => format!("{} → (活跃)", ticket.open_date),
        };
        println!(
            "\n【ticket#{}】{}  {}  stage={}  source={}",
            ticket.id, ticket.code, span, ticket.stage, ticket.source
        );
        if let Some(note) = &ticket.note {
            println!("  note: {note}");
        }
        let events = ticket_lifecycle::load_events(&mut *conn, ticket.id)?;
        if events.is_empty() {
            println!("  (无事件)");
            continue;
        }
        for event in &events {
            let mut line = format!("  {} [{}]", event.event_at, event.event_kind);
            if let Some(kind) = &event.push_kind {
                line.push_str(&format!(" kind={kind}"));
            }
            if let Some(identity) = &event.occurrence_identity {
                line.push_str(&format!(" id={identity}"));
            }
            if let Some(summary) = &event.summary {
                line.push_str(&format!(" | {summary}"));
            }
            println!("{line}");
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let path = std::env::var("STOCK_DB").ok().map(std::path::PathBuf::from);
    DatabaseManager::init_retained_monitor(path)?;
    let db = DatabaseManager::get();

    if args.iter().any(|arg| arg == "--active") {
        let mut conn = db.get_conn().map_err(|e| e.to_string())?;
        let active = ticket_lifecycle::count_active(&mut *conn)?;
        println!("活跃 ticket: {active}");
        return Ok(());
    }

    let Some(code) = args.iter().skip(1).find(|arg| !arg.starts_with("--")) else {
        return Err("用法: ticket_show <code> | ticket_show --active".into());
    };
    print_timeline(db, code)
}
