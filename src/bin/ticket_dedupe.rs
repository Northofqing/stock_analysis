//! 重复活跃 ticket 只读盘点（WP4，2026-10-27）。
//!
//! 本轮只做 dry-run，不关闭任何票。真实清理必须由 operator 明确批准，
//! 并在备份后另行实现 apply 流程；这里故意不提供写入路径，避免误操作。

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use rusqlite::{Connection, OpenFlags};

#[derive(Debug, Parser)]
#[command(about = "盘点同一 code 的多张活跃 ticket（只读）")]
struct Args {
    /// 主库路径
    #[arg(long)]
    database: PathBuf,
}

#[derive(Debug)]
struct ActiveTicket {
    id: i64,
    code: String,
    stage: String,
    source: String,
    open_date: String,
    latest_event: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let conn = Connection::open_with_flags(&args.database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("只读打开失败: {}", args.database.display()))?;

    let mut stmt = conn.prepare(
        "SELECT t.id, t.code, t.stage, t.source, t.open_date, \
                (SELECT e.event_kind || '@' || e.event_at \
                   FROM ticket_event e \
                  WHERE e.ticket_id = t.id \
                  ORDER BY e.event_at DESC, e.id DESC LIMIT 1) AS latest_event \
           FROM ticket_lifecycle t \
          WHERE t.close_date IS NULL \
          ORDER BY t.code, t.open_date, t.id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ActiveTicket {
            id: row.get(0)?,
            code: row.get(1)?,
            stage: row.get(2)?,
            source: row.get(3)?,
            open_date: row.get(4)?,
            latest_event: row.get(5)?,
        })
    })?;

    let mut tickets = Vec::new();
    for row in rows {
        tickets.push(row?);
    }

    let mut groups = 0usize;
    let mut cursor = 0usize;
    while cursor < tickets.len() {
        let code = tickets[cursor].code.clone();
        let mut end = cursor + 1;
        while end < tickets.len() && tickets[end].code == code {
            end += 1;
        }
        if end - cursor > 1 {
            groups += 1;
            println!("[重复] code={code} active={}", end - cursor);
            for ticket in &tickets[cursor..end] {
                println!(
                    "  id={} stage={} source={} open_date={} latest={}",
                    ticket.id,
                    ticket.stage,
                    ticket.source,
                    ticket.open_date,
                    ticket.latest_event.as_deref().unwrap_or("<none>")
                );
            }
        }
        cursor = end;
    }

    if groups == 0 {
        println!("没有重复活跃 ticket");
    } else {
        println!("重复组={groups}；本轮只读，未关闭任何票");
    }
    Ok(())
}
