//! P6 二层归因（2026-10-19）。
//!
//! 补上「个股 vs 市场」这一层拆分。现有的 `attribution.rs` 是**事件归因**
//! （异动 → 链上哪家公司），`paper_attribution_daily` 是**按信号族的盈亏汇总**，
//! 两者都回答不了「这只票今天涨 3%，是我选对了还是大盘带的」。
//!
//! ## 输出
//!
//! - `stock_pct`   个股当日涨跌幅（`stock_daily.pct_chg`）
//! - `market_pct`  全市场等权平均（样本 >= 50 只才算基准，否则 NULL）
//! - `excess_pct`  `stock_pct - market_pct`，选股超额
//! - `board_pct`   板块等权成员涨幅（同交易日 `board_member_snapshot` 成分 +
//!                 `stock_daily.pct_chg`；成员数少于此值的板块不参与）
//!
//! ## 三条硬约束
//!
//! 1. **BR-222 不补 0**：样本不足时 `market_pct` / `excess_pct` / `board_pct`
//!    写 NULL，不拿 20 只票的平均冒充「全市场」。
//! 2. **板块只做收盘等权聚合**：不用盘中 `board_ranking_daily` 快照冒充收盘；
//!    板块无当日成分或成员样本不足时留 NULL。
//! 3. **板块口径可复核**：`benchmark` 注明 `board=equal_weight_members(...)`，
//!    `board_pct` 是成分股收盘涨跌幅的等权平均，不是交易所板块指数。

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};
use std::collections::HashMap;

use crate::database::DatabaseManager;

/// 市场基准所需的最少样本数。
pub const MIN_BENCHMARK_SAMPLE: i64 = 50;

/// 板块等权基准所需的最少成分股数；低于此值留 NULL。
pub const MIN_BOARD_MEMBER_SAMPLE: i64 = 5;

/// 建表（幂等）。
pub fn create_schema(conn: &mut SqliteConnection) -> Result<(), diesel::result::Error> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS attribution_benchmark_daily (\
         trade_date DATE NOT NULL,\
         code TEXT NOT NULL,\
         stock_pct REAL,\
         market_pct REAL,\
         board_pct REAL,\
         excess_pct REAL,\
         sample_size INTEGER,\
         benchmark TEXT,\
         created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,\
         PRIMARY KEY (trade_date, code))",
    )
    .execute(conn)
    .map(|_| ())
}

#[derive(Debug, Clone)]
pub struct BuildResult {
    pub trade_date: String,
    pub rows: usize,
    pub with_benchmark: usize,
    pub with_board: usize,
    pub sample_size: i64,
    pub benchmark_accepted: bool,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub code: String,
    pub stock_pct: Option<f64>,
    pub market_pct: Option<f64>,
    pub board_pct: Option<f64>,
    pub excess_pct: Option<f64>,
    pub benchmark: Option<String>,
}

/// 读同交易日板块成分的收盘等权涨幅；没有成分快照或样本不足返回空表。
fn load_board_pcts(conn: &mut SqliteConnection, trade_date: &str) -> HashMap<String, f64> {
    #[derive(QueryableByName)]
    struct BoardRow {
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Nullable<Double>)]
        board_pct: Option<f64>,
    }

    match diesel::sql_query(
        "SELECT m.instrument_code AS code, AVG(board.bp) AS board_pct \
           FROM board_member_snapshot m \
           JOIN ( \
                 SELECT ms.board_name AS board_name, \
                        AVG(s.pct_chg) AS bp, \
                        COUNT(DISTINCT s.code) AS n \
                   FROM board_member_snapshot ms \
                   JOIN stock_daily s \
                     ON s.code = ms.instrument_code AND s.date = ? \
                  WHERE ms.trade_date = ? AND s.pct_chg IS NOT NULL \
                  GROUP BY ms.board_name \
                 HAVING COUNT(DISTINCT s.code) >= ? \
                ) board ON board.board_name = m.board_name \
          WHERE m.trade_date = ? \
          GROUP BY m.instrument_code",
    )
    .bind::<Text, _>(trade_date)
    .bind::<Text, _>(trade_date)
    .bind::<BigInt, _>(MIN_BOARD_MEMBER_SAMPLE)
    .bind::<Text, _>(trade_date)
    .load::<BoardRow>(conn)
    {
        Ok(rows) => rows
            .into_iter()
            .filter_map(|row| row.board_pct.map(|value| (row.code, value)))
            .collect(),
        Err(error) => {
            log::warn!("[归因基准] board_member_snapshot 不可用, board_pct 留 NULL: {error}");
            HashMap::new()
        }
    }
}

/// 为某个交易日生成「个股 vs 市场」归因。
pub fn build(db: &DatabaseManager, trade_date: &str) -> Result<BuildResult, String> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    create_schema(&mut conn).map_err(|e| e.to_string())?;

    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        n: i64,
    }

    let sample_size: i64 =
        diesel::sql_query("SELECT COUNT(*) AS n FROM stock_daily WHERE date = ?")
            .bind::<Text, _>(trade_date)
            .load::<CountRow>(&mut conn)
            .map_err(|e| format!("样本量查询失败: {e}"))?
            .into_iter()
            .next()
            .map(|row| row.n)
            .unwrap_or(0);

    if sample_size == 0 {
        return Err(format!("{trade_date} 没有日线数据，无法归因"));
    }

    let benchmark_accepted = sample_size >= MIN_BENCHMARK_SAMPLE;

    let market_pct: Option<f64> = if benchmark_accepted {
        #[derive(QueryableByName)]
        struct AvgRow {
            #[diesel(sql_type = Nullable<Double>)]
            avg: Option<f64>,
        }
        diesel::sql_query("SELECT AVG(pct_chg) AS avg FROM stock_daily WHERE date = ?")
            .bind::<Text, _>(trade_date)
            .load::<AvgRow>(&mut conn)
            .map_err(|e| format!("基准计算失败: {e}"))?
            .into_iter()
            .next()
            .and_then(|row| row.avg)
    } else {
        None
    };

    // 板块基准：同交易日成分 + 收盘涨跌幅做等权。只用当日快照，避免未来成分；
    // 不用盘中 board_ranking_daily 的 change_pct 冒充收盘。
    let board_pcts = load_board_pcts(&mut conn, trade_date);

    #[derive(QueryableByName)]
    struct BarRow {
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Nullable<Double>)]
        pct_chg: Option<f64>,
    }

    let bars: Vec<BarRow> =
        diesel::sql_query("SELECT code, pct_chg FROM stock_daily WHERE date = ? ORDER BY code")
            .bind::<Text, _>(trade_date)
            .load::<BarRow>(&mut conn)
            .map_err(|e| format!("日线读取失败: {e}"))?;

    let mut benchmark_label = if benchmark_accepted {
        format!("equal_weight_all_market(n={sample_size})")
    } else {
        format!("insufficient_sample(n={sample_size},min={MIN_BENCHMARK_SAMPLE})")
    };
    if !board_pcts.is_empty() {
        benchmark_label.push_str(&format!(
            ";board=equal_weight_members(min={MIN_BOARD_MEMBER_SAMPLE},codes={})",
            board_pcts.len()
        ));
    }

    let mut with_benchmark = 0usize;
    let mut with_board = 0usize;

    // 5k+ 行逐行 autocommit 在 5GB 生产库上会拖到几分钟；整日归因必须一个事务。
    let written = conn
        .transaction::<usize, diesel::result::Error, _>(|conn| {
            let mut written = 0usize;
            for bar in &bars {
                let board_pct = board_pcts.get(&bar.code).copied();
                let excess = match (bar.pct_chg, market_pct) {
                    (Some(stock), Some(market)) => Some(stock - market),
                    _ => None,
                };
                if excess.is_some() {
                    with_benchmark += 1;
                }
                if board_pct.is_some() {
                    with_board += 1;
                }

                diesel::sql_query(
                    "INSERT INTO attribution_benchmark_daily \
                      (trade_date, code, stock_pct, market_pct, board_pct, excess_pct, sample_size, benchmark) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
                     ON CONFLICT(trade_date, code) DO UPDATE SET \
                       stock_pct = excluded.stock_pct, \
                       market_pct = excluded.market_pct, \
                       board_pct = excluded.board_pct, \
                       excess_pct = excluded.excess_pct, \
                       sample_size = excluded.sample_size, \
                       benchmark = excluded.benchmark",
                )
                .bind::<Text, _>(trade_date)
                .bind::<Text, _>(bar.code.clone())
                .bind::<Nullable<Double>, _>(bar.pct_chg)
                .bind::<Nullable<Double>, _>(market_pct)
                .bind::<Nullable<Double>, _>(board_pct)
                .bind::<Nullable<Double>, _>(excess)
                .bind::<BigInt, _>(sample_size)
                .bind::<Text, _>(benchmark_label.clone())
                .execute(conn)?;

                written += 1;
            }
            Ok(written)
        })
        .map_err(|e| format!("归因写入事务失败: {e}"))?;

    Ok(BuildResult {
        trade_date: trade_date.to_owned(),
        rows: written,
        with_benchmark,
        with_board,
        sample_size,
        benchmark_accepted,
    })
}

/// 读回某日归因（超额降序，NULL 排最后）。
pub fn show(db: &DatabaseManager, trade_date: &str) -> Result<Vec<Row>, String> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    create_schema(&mut conn).map_err(|e| e.to_string())?;

    #[derive(QueryableByName)]
    struct DbRow {
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Nullable<Double>)]
        stock_pct: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        market_pct: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        board_pct: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        excess_pct: Option<f64>,
        #[diesel(sql_type = Nullable<Text>)]
        benchmark: Option<String>,
    }

    let rows: Vec<DbRow> = diesel::sql_query(
        "SELECT code, stock_pct, market_pct, board_pct, excess_pct, benchmark \
           FROM attribution_benchmark_daily WHERE trade_date = ? \
          ORDER BY excess_pct IS NULL, excess_pct DESC, code",
    )
    .bind::<Text, _>(trade_date)
    .load::<DbRow>(&mut conn)
    .map_err(|e| format!("归因读取失败: {e}"))?;

    Ok(rows
        .into_iter()
        .map(|row| Row {
            code: row.code,
            stock_pct: row.stock_pct,
            market_pct: row.market_pct,
            board_pct: row.board_pct,
            excess_pct: row.excess_pct,
            benchmark: row.benchmark,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::connection::SimpleConnection;
    use diesel::Connection;

    fn insert_bar(conn: &mut SqliteConnection, code: &str, pct: f64) {
        diesel::sql_query(
            "INSERT INTO stock_daily (date, code, pct_chg) VALUES ('2026-10-27', ?, ?)",
        )
        .bind::<Text, _>(code)
        .bind::<Double, _>(pct)
        .execute(conn)
        .expect("insert bar");
    }

    fn insert_member(conn: &mut SqliteConnection, board: &str, code: &str) {
        diesel::sql_query(
            "INSERT INTO board_member_snapshot (board_name, instrument_code, trade_date) \
             VALUES (?, ?, '2026-10-27')",
        )
        .bind::<Text, _>(board)
        .bind::<Text, _>(code)
        .execute(conn)
        .expect("insert member");
    }

    #[test]
    fn board_pct_is_equal_weight_of_same_day_members() {
        let mut conn = SqliteConnection::establish(":memory:").expect("memory db");
        conn.batch_execute(
            "CREATE TABLE stock_daily (date TEXT, code TEXT, pct_chg REAL);
             CREATE TABLE board_member_snapshot (
                 board_name TEXT, instrument_code TEXT, trade_date TEXT);",
        )
        .expect("schema");

        let mut members = Vec::new();
        for (index, pct) in [1.0, 2.0, 3.0, 4.0, 5.0].into_iter().enumerate() {
            let code = format!("00000{}", index + 1);
            insert_bar(&mut conn, &code, pct);
            insert_member(&mut conn, "测试板块", &code);
            members.push(code);
        }
        // 只有 1 只成分的板块不参与，不能拿单票涨幅冒充板块。
        insert_bar(&mut conn, "999999", 100.0);
        insert_member(&mut conn, "单票板块", "999999");

        let board_pcts = load_board_pcts(&mut conn, "2026-10-27");
        assert_eq!(board_pcts.len(), members.len());
        for code in members {
            let value = board_pcts.get(&code).copied().expect("board pct");
            assert!((value - 3.0).abs() < 1e-9, "{code} board_pct={value}");
        }
        assert!(!board_pcts.contains_key("999999"));
    }
}
