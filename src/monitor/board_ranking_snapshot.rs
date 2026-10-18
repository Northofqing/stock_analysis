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
use encoding_rs::GBK;

use crate::data_gateway::board_ranking::BoardRankingFact;
use crate::database::DatabaseManager;

const TOP_N: usize = 30;

/// 新浪板块 —— 东财 clist 不可用时的 f3 备用源（2026-10-18 实测 200）。
///
/// 为什么需要它：东财 `push2.eastmoney.com/api/qt/clist/get` 从本机返回 502
/// （同一 host 的 `stock/get` 却是 200），而新浪这两个接口稳定返回真实板块报价。
///
/// **只拉概念板块**（2026-10-18 实测依据）：
///
/// - 概念板块 `newFLJK.php?param=class` → `gn_hwqc,华为汽车` …
///   175 个板块里 98 个能对上 `stock_concepts`（601811 命中百度概念/参股金融/含H股/体育概念）
/// - 行业板块 `newSinaHy.php` → `new_fjzz,飞机制造` …
///   行业名在 `stock_concepts` 里 **0 匹配**（逐名验证过），
///   拉进来只会挤占 top30 名额，所以不用。
///
/// 返回格式：GBK 编码的 JS 变量，每行 13 个逗号分隔字段：
///
///     0 板块代码      1 板块名称    2 公司数      3 均价       4 涨跌额
///     5 涨跌幅(%)     6 成交量      7 成交额      8 领涨股代码 9 领涨股涨跌幅
///     10 领涨股价     11 领涨股涨跌额 12 领涨股名称
///
/// **只覆盖涨幅榜**：新浪没有主力净流入/量比/换手，这些列保持 NULL 不补 0（BR-222）。
const SINA_BOARD_URLS: [(&str, &str); 1] = [(
    "concept",
    "https://vip.stock.finance.sina.com.cn/q/view/newFLJK.php?param=class",
)];

/// 一行板块榜数据。
///
/// 字段全部 `Option` —— 不同数据源能提供的列不同，缺失就留 `None` 落库为 NULL。
/// 不补 0 的理由：0 是"跌了 0%"或"没有资金流入"的有效观测值，
/// 用 0 表示"没数据"会让下游把缺失误读成真实观测（BR-222）。
#[derive(Debug, Clone, Default)]
pub struct BoardRow {
    pub code: String,
    pub name: String,
    pub change_pct: Option<f64>,
    pub main_inflow: Option<f64>,
    pub leader_name: Option<String>,
    pub vol_ratio: Option<f64>,
    pub turnover: Option<f64>,
    pub day1_ratio: Option<f64>,
    pub day5_ratio: Option<f64>,
}

impl BoardRow {
    /// 东财桥的全字段记录 → BoardRow（所有列都有值）。
    fn from_fact(fact: BoardRankingFact) -> Self {
        Self {
            code: fact.code,
            name: fact.name,
            change_pct: Some(fact.change_pct),
            main_inflow: Some(fact.main_inflow),
            leader_name: Some(fact.leader_name),
            vol_ratio: Some(fact.vol_ratio),
            turnover: Some(fact.turnover),
            day1_ratio: Some(fact.day1_ratio),
            day5_ratio: Some(fact.day5_ratio),
        }
    }
}

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

/// 拉取新浪板块（概念 + 行业），合并后按涨幅排序取前 N。
async fn fetch_sina_boards(top_n: usize) -> Result<Vec<BoardRow>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| format!("新浪板块 client 构造失败: {error}"))?;

    let mut rows: Vec<BoardRow> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for (kind, url) in SINA_BOARD_URLS {
        let response = match client
            .get(url)
            .header("User-Agent", "Mozilla/5.0")
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                failures.push(format!("{kind} 请求失败: {error}"));
                continue;
            }
        };
        if !response.status().is_success() {
            failures.push(format!("{kind} HTTP {}", response.status()));
            continue;
        }
        let bytes = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => {
                failures.push(format!("{kind} 读取失败: {error}"));
                continue;
            }
        };
        let (text, _, _) = GBK.decode(&bytes);

        // 逐对提取 "板块代码":"逗号分隔字段"
        let mut parsed = 0usize;
        let mut rest: &str = text.as_ref();
        while let Some(colon) = rest.find(":\"") {
            let after = &rest[colon + 2..];
            let Some(quote) = after.find('"') else { break };
            let payload = &after[..quote];
            rest = &after[quote..];

            let fields: Vec<&str> = payload.split(',').collect();
            if fields.len() < 13 {
                continue;
            }
            parsed += 1;
            rows.push(BoardRow {
                code: fields[0].to_string(),
                name: fields[1].to_string(),
                change_pct: fields[5].parse::<f64>().ok(),
                main_inflow: None,
                leader_name: Some(fields[12].to_string()),
                vol_ratio: None,
                turnover: None,
                day1_ratio: None,
                day5_ratio: None,
            });
        }
        log::info!("[板块榜] 新浪 {kind} 解析 {parsed} 行");
    }

    if rows.is_empty() {
        return Err(format!(
            "新浪板块解析出 0 行（失败项: {}）",
            if failures.is_empty() {
                "无".to_string()
            } else {
                failures.join("; ")
            }
        ));
    }

    // 同 code 去重（概念/行业可能有重名），保留涨幅高的那条
    rows.sort_by(|left, right| {
        right
            .change_pct
            .unwrap_or(f64::NEG_INFINITY)
            .partial_cmp(&left.change_pct.unwrap_or(f64::NEG_INFINITY))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut seen = std::collections::HashSet::new();
    rows.retain(|row| seen.insert(row.name.clone()));
    rows.truncate(top_n);
    Ok(rows)
}

/// 拉一路板块榜。
///
/// 顺序：东财 gRPC 桥（全字段）→ 失败时 f3 回退新浪（只有涨跌幅/领涨股）。
/// f62 没有备用源：新浪不提供主力净流入，硬算就是编造，所以直接报错留空。
async fn fetch(fid: &str) -> Result<Vec<BoardRow>, String> {
    let via_bridge = match crate::data_gateway::grpc_source::bridge_for("BoardRanking") {
        Ok(bridge) => match bridge.board_ranking_async(fid, TOP_N).await {
            Ok(crate::data_gateway::GatewayBatch::Available { records, .. }) => {
                Ok(records.into_iter().map(BoardRow::from_fact).collect())
            }
            Ok(crate::data_gateway::GatewayBatch::VerifiedEmpty(_)) => Ok(Vec::new()),
            Err(error) => Err(error.to_string()),
        },
        Err(error) => Err(error.to_string()),
    };

    match via_bridge {
        Ok(rows) if !rows.is_empty() => Ok(rows),
        Ok(_) => {
            if fid == "f3" {
                log::warn!("[板块榜] f3 东财桥返回空，回退新浪行业板块");
                fetch_sina_boards(TOP_N).await
            } else {
                Ok(Vec::new())
            }
        }
        Err(error) => {
            if fid == "f3" {
                log::warn!("[板块榜] f3 东财桥失败（{error}），回退新浪行业板块");
                fetch_sina_boards(TOP_N).await
            } else {
                Err(format!(
                    "fid={fid}: {error}；该榜无备用源（新浪不提供主力净流入，强行推算等于编造）"
                ))
            }
        }
    }
}

fn persist(conn: &mut SqliteConnection, date: &str, fid: &str, rows: &[BoardRow]) -> Result<usize, String> {
    let mut written = 0usize;
    for (index, row) in rows.iter().enumerate() {
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
        .bind::<Text, _>(&row.code)
        .bind::<Text, _>(&row.name)
        // 缺失列写 NULL，不补 0（BR-222）
        .bind::<Nullable<Double>, _>(row.change_pct)
        .bind::<Nullable<Double>, _>(row.main_inflow)
        .bind::<Nullable<Text>, _>(row.leader_name.as_deref())
        .bind::<Nullable<Double>, _>(row.vol_ratio)
        .bind::<Nullable<Double>, _>(row.turnover)
        .bind::<Nullable<Double>, _>(row.day1_ratio)
        .bind::<Nullable<Double>, _>(row.day5_ratio)
        .execute(conn)
        .map_err(|error| format!("{fid} {} 写入失败: {error}", row.code))?;
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
