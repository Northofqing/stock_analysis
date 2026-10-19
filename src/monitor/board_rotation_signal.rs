//! P5 板块轮动（2026-10-19）。
//!
//! 回答「今天这个板块是主线、还是刚接力、还是在退潮」。
//!
//! ## 输入
//!
//! `board_ranking_daily`（`rank_fid='f3'`，市场概念板块按涨幅排序）。
//! **至少需要两个交易日**才能比较，单日无法判断轮动。
//!
//! ## 三种状态
//!
//! - `mainline` 上一个交易日在 top N、今天还在 top N → 主线延续
//! - `new`      今天进 top N、上一交易日不在 → 接力候选
//! - `fading`   上一交易日在 top N、今天掉出 → 退潮
//!
//! ## 为什么单独建表
//!
//! `board_rotation_daily` 是旧链路（要求 `news_title` 非空），它把
//! 「板块 + 新闻标题」绑在一起写入。本模块只做纯排名对比，没有新闻标题，
//! 硬塞空串等于造假（BR-222）。所以另立 `board_rotation_signal`。
//!
//! ## 边界
//!
//! 板块轮动的「早期信号」本质滞后：看到连续两天在榜时，行情往往走了一半。
//! 本模块只做提示，不构成买入依据。

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Integer, Nullable, Text};

use crate::database::DatabaseManager;

/// 判定「在榜」的名次上限。
pub const TOP_N: i64 = 10;

/// 建表（幂等）。
pub fn create_schema(conn: &mut SqliteConnection) -> Result<(), diesel::result::Error> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS board_rotation_signal (\
         trade_date DATE NOT NULL,\
         prev_date DATE,\
         board_code TEXT NOT NULL,\
         board_name TEXT NOT NULL,\
         status TEXT NOT NULL,\
         rank_today INTEGER,\
         rank_prev INTEGER,\
         pct_today REAL,\
         pct_prev REAL,\
         created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,\
         PRIMARY KEY (trade_date, board_code))",
    )
    .execute(conn)
    .map(|_| ())
}

#[derive(Debug, Clone)]
pub struct RotationRow {
    pub board_code: String,
    pub board_name: String,
    pub status: String,
    pub rank_today: Option<i64>,
    pub rank_prev: Option<i64>,
    pub pct_today: Option<f64>,
    pub pct_prev: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct BuildResult {
    pub trade_date: String,
    pub prev_date: Option<String>,
    pub mainline: usize,
    pub new: usize,
    pub fading: usize,
}

/// 找到 `trade_date` 之前最近一个有板块榜数据的交易日。
fn previous_trade_date(
    conn: &mut SqliteConnection,
    trade_date: &str,
) -> Result<Option<String>, String> {
    #[derive(QueryableByName)]
    struct DateRow {
        #[diesel(sql_type = Text)]
        d: String,
    }
    let rows: Vec<DateRow> = diesel::sql_query(
        "SELECT DISTINCT trade_date AS d FROM board_ranking_daily \
          WHERE rank_fid = 'f3' AND trade_date < ? \
          ORDER BY trade_date DESC LIMIT 1",
    )
    .bind::<Text, _>(trade_date.to_owned())
    .load::<DateRow>(conn)
    .map_err(|e| format!("上一交易日查询失败: {e}"))?;
    Ok(rows.into_iter().next().map(|row| row.d))
}

#[derive(QueryableByName)]
struct RankRow {
    #[diesel(sql_type = Text)]
    board_code: String,
    #[diesel(sql_type = Text)]
    board_name: String,
    #[diesel(sql_type = BigInt)]
    rank_position: i64,
    #[diesel(sql_type = Nullable<Double>)]
    change_pct: Option<f64>,
}

fn load_ranking(conn: &mut SqliteConnection, trade_date: &str) -> Result<Vec<RankRow>, String> {
    diesel::sql_query(
        "SELECT board_code, board_name, rank_position, change_pct \
           FROM board_ranking_daily \
          WHERE rank_fid = 'f3' AND trade_date = ? \
          ORDER BY rank_position",
    )
    .bind::<Text, _>(trade_date.to_owned())
    .load::<RankRow>(conn)
    .map_err(|e| format!("板块榜读取失败({trade_date}): {e}"))
}

/// 生成某日的轮动信号。
pub fn build(db: &DatabaseManager, trade_date: &str) -> Result<BuildResult, String> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    create_schema(&mut conn).map_err(|e| e.to_string())?;

    let today = load_ranking(&mut conn, trade_date)?;
    if today.is_empty() {
        return Err(format!("{trade_date} 没有板块榜数据"));
    }
    let prev_date = previous_trade_date(&mut conn, trade_date)?;
    let prev = match prev_date.as_deref() {
        Some(day) => load_ranking(&mut conn, day)?,
        None => Vec::new(),
    };

    // 只用名次做判定：涨跌幅可能因复权/口径差异不可比。
    let prev_top: std::collections::HashMap<String, (String, i64, Option<f64>)> = prev
        .iter()
        .filter(|row| row.rank_position < TOP_N)
        .map(|row| {
            (
                row.board_code.clone(),
                (row.board_name.clone(), row.rank_position, row.change_pct),
            )
        })
        .collect();
    let today_top: std::collections::HashSet<String> = today
        .iter()
        .filter(|row| row.rank_position < TOP_N)
        .map(|row| row.board_code.clone())
        .collect();

    let mut rows: Vec<RotationRow> = Vec::new();

    for row in &today {
        let in_today_top = row.rank_position < TOP_N;
        let prev_hit = prev_top.get(&row.board_code);
        let status = match (in_today_top, prev_hit.is_some()) {
            (true, true) => "mainline",
            (true, false) => "new",
            (false, true) => "fading",
            (false, false) => continue, // 两天都不在榜，不记
        };
        rows.push(RotationRow {
            board_code: row.board_code.clone(),
            board_name: row.board_name.clone(),
            status: status.to_owned(),
            rank_today: Some(row.rank_position),
            rank_prev: prev_hit.map(|(_, rank, _)| *rank),
            pct_today: row.change_pct,
            pct_prev: prev_hit.and_then(|(_, _, pct)| *pct),
        });
    }

    // 昨天在榜、今天整行都消失的板块（数据缺失不算退潮，只有确认不在榜才算）。
    for (code, (name, rank, pct)) in &prev_top {
        if today_top.contains(code) {
            continue;
        }
        if rows.iter().any(|row| &row.board_code == code) {
            continue; // 上面已经记成 fading
        }
        rows.push(RotationRow {
            board_code: code.clone(),
            board_name: name.clone(),
            status: "fading".to_owned(),
            rank_today: None,
            rank_prev: Some(*rank),
            pct_today: None,
            pct_prev: *pct,
        });
    }

    let mut mainline = 0usize;
    let mut fresh = 0usize;
    let mut fading = 0usize;
    for row in &rows {
        match row.status.as_str() {
            "mainline" => mainline += 1,
            "new" => fresh += 1,
            "fading" => fading += 1,
            _ => {}
        }

        diesel::sql_query(
            "INSERT INTO board_rotation_signal \
              (trade_date, prev_date, board_code, board_name, status, \
               rank_today, rank_prev, pct_today, pct_prev) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(trade_date, board_code) DO UPDATE SET \
               prev_date = excluded.prev_date, \
               status = excluded.status, \
               rank_today = excluded.rank_today, \
               rank_prev = excluded.rank_prev, \
               pct_today = excluded.pct_today, \
               pct_prev = excluded.pct_prev",
        )
        .bind::<Text, _>(trade_date.to_owned())
        .bind::<Nullable<Text>, _>(prev_date.clone())
        .bind::<Text, _>(row.board_code.clone())
        .bind::<Text, _>(row.board_name.clone())
        .bind::<Text, _>(row.status.clone())
        .bind::<Nullable<BigInt>, _>(row.rank_today)
        .bind::<Nullable<BigInt>, _>(row.rank_prev)
        .bind::<Nullable<Double>, _>(row.pct_today)
        .bind::<Nullable<Double>, _>(row.pct_prev)
        .execute(&mut conn)
        .map_err(|e| format!("写入 {} 失败: {e}", row.board_code))?;
    }

    Ok(BuildResult {
        trade_date: trade_date.to_owned(),
        prev_date,
        mainline,
        new: fresh,
        fading,
    })
}

/// 读回某日的轮动信号（接力优先，其次主线，最后退潮）。
pub fn show(db: &DatabaseManager, trade_date: &str) -> Result<Vec<RotationRow>, String> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    create_schema(&mut conn).map_err(|e| e.to_string())?;

    #[derive(QueryableByName)]
    struct DbRow {
        #[diesel(sql_type = Text)]
        board_code: String,
        #[diesel(sql_type = Text)]
        board_name: String,
        #[diesel(sql_type = Text)]
        status: String,
        #[diesel(sql_type = Nullable<BigInt>)]
        rank_today: Option<i64>,
        #[diesel(sql_type = Nullable<BigInt>)]
        rank_prev: Option<i64>,
        #[diesel(sql_type = Nullable<Double>)]
        pct_today: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        pct_prev: Option<f64>,
    }

    let rows: Vec<DbRow> = diesel::sql_query(
        "SELECT board_code, board_name, status, rank_today, rank_prev, pct_today, pct_prev \
           FROM board_rotation_signal WHERE trade_date = ? \
          ORDER BY CASE status WHEN 'new' THEN 0 WHEN 'mainline' THEN 1 ELSE 2 END, \
                   rank_today IS NULL, rank_today, board_code",
    )
    .bind::<Text, _>(trade_date.to_owned())
    .load::<DbRow>(&mut conn)
    .map_err(|e| format!("轮动信号读取失败: {e}"))?;

    Ok(rows
        .into_iter()
        .map(|row| RotationRow {
            board_code: row.board_code,
            board_name: row.board_name,
            status: row.status,
            rank_today: row.rank_today,
            rank_prev: row.rank_prev,
            pct_today: row.pct_today,
            pct_prev: row.pct_prev,
        })
        .collect())
}

/// 供 CLI 显示的「两天都有数据」判断。
pub fn has_comparable_days(db: &DatabaseManager, trade_date: &str) -> Result<bool, String> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        n: i64,
    }
    let rows: Vec<CountRow> = diesel::sql_query(
        "SELECT COUNT(DISTINCT trade_date) AS n FROM board_ranking_daily \
          WHERE rank_fid = 'f3' AND trade_date <= ?",
    )
    .bind::<Text, _>(trade_date.to_owned())
    .load::<CountRow>(&mut conn)
    .map_err(|e| format!("可比日查询失败: {e}"))?;
    Ok(rows.first().map(|row| row.n >= 2).unwrap_or(false))
}
