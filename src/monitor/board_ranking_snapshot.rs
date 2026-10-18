//! 板块榜每日落库（2026-10-18）。
//!
//! ## 为什么不复用 `board_rotation_daily`
//!
//! 那张表的 `save_board_rotations` 要求 `news_title` **非空**（它的语义是"板块轮动 + 新闻标题"），
//! 而 `BoardRankingFact` 只有涨跌幅和资金流，没有新闻标题。硬塞一个板块名进去等于造假，
//! 所以另立一张表，各存各的语义。
//!
//! ## 两路排行
//!
//! - `f3`  涨幅榜（谁在涨）
//! - `f62` 主力净流入榜（钱在往哪走）
//!
//! 落库后观察池可以读"当日最强板块"、按板块成分挑票 —— 这是观察池的第二个输入
//! （第一个是新闻催化，目前上游断供）。
//!
//! ## 为什么每天都要跑
//!
//! 板块榜是当日快照，不落库就永久丢失。历史可比性（"这个板块连续三天上榜"）只能靠落库积累。

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};

use crate::data_gateway::board_ranking::BoardRankingFact;
use crate::database::DatabaseManager;

const TOP_N: usize = 30;

#[derive(Debug, Default, Clone)]
pub struct BoardReport {
    pub f3_count: usize,
    pub f62_count: usize,
    pub persisted: usize,
    pub errors: Vec<String>,
}

impl BoardReport {
    pub fn log(&self) {
        log::info!(
            "[板块榜] f3={} f62={} persisted={}",
            self.f3_count,
            self.f62_count,
            self.persisted
        );
        for error in &self.errors {
            log::warn!("[板块榜] {error}");
        }
    }
}

fn create_schema(conn: &mut SqliteConnection) -> Result<(), diesel::result::Error> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS board_ranking_daily (\
            trade_date TEXT NOT NULL,\
            rank_fid TEXT NOT NULL,\
            rank_position INTEGER NOT NULL,\
            board_code TEXT NOT NULL,\
            board_name TEXT NOT NULL,\
            change_pct REAL,\
            main_inflow REAL,\
            leader_name TEXT,\
            vol_ratio REAL,\
            turnover REAL,\
            day1_ratio REAL,\
            day5_ratio REAL,\
            fetched_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,\
            PRIMARY KEY (trade_date, rank_fid, board_code)\
        )",
    )
    .execute(conn)?;
    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS ix_board_ranking_daily_date \
         ON board_ranking_daily (trade_date, rank_fid, rank_position)",
    )
    .execute(conn)?;
    Ok(())
}

async fn fetch(fid: &str) -> Result<Vec<BoardRankingFact>, String> {
    let bridge = crate::data_gateway::grpc_source::bridge_for("BoardRanking")
        .map_err(|error| error.to_string())?;
    match bridge.board_ranking_async(fid, TOP_N).await {
        Ok(crate::data_gateway::GatewayBatch::Available { records, .. }) => Ok(records),
        Ok(crate::data_gateway::GatewayBatch::VerifiedEmpty(_)) => Ok(Vec::new()),
        Err(error) => Err(format!("fid={fid}: {error}")),
    }
}

fn persist(conn: &mut SqliteConnection, date: &str, fid: &str, facts: &[BoardRankingFact]) -> Result<usize, String> {
    let mut written = 0usize;
    for (index, fact) in facts.iter().enumerate() {
        written += diesel::sql_query(
            "INSERT INTO board_ranking_daily (\
                trade_date, rank_fid, rank_position, board_code, board_name,\
                change_pct, main_inflow, leader_name, vol_ratio, turnover,\
                day1_ratio, day5_ratio\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)\
             ON CONFLICT(trade_date, rank_fid, board_code) DO UPDATE SET \
                rank_position=excluded.rank_position,\
                board_name=excluded.board_name,\
                change_pct=excluded.change_pct,\
                main_inflow=excluded.main_inflow,\
                leader_name=excluded.leader_name,\
                vol_ratio=excluded.vol_ratio,\
                turnover=excluded.turnover,\
                day1_ratio=excluded.day1_ratio,\
                day5_ratio=excluded.day5_ratio,\
                fetched_at=CURRENT_TIMESTAMP",
        )
        .bind::<Text, _>(date)
        .bind::<Text, _>(fid)
        .bind::<BigInt, _>(index as i64)
        .bind::<Text, _>(&fact.code)
        .bind::<Text, _>(&fact.name)
        .bind::<Nullable<Double>, _>(Some(fact.change_pct))
        .bind::<Nullable<Double>, _>(Some(fact.main_inflow))
        .bind::<Nullable<Text>, _>(Some(fact.leader_name.as_str()))
        .bind::<Nullable<Double>, _>(Some(fact.vol_ratio))
        .bind::<Nullable<Double>, _>(Some(fact.turnover))
        .bind::<Nullable<Double>, _>(Some(fact.day1_ratio))
        .bind::<Nullable<Double>, _>(Some(fact.day5_ratio))
        .execute(conn)
        .map_err(|error| format!("{fid} {} 写入失败: {error}", fact.code))?;
    }
    Ok(written)
}

pub async fn refresh_board_rankings(
    db: &DatabaseManager,
    date: &str,
) -> Result<BoardReport, String> {
    let mut report = BoardReport::default();

    let f3 = match fetch("f3").await {
        Ok(rows) => {
            report.f3_count = rows.len();
            rows
        }
        Err(error) => {
            report.errors.push(format!("涨幅榜 {error}"));
            Vec::new()
        }
    };
    let f62 = match fetch("f62").await {
        Ok(rows) => {
            report.f62_count = rows.len();
            rows
        }
        Err(error) => {
            report.errors.push(format!("净流入榜 {error}"));
            Vec::new()
        }
    };

    if f3.is_empty() && f62.is_empty() {
        return Ok(report);
    }

    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    create_schema(&mut conn).map_err(|e| e.to_string())?;

    report.persisted += persist(&mut conn, date, "f3", &f3)?;
    report.persisted += persist(&mut conn, date, "f62", &f62)?;

    Ok(report)
}

/// 读当日最强板块（按涨幅），供观察池选票。
pub fn top_gaining_boards(
    conn: &mut SqliteConnection,
    date: &str,
    limit: i64,
) -> Result<Vec<(String, String, f64)>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        board_code: String,
        #[diesel(sql_type = Text)]
        board_name: String,
        #[diesel(sql_type = Nullable<Double>)]
        change_pct: Option<f64>,
    }
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT board_code, board_name, change_pct \
         FROM board_ranking_daily \
         WHERE trade_date = ? AND rank_fid = 'f3' AND change_pct IS NOT NULL \
         ORDER BY change_pct DESC LIMIT ?",
    )
    .bind::<Text, _>(date)
    .bind::<BigInt, _>(limit)
    .load(conn)
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| (row.board_code, row.board_name, row.change_pct.unwrap_or_default()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_n_is_bounded() {
        assert!(TOP_N > 0 && TOP_N <= 100);
    }
}
