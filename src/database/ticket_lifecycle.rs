//! 票的生命周期 (2026-10-17): 把"一只票"变成系统的一等公民.
//!
//! 背景: 系统有 60+ 种 push_kind, 每种都是"某模块算完推一条"的孤立信号,
//! 没有"以票为主键"的聚合层. 于是"盘中跟踪 / 次日决策 / 盈亏复盘"三件事
//! 都无法回答同一个问题 —— 这只票的完整历史是什么.
//!
//! 本模块只做"数据层": 两张表 + 读写 API, **不改任何现有推送链路**.
//! 现有推送照常走 v14_adapter / durable_delivery; 本表只做旁路记录与聚合.
//!
//! 设计要点:
//! - `stage` 用 TEXT **不加 CHECK 约束** —— 枚举必然演进(尤其"加仓/减仓"是
//!   独立 stage 还是同一 stage 的两个事件), 硬约束会让每次调整都要迁移表.
//!   语义由 [`STAGES`] 常量 + 应用层校验保证, 表结构保持可演进.
//! - `ticket_event` 只存"指针 + 摘要", 不重复存 payload: 明细仍在
//!   `advice_outcome_tracking` / `delivery_decisions` 里, 这里只做聚合索引.
//! - 与 `candidate_panel` 的分工: 候选台 = **当日筛选结果**(每天重算);
//!   ticket = **被系统持续跟踪的票**(跨交易日, 有生命周期). 同一只票可以
//!   在 ticket 里待一周, 而候选台只认当天.

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable, Text};

/// 已知的 stage 取值（**文档性常量，不是表约束**）。
///
/// 新增取值时只需此处加一项，无需迁移表结构。
pub const STAGES: &[&str] = &[
    "observing",      // 在观察池里，尚未提出建仓
    "entry_proposed", // 已给出建仓建议（含价格），等待执行
    "holding",        // 已建仓/已持有
    "adding",         // 加仓进行中
    "reducing",       // 减仓进行中
    "exited",         // 已清仓
    "unknown",        // 状态不可判定（例如数据缺失），不得折叠成其它值
];

/// 建表 (幂等). 由 `init_with_scope` 调用.
pub fn create_schema(conn: &mut SqliteConnection) -> Result<(), diesel::result::Error> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS ticket_lifecycle (\
         id INTEGER PRIMARY KEY AUTOINCREMENT,\
         code TEXT NOT NULL,\
         open_date DATE NOT NULL,\
         close_date DATE,\
         stage TEXT NOT NULL,\
         source TEXT NOT NULL,\
         note TEXT,\
         created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,\
         updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP)",
    )
    .execute(conn)?;
    // 同一只票同一天只开一条 ticket（重跑盘前任务时幂等）。
    diesel::sql_query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_ticket_lifecycle_code_open \
         ON ticket_lifecycle(code, open_date)",
    )
    .execute(conn)
    .ok();
    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS idx_ticket_lifecycle_code ON ticket_lifecycle(code)",
    )
    .execute(conn)
    .ok();
    // 查"当前活跃票"的主用索引。
    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS idx_ticket_lifecycle_active \
         ON ticket_lifecycle(stage, close_date)",
    )
    .execute(conn)
    .ok();

    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS ticket_event (\
         id INTEGER PRIMARY KEY AUTOINCREMENT,\
         ticket_id INTEGER NOT NULL,\
         event_at TIMESTAMP NOT NULL,\
         event_kind TEXT NOT NULL,\
         push_kind TEXT,\
         occurrence_identity TEXT,\
         summary TEXT,\
         created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP)",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS idx_ticket_event_ticket ON ticket_event(ticket_id, event_at)",
    )
    .execute(conn)
    .ok();
    // 事件 ↔ 推送记录的双向对位（advice_outcome_tracking 用同一列）。
    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS idx_ticket_event_occurrence \
         ON ticket_event(occurrence_identity)",
    )
    .execute(conn)
    .ok();
    Ok(())
}

/// 一条 ticket（一个生命周期）。
#[derive(Debug, Clone)]
pub struct Ticket {
    pub id: i64,
    pub code: String,
    pub open_date: String,
    pub close_date: Option<String>,
    pub stage: String,
    pub source: String,
    pub note: Option<String>,
}

/// 一条事件。
#[derive(Debug, Clone)]
pub struct TicketEvent {
    pub event_at: String,
    pub event_kind: String,
    pub push_kind: Option<String>,
    pub occurrence_identity: Option<String>,
    pub summary: Option<String>,
}

/// 开一条 ticket；已存在 (code, open_date) 时返回既有 id，不重复插入。
pub fn open_ticket(
    conn: &mut SqliteConnection,
    code: &str,
    open_date: &str,
    stage: &str,
    source: &str,
    note: Option<&str>,
) -> Result<i64, String> {
    // 2026-10-22: 同 code 活跃票防护。
    //
    // 同一只票已有活跃 ticket（observing / entry_proposed / holding）时不再新开：
    // 跨日重复入选（如 P01 连推两日）会造出孤儿票——find_active_ticket 只取最新
    // 一张，淘汰扫描按 code 走也永远命中新票，旧的 entry_proposed 票逃逸 10 天 /
    // 15% 两条规则永远关不掉，明日计划还会同票列两行。
    if let Some(existing) = find_active_ticket(conn, code)? {
        // 记录一次「重复入选」事件，让选股信号在票的历史里可见（同日幂等）。
        let summary = format!("重复入选（已有 ticket#{}）", existing.id);
        let already = event_exists_same_day(conn, existing.id, "catalyst", open_date, &summary)
            .unwrap_or(false);
        if !already {
            let _ = append_event(
                conn,
                existing.id,
                open_date,
                "catalyst",
                None,
                None,
                Some(&summary),
            );
        }
        return Ok(existing.id);
    }

    diesel::sql_query(
        "INSERT OR IGNORE INTO ticket_lifecycle (code, open_date, stage, source, note) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(open_date)
    .bind::<Text, _>(stage)
    .bind::<Text, _>(source)
    .bind::<Nullable<Text>, _>(note)
    .execute(conn)
    .map_err(|e| format!("ticket open failed ({code} {open_date}): {e}"))?;

    #[derive(QueryableByName)]
    struct Id {
        #[diesel(sql_type = BigInt)]
        id: i64,
    }
    diesel::sql_query("SELECT id FROM ticket_lifecycle WHERE code = ? AND open_date = ? LIMIT 1")
        .bind::<Text, _>(code)
        .bind::<Text, _>(open_date)
        .get_result::<Id>(conn)
        .map(|row| row.id)
        .map_err(|e| format!("ticket id lookup failed ({code} {open_date}): {e}"))
}

/// 更新 stage（只在 ticket 仍活跃时生效）。
pub fn set_stage(
    conn: &mut SqliteConnection,
    ticket_id: i64,
    stage: &str,
) -> Result<usize, String> {
    diesel::sql_query(
        "UPDATE ticket_lifecycle SET stage = ?, updated_at = CURRENT_TIMESTAMP \
         WHERE id = ? AND close_date IS NULL",
    )
    .bind::<Text, _>(stage)
    .bind::<BigInt, _>(ticket_id)
    .execute(conn)
    .map_err(|e| format!("ticket stage update failed (id={ticket_id}): {e}"))
}

/// 关闭 ticket（写 close_date + 终态 stage）。
pub fn close_ticket(
    conn: &mut SqliteConnection,
    ticket_id: i64,
    close_date: &str,
    stage: &str,
) -> Result<usize, String> {
    diesel::sql_query(
        "UPDATE ticket_lifecycle SET stage = ?, close_date = ?, updated_at = CURRENT_TIMESTAMP \
         WHERE id = ? AND close_date IS NULL",
    )
    .bind::<Text, _>(stage)
    .bind::<Text, _>(close_date)
    .bind::<BigInt, _>(ticket_id)
    .execute(conn)
    .map_err(|e| format!("ticket close failed (id={ticket_id}): {e}"))
}

/// 追一条事件。`summary` 是一句话摘要，**不要存推送全文**。
#[allow(clippy::too_many_arguments)]
pub fn append_event(
    conn: &mut SqliteConnection,
    ticket_id: i64,
    event_at: &str,
    event_kind: &str,
    push_kind: Option<&str>,
    occurrence_identity: Option<&str>,
    summary: Option<&str>,
) -> Result<(), String> {
    diesel::sql_query(
        "INSERT INTO ticket_event \
         (ticket_id, event_at, event_kind, push_kind, occurrence_identity, summary) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind::<BigInt, _>(ticket_id)
    .bind::<Text, _>(event_at)
    .bind::<Text, _>(event_kind)
    .bind::<Nullable<Text>, _>(push_kind)
    .bind::<Nullable<Text>, _>(occurrence_identity)
    .bind::<Nullable<Text>, _>(summary)
    .execute(conn)
    .map_err(|e| format!("ticket event insert failed (ticket={ticket_id}): {e}"))?;
    Ok(())
}

/// 同日同票同摘要的事件是否已存在（幂等判重，2026-10-22）。
///
/// 用于持仓差量识别：同一份快照被重复比对时（重启补跑、多轮扫描），
/// 同一笔「新建仓 500 股」不会重复写多条事件。
pub fn event_exists_same_day(
    conn: &mut SqliteConnection,
    ticket_id: i64,
    event_kind: &str,
    event_at: &str,
    summary: &str,
) -> Result<bool, String> {
    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        n: i64,
    }
    let row: CountRow = diesel::sql_query(
        "SELECT COUNT(*) AS n FROM ticket_event \
          WHERE ticket_id = ? AND event_kind = ? AND date(event_at) = date(?) AND summary = ?",
    )
    .bind::<BigInt, _>(ticket_id)
    .bind::<Text, _>(event_kind)
    .bind::<Text, _>(event_at)
    .bind::<Text, _>(summary)
    .get_result(conn)
    .map_err(|e| format!("ticket event dedup query failed (ticket={ticket_id}): {e}"))?;
    Ok(row.n > 0)
}

/// 取该票当前活跃的 ticket（close_date IS NULL）；没有则 None。
pub fn find_active_ticket(
    conn: &mut SqliteConnection,
    code: &str,
) -> Result<Option<Ticket>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = BigInt)]
        id: i64,
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Text)]
        open_date: String,
        #[diesel(sql_type = Nullable<Text>)]
        close_date: Option<String>,
        #[diesel(sql_type = Text)]
        stage: String,
        #[diesel(sql_type = Text)]
        source: String,
        #[diesel(sql_type = Nullable<Text>)]
        note: Option<String>,
    }
    let row = diesel::sql_query(
        "SELECT id, code, open_date, close_date, stage, source, note \
         FROM ticket_lifecycle WHERE code = ? AND close_date IS NULL \
         ORDER BY open_date DESC, id DESC LIMIT 1",
    )
    .bind::<Text, _>(code)
    .get_result::<Row>(conn)
    .optional()
    .map_err(|e| format!("ticket lookup failed ({code}): {e}"))?;
    Ok(row.map(|r| Ticket {
        id: r.id,
        code: r.code,
        open_date: r.open_date,
        close_date: r.close_date,
        stage: r.stage,
        source: r.source,
        note: r.note,
    }))
}

/// 取该票的全部生命周期（含已关闭），按 open_date 升序。
pub fn load_tickets(conn: &mut SqliteConnection, code: &str) -> Result<Vec<Ticket>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = BigInt)]
        id: i64,
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Text)]
        open_date: String,
        #[diesel(sql_type = Nullable<Text>)]
        close_date: Option<String>,
        #[diesel(sql_type = Text)]
        stage: String,
        #[diesel(sql_type = Text)]
        source: String,
        #[diesel(sql_type = Nullable<Text>)]
        note: Option<String>,
    }
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT id, code, open_date, close_date, stage, source, note \
         FROM ticket_lifecycle WHERE code = ? ORDER BY open_date ASC, id ASC",
    )
    .bind::<Text, _>(code)
    .load(conn)
    .map_err(|e| format!("tickets load failed ({code}): {e}"))?;
    Ok(rows
        .into_iter()
        .map(|r| Ticket {
            id: r.id,
            code: r.code,
            open_date: r.open_date,
            close_date: r.close_date,
            stage: r.stage,
            source: r.source,
            note: r.note,
        })
        .collect())
}

/// 取某个 ticket 的事件时间轴。
pub fn load_events(
    conn: &mut SqliteConnection,
    ticket_id: i64,
) -> Result<Vec<TicketEvent>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        event_at: String,
        #[diesel(sql_type = Text)]
        event_kind: String,
        #[diesel(sql_type = Nullable<Text>)]
        push_kind: Option<String>,
        #[diesel(sql_type = Nullable<Text>)]
        occurrence_identity: Option<String>,
        #[diesel(sql_type = Nullable<Text>)]
        summary: Option<String>,
    }
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT event_at, event_kind, push_kind, occurrence_identity, summary \
         FROM ticket_event WHERE ticket_id = ? ORDER BY event_at ASC, id ASC",
    )
    .bind::<BigInt, _>(ticket_id)
    .load(conn)
    .map_err(|e| format!("ticket events load failed (ticket={ticket_id}): {e}"))?;
    Ok(rows
        .into_iter()
        .map(|r| TicketEvent {
            event_at: r.event_at,
            event_kind: r.event_kind,
            push_kind: r.push_kind,
            occurrence_identity: r.occurrence_identity,
            summary: r.summary,
        })
        .collect())
}

/// 当前活跃的票数（运维速查）。
pub fn count_active(conn: &mut SqliteConnection) -> Result<i64, String> {
    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type = BigInt)]
        n: i64,
    }
    diesel::sql_query("SELECT COUNT(*) AS n FROM ticket_lifecycle WHERE close_date IS NULL")
        .get_result::<Count>(conn)
        .map(|row| row.n)
        .map_err(|e| format!("ticket count failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::connection::SimpleConnection;

    fn connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory sqlite");
        conn.batch_execute(
            "CREATE TABLE ticket_lifecycle (\
               id INTEGER PRIMARY KEY AUTOINCREMENT, code TEXT NOT NULL, open_date DATE NOT NULL, \
               close_date DATE, stage TEXT NOT NULL, source TEXT NOT NULL, note TEXT, \
               created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, \
               updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP);\
             CREATE UNIQUE INDEX idx_ticket_lifecycle_code_open ON ticket_lifecycle(code, open_date);\
             CREATE TABLE ticket_event (\
               id INTEGER PRIMARY KEY AUTOINCREMENT, ticket_id INTEGER NOT NULL, \
               event_at TIMESTAMP NOT NULL, event_kind TEXT NOT NULL, push_kind TEXT, \
               occurrence_identity TEXT, summary TEXT, \
               created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP);",
        )
        .expect("ticket schema");
        conn
    }

    fn scalar(conn: &mut SqliteConnection, sql: &str) -> i64 {
        #[derive(QueryableByName)]
        struct N {
            #[diesel(sql_type = BigInt)]
            n: i64,
        }
        diesel::sql_query(sql)
            .get_result::<N>(conn)
            .expect("count query")
            .n
    }

    /// B.1 回归（2026-10-22）：跨日重复入选不再产生第二张活跃票，
    /// 重复信号本身记入事件流（同日幂等），票关闭后才允许开新票。
    #[test]
    fn open_ticket_reuses_active_ticket_and_records_repeat_selection() {
        let mut conn = connection();
        let first = open_ticket(
            &mut conn,
            "603278",
            "2026-10-19",
            "observing",
            "board_rotation",
            None,
        )
        .expect("first open");
        let repeated = open_ticket(
            &mut conn,
            "603278",
            "2026-10-22",
            "observing",
            "board_rotation",
            None,
        )
        .expect("repeat open");
        assert_eq!(repeated, first, "同 code 应复用既有活跃票");
        assert_eq!(
            scalar(&mut conn, "SELECT COUNT(*) AS n FROM ticket_lifecycle"),
            1
        );
        assert_eq!(
            scalar(
                &mut conn,
                "SELECT COUNT(*) AS n FROM ticket_event WHERE event_kind = 'catalyst'"
            ),
            1,
            "重复入选应记一条 catalyst 事件"
        );

        let same_day = open_ticket(
            &mut conn,
            "603278",
            "2026-10-22",
            "observing",
            "board_rotation",
            None,
        )
        .expect("same day open");
        assert_eq!(same_day, first);
        assert_eq!(
            scalar(
                &mut conn,
                "SELECT COUNT(*) AS n FROM ticket_event WHERE event_kind = 'catalyst'"
            ),
            1,
            "同日重复入选不重复写事件"
        );

        close_ticket(&mut conn, first, "2026-10-23", "exited").expect("close");
        let reopened = open_ticket(
            &mut conn,
            "603278",
            "2026-10-26",
            "observing",
            "board_rotation",
            None,
        )
        .expect("reopen");
        assert_ne!(reopened, first, "关闭后应开新票");
        assert_eq!(
            scalar(&mut conn, "SELECT COUNT(*) AS n FROM ticket_lifecycle"),
            2
        );
    }
}
