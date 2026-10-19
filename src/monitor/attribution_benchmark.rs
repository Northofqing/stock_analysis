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
//! - `board_pct`   板块涨跌幅 —— 数据不足，留 NULL
//!
//! ## 两条硬约束
//!
//! 1. **BR-222 不补 0**：样本不足时 `market_pct` / `excess_pct` 写 NULL，
//!    不拿 20 只票的平均冒充「全市场」。
//! 2. **板块留空**：`board_ranking_daily` 只有 10-16 一天的新浪概念快照，
//!    且无法按票精确归属（D3 未落地）。填一个估出来的板块涨幅比 NULL 更糟。

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};

use crate::database::DatabaseManager;

/// 市场基准所需的最少样本数。
pub const MIN_BENCHMARK_SAMPLE: i64 = 50;

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

    let benchmark_label = if benchmark_accepted {
        format!("equal_weight_all_market(n={sample_size})")
    } else {
        format!("insufficient_sample(n={sample_size},min={MIN_BENCHMARK_SAMPLE})")
    };

    let mut written = 0usize;
    let mut with_benchmark = 0usize;

    for bar in &bars {
        let excess = match (bar.pct_chg, market_pct) {
            (Some(stock), Some(market)) => Some(stock - market),
            _ => None,
        };
        if excess.is_some() {
            with_benchmark += 1;
        }

        diesel::sql_query(
            "INSERT INTO attribution_benchmark_daily \
              (trade_date, code, stock_pct, market_pct, board_pct, excess_pct, sample_size, benchmark) \
             VALUES (?, ?, ?, ?, NULL, ?, ?, ?) \
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
        .bind::<Nullable<Double>, _>(excess)
        .bind::<BigInt, _>(sample_size)
        .bind::<Text, _>(benchmark_label.clone())
        .execute(&mut conn)
        .map_err(|e| format!("写入 {} 失败: {e}", bar.code))?;

        written += 1;
    }

    Ok(BuildResult {
        trade_date: trade_date.to_owned(),
        rows: written,
        with_benchmark,
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
