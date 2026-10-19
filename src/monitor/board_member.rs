//! 板块 ← 成分股 反查表（2026-10-19）。
//!
//! ## 为什么需要它
//!
//! 数据源只提供「票 → 板块」（`BoardConstituents` / `BoardMemberships` 都是这个方向），
//! 没有「板块 → 成分股」。但 P5 板块轮动要做「识别到接力的板块 → 找出该板块的票」，
//! 需要反方向查。所以把全市场每只票的正向归属拉一遍，反向建成一张表。
//!
//! ## 数据来源
//!
//! `board_constituents_async(code)`，Mac LocalBridge 实测可用：
//! `batch=tdx-block-files:memberships:unix-ms:...`，单票返回 8-26 条归属。
//!
//! ## 边界
//!
//! - 这是**反向索引**，不是权威成分。数据源更新后要重跑（`--codes` 可增量）。
//! - `kind` 区分 Industry / Concept / Region；板块榜用的是概念板块，
//!   所以查成分时按 `kind='concept'` 过滤。
//! - 一只票可能同时属于几十个板块，行数会是票数的 10-30 倍，属正常。

use diesel::prelude::*;
use diesel::sql_types::Text;

use crate::data_gateway::grpc_source::bridge_for;
use crate::data_gateway::{BoardKind, GatewayBatch};
use crate::database::DatabaseManager;

/// 建表（幂等）。
pub fn create_schema(conn: &mut SqliteConnection) -> Result<(), diesel::result::Error> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS board_member (\
         board_code TEXT NOT NULL,\
         board_name TEXT NOT NULL,\
         kind TEXT NOT NULL,\
         instrument_code TEXT NOT NULL,\
         updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,\
         PRIMARY KEY (board_code, instrument_code))",
    )
    .execute(conn)
    .map(|_| ())
}

fn kind_label(kind: BoardKind) -> &'static str {
    match kind {
        BoardKind::Industry => "industry",
        BoardKind::Concept => "concept",
        BoardKind::Region => "region",
    }
}

#[derive(Debug, Clone, Default)]
pub struct BuildReport {
    pub requested: usize,
    pub fetched: usize,
    pub empty: usize,
    pub failed: usize,
    pub rows: usize,
    pub boards: usize,
}

impl BuildReport {
    pub fn log(&self) {
        log::info!(
            "[板块成分] requested={} fetched={} empty={} failed={} rows={} boards={}",
            self.requested,
            self.fetched,
            self.empty,
            self.failed,
            self.rows,
            self.boards
        );
    }
}

/// 拉一批票的板块归属并写入 `board_member`。
///
/// 幂等：同一 `(board_code, instrument_code)` 覆盖 `board_name` / `kind` / `updated_at`。
///
/// 按 `chunk` 分批调用——底层 op 收 codes 数组，逐只调用每只约 16 秒
/// （每次重读 TDX block 文件），全市场 1265 只逐只跑要 5 小时。
pub async fn build(
    db: &DatabaseManager,
    codes: &[String],
    chunk: usize,
) -> Result<BuildReport, String> {
    let mut report = BuildReport {
        requested: codes.len(),
        ..Default::default()
    };
    if codes.is_empty() {
        return Ok(report);
    }

    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    create_schema(&mut conn).map_err(|e| e.to_string())?;

    let bridge = bridge_for("BoardConstituents").map_err(|e| e.to_string())?;
    let chunk = chunk.max(1);

    for (index, batch) in codes.chunks(chunk).enumerate() {
        let started = std::time::Instant::now();
        match bridge.board_constituents_many_async(batch).await {
            Ok(GatewayBatch::Available { records, .. }) => {
                let mut covered = std::collections::HashSet::new();
                for record in &records {
                    covered.insert(record.instrument_code.clone());
                    let kind = kind_label(record.kind);
                    diesel::sql_query(
                        "INSERT INTO board_member \
                          (board_code, board_name, kind, instrument_code) \
                         VALUES (?, ?, ?, ?) \
                         ON CONFLICT(board_code, instrument_code) DO UPDATE SET \
                           board_name = excluded.board_name, \
                           kind = excluded.kind, \
                           updated_at = CURRENT_TIMESTAMP",
                    )
                    .bind::<Text, _>(record.board_code.clone())
                    .bind::<Text, _>(record.board_name.clone())
                    .bind::<Text, _>(kind.to_owned())
                    .bind::<Text, _>(record.instrument_code.clone())
                    .execute(&mut conn)
                    .map_err(|e| format!("写入 {} 失败: {e}", record.board_code))?;
                    report.rows += 1;
                }
                report.fetched += covered.len();
                report.empty += batch.len().saturating_sub(covered.len());
            }
            Ok(GatewayBatch::VerifiedEmpty(_)) => {
                report.empty += batch.len();
            }
            Err(error) => {
                report.failed += batch.len();
                log::warn!(
                    "[板块成分] 批次 {index} ({} 只) 失败: {error}",
                    batch.len()
                );
                continue;
            }
        }
        log::info!(
            "[板块成分] 批次 {index} {} 只 用时 {:?} 累计 rows={}",
            batch.len(),
            started.elapsed(),
            report.rows
        );
    }

    // 统计板块数
    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        n: i64,
    }
    report.boards = diesel::sql_query("SELECT COUNT(DISTINCT board_code) AS n FROM board_member")
        .load::<CountRow>(&mut conn)
        .map(|rows| rows.into_iter().next().map(|r| r.n).unwrap_or(0))
        .unwrap_or(0) as usize;

    Ok(report)
}

/// 按板块代码查成分股（P5 用这个）。
pub fn members_of(
    db: &DatabaseManager,
    board_code: &str,
    kind: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    create_schema(&mut conn).map_err(|e| e.to_string())?;

    #[derive(QueryableByName)]
    struct CodeRow {
        #[diesel(sql_type = Text)]
        instrument_code: String,
    }

    let rows: Vec<CodeRow> = match kind {
        Some(k) => diesel::sql_query(
            "SELECT instrument_code FROM board_member \
              WHERE board_code = ? AND kind = ? ORDER BY instrument_code",
        )
        .bind::<Text, _>(board_code.to_owned())
        .bind::<Text, _>(k.to_owned())
        .load(&mut conn),
        None => diesel::sql_query(
            "SELECT instrument_code FROM board_member \
              WHERE board_code = ? ORDER BY instrument_code",
        )
        .bind::<Text, _>(board_code.to_owned())
        .load(&mut conn),
    }
    .map_err(|e| format!("查成分失败: {e}"))?;

    Ok(rows.into_iter().map(|row| row.instrument_code).collect())
}

/// 当日板块榜的板块条目。
#[derive(Debug, Clone)]
pub struct RankingBoard {
    pub rank_position: i32,
    pub board_name: String,
    pub change_pct: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct RankingBuildReport {
    pub boards_total: usize,
    pub boards_ok: usize,
    pub boards_failed: usize,
    pub rows: usize,
    /// (板块名, 解析出的 board_code, 成分股数)
    pub per_board: Vec<(String, String, usize)>,
    pub failures: Vec<String>,
}

impl RankingBuildReport {
    pub fn log(&self) {
        log::info!(
            "[板块成分] 榜单模式 boards={} ok={} failed={} rows={}",
            self.boards_total,
            self.boards_ok,
            self.boards_failed,
            self.rows
        );
        for (name, code, count) in &self.per_board {
            log::info!("[板块成分]   {name} → {code} {count} 只");
        }
        for failure in &self.failures {
            log::warn!("[板块成分] 失败: {failure}");
        }
    }
}

/// 读当日板块榜前 N 名的板块（按 rank_position）。
pub fn load_ranking_boards(
    db: &DatabaseManager,
    trade_date: &str,
    top_n: usize,
) -> Result<Vec<RankingBoard>, String> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;

    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        rank_position: i32,
        #[diesel(sql_type = Text)]
        board_name: String,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
        change_pct: Option<f64>,
    }

    let rows: Vec<Row> = diesel::sql_query(
        "SELECT MIN(rank_position) AS rank_position, board_name, MAX(change_pct) AS change_pct \
           FROM board_ranking_daily \
          WHERE trade_date = ?1 AND rank_position < ?2 \
          GROUP BY board_name ORDER BY rank_position",
    )
    .bind::<Text, _>(trade_date.to_owned())
    .bind::<diesel::sql_types::Integer, _>(top_n as i32)
    .load(&mut conn)
    .map_err(|e| format!("读板块榜失败: {e}"))?;

    Ok(rows
        .into_iter()
        .map(|row| RankingBoard {
            rank_position: row.rank_position,
            board_name: row.board_name,
            change_pct: row.change_pct,
        })
        .collect())
}

/// 按榜单查成分股并落 `board_member`。
///
/// 板块名来自新浪概念榜（`gn_*` 口径），TDX 的板块名不一定一一对应，
/// 且概念/行业分类可能不同——所以每个名字先试 `tdx:concept:`，失败再试
/// `tdx:industry:`。两者都失败就记进 `failures`，不静默跳过。
pub async fn build_from_ranking(
    db: &DatabaseManager,
    trade_date: &str,
    top_n: usize,
    limit: u32,
) -> Result<RankingBuildReport, String> {
    let boards = load_ranking_boards(db, trade_date, top_n)?;
    let mut report = RankingBuildReport {
        boards_total: boards.len(),
        ..Default::default()
    };
    if boards.is_empty() {
        return Ok(report);
    }

    let bridge = bridge_for("BoardConstituents").map_err(|e| e.to_string())?;
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    create_schema(&mut conn).map_err(|e| e.to_string())?;

    for board in &boards {
        let candidates = [
            (format!("tdx:concept:{}", board.board_name), "concept"),
            (format!("tdx:industry:{}", board.board_name), "industry"),
        ];
        let mut landed = None;
        let mut last_error = String::new();

        for (board_code, kind) in candidates {
            match bridge.board_constituents_by_board_async(&board_code, limit).await {
                Ok(GatewayBatch::Available { records, .. }) if !records.is_empty() => {
                    let mut written = 0usize;
                    for record in &records {
                        diesel::sql_query(
                            "INSERT INTO board_member \
                              (board_code, board_name, kind, instrument_code) \
                             VALUES (?, ?, ?, ?) \
                             ON CONFLICT(board_code, instrument_code) DO UPDATE SET \
                               board_name = excluded.board_name, \
                               kind = excluded.kind, \
                               updated_at = CURRENT_TIMESTAMP",
                        )
                        .bind::<Text, _>(board_code.clone())
                        .bind::<Text, _>(board.board_name.clone())
                        .bind::<Text, _>(kind.to_owned())
                        .bind::<Text, _>(record.instrument_code.clone())
                        .execute(&mut conn)
                        .map_err(|e| format!("写入 {board_code} 失败: {e}"))?;
                        written += 1;
                    }
                    landed = Some((board_code, written));
                    break;
                }
                Ok(GatewayBatch::Available { .. }) | Ok(GatewayBatch::VerifiedEmpty(_)) => {
                    last_error = format!("{board_code} 返回空");
                }
                Err(error) => {
                    last_error = format!("{board_code}: {error}");
                }
            }
        }

        match landed {
            Some((board_code, count)) => {
                report.boards_ok += 1;
                report.rows += count;
                report.per_board.push((
                    board.board_name.clone(),
                    board_code,
                    count,
                ));
            }
            None => {
                report.boards_failed += 1;
                report.failures.push(format!(
                    "#{} {} → {last_error}",
                    board.rank_position + 1,
                    board.board_name
                ));
            }
        }
    }

    Ok(report)
}
