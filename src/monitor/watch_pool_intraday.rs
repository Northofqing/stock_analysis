//! 盘中选票（2026-10-24）：14:50 用实时行情 + 当日实时板块榜再选一次。
//!
//! 为什么要有这一轮：盘后 15:40 的观察池是给「明日计划」用的，盘中想买只能等第二天；
//! 而盘前/盘中的 `stock_daily` 还没有当日行，`collect_board_picks` 会退到 2~3 个交易日前的
//! 涨幅（10-19 那批 note 写的是「成分股 2026-10-16」）。这里用两条实时数据补上：
//! 1. `BoardFlows` 当日实时板块榜（15:30 的盘后任务随后会用最终值覆盖同一批行）；
//! 2. `RealtimeQuotes` 当日实时涨幅，覆盖 `collect_board_picks` 的排序字段。
//!
//! 记账口径：这一轮开出的票 `source='board_rotation_pm'`、note 带「盘中14:50」，
//! 与盘后 `board_rotation` 分开，复盘时能直接比较两种时点。

use crate::data_gateway::grpc_source;
use crate::data_gateway::GatewayBatch;
use crate::data_gateway::market_data::RealtimeMarketQuote;
use crate::database::DatabaseManager;
use crate::monitor::{
    board_member, board_ranking_snapshot, ticket_entry_proposal, ticket_watch_pool,
};
use diesel::sql_types::{Integer, Nullable, Text};
use diesel::RunQueryDsl as _;
use std::collections::{HashMap, HashSet};

/// 盘中轮动的来源标记（与盘后 `board_rotation` 分开记）。
pub const INTRADAY_SOURCE: &str = "board_rotation_pm";
/// note 里的口径标签。
pub const INTRADAY_LABEL: &str = "盘中14:50";
/// 行情批次大小（桥对单批 code 数有限制，100 是既有调用方在用的量级）。
const QUOTE_CHUNK: usize = 100;
/// 取当日榜 top 板块数，与 `BOARD_TOP_N` 对齐。
const TOP_BOARDS: usize = 10;

pub struct IntradayReport {
    pub boards: usize,
    pub member_codes: usize,
    pub quoted: usize,
    pub candidates: usize,
    pub opened: usize,
    pub picks: Vec<(String, String)>,
}

impl IntradayReport {
    pub fn summary_line(&self) -> String {
        format!(
            "[盘中选票] 板块 {} 个 / 成分 {} 只 / 有行情 {} 只 / 候选 {} / 新开 {} / 推送 {} 只",
            self.boards,
            self.member_codes,
            self.quoted,
            self.candidates,
            self.opened,
            self.picks.len()
        )
    }
}

/// 跑一轮盘中选票。失败返回 Err 由调用方记 warn 并在下一个 tick 重试。
pub async fn run(db: &DatabaseManager, date: &str) -> Result<IntradayReport, String> {
    // 1) 当日实时板块榜（BoardFlows；失败时函数内部已回退新浪，只有两路都失败才 Err）。
    let board_report = board_ranking_snapshot::refresh_board_rankings(db, date).await?;
    board_report.log();

    // 2) 当日榜单 top 板块 → 成分股（TDX 目录）。
    if let Err(error) = board_member::build_from_ranking(db, date, TOP_BOARDS, 10_000).await {
        log::warn!("[盘中选票] 板块成分建表失败（继续用已有成分）: {error}");
    }

    // 3) 涨停池 + 板块龙头：与盘后 15:40 同一口径，失败不阻断。
    let limit_up_codes = collect_limit_up_codes(date);
    let leader_codes = collect_leader_codes().await;

    // 4) top 板块成分股 → 实时涨幅。
    let codes = member_codes(db, date)?;
    let pct_by_code = fetch_pct_map(&codes).await;
    let quoted = pct_by_code.len();

    // 5) 用实时涨幅建池（source=board_rotation_pm）。
    let live = ticket_watch_pool::LiveSnapshot {
        pct_by_code,
        label: INTRADAY_LABEL.to_string(),
        source: INTRADAY_SOURCE,
    };
    let empty_members: HashMap<String, Vec<String>> = HashMap::new();
    let report = ticket_watch_pool::build_watch_pool(
        db,
        date,
        None,
        false,
        &limit_up_codes,
        &leader_codes,
        &empty_members,
        Some(&live),
    )?;
    report.log();

    // 6) 有买点的票立刻给入场区间（observing → entry_proposed），当日可执行。
    match ticket_entry_proposal::propose_for_observing(db, date) {
        Ok(proposal) => log::info!("{}", proposal.summary_line()),
        Err(error) => log::warn!("[盘中选票] 入场参考失败: {error}"),
    }

    // 7) 本轮开出来的票（供推送）。
    let picks = tickets_for_source(db, date, INTRADAY_SOURCE)?;

    Ok(IntradayReport {
        boards: board_report.f3_count,
        member_codes: codes.len(),
        quoted,
        candidates: report.candidates,
        opened: report.opened,
        picks,
    })
}

fn collect_limit_up_codes(date: &str) -> HashSet<String> {
    let Ok(bridge) = grpc_source::bridge_for("LimitPools") else {
        log::warn!("[盘中选票] 涨停池桥不可用，本轮退回涨幅排序");
        return HashSet::new();
    };
    let Ok(day) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
        return HashSet::new();
    };
    match bridge.limit_pools(day) {
        Ok(batch) => batch
            .records()
            .iter()
            .map(|entry| entry.instrument.code().to_owned())
            .collect(),
        Err(error) => {
            log::warn!("[盘中选票] 涨停池拉取失败，本轮退回涨幅排序: {error}");
            HashSet::new()
        }
    }
}

async fn collect_leader_codes() -> HashSet<String> {
    use crate::data_gateway::board::{BoardDataGateway, BoardKind};
    let mut set = HashSet::new();
    for kind in [BoardKind::Industry, BoardKind::Concept] {
        match BoardDataGateway::default()
            .day1_flows(kind, TOP_BOARDS as u32)
            .await {
            Ok(batch) => {
                for fact in batch.records() {
                    if let Some(code) = &fact.leader_code {
                        let code = code.trim();
                        if !code.is_empty() {
                            set.insert(code.to_owned());
                        }
                    }
                }
            }
            Err(error) => log::warn!("[盘中选票] 板块龙头拉取失败({kind:?}): {error}"),
        }
    }
    set
}

/// 当日榜 top 板块的成分股（去重）。
fn member_codes(db: &DatabaseManager, date: &str) -> Result<Vec<String>, String> {
    #[derive(diesel::QueryableByName)]
    struct CodeRow {
        #[diesel(sql_type = Text)]
        instrument_code: String,
    }
    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    let rows: Vec<CodeRow> = diesel::sql_query(
        "SELECT DISTINCT m.instrument_code \
           FROM board_member m \
           JOIN board_ranking_daily r \
             ON r.board_name = m.board_name AND r.trade_date = ?1 \
          WHERE r.rank_position < ?2 AND r.rank_fid IN ('f3','f62')",
    )
    .bind::<Text, _>(date.to_owned())
    .bind::<Integer, _>(TOP_BOARDS as i32)
    .load(&mut *conn)
    .map_err(|error| format!("盘中候选成分查询失败: {error}"))?;
    Ok(rows.into_iter().map(|row| row.instrument_code).collect())
}

/// 批量拉实时涨幅；单批失败只记录，不影响其它批次。
async fn fetch_pct_map(codes: &[String]) -> HashMap<String, f64> {
    let mut map = HashMap::new();
    if codes.is_empty() {
        return map;
    }
    let Ok(bridge) = grpc_source::bridge_for("RealtimeQuotes") else {
        log::warn!("[盘中选票] 行情桥不可用，本轮不选票");
        return map;
    };
    for chunk in codes.chunks(QUOTE_CHUNK) {
        let quotes: Vec<RealtimeMarketQuote> = match bridge.realtime_quotes_async(chunk).await {
            Ok(GatewayBatch::Available { records, .. }) => records,
            Ok(GatewayBatch::VerifiedEmpty(_)) => Vec::new(),
            Err(error) if error.reason_code() == "no_last_traded_price" => {
                bridge.realtime_quotes_with_isolation(chunk).await.quotes
            }
            Err(error) => {
                log::warn!(
                    "[盘中选票] 行情批次失败（{} 只）: {error}",
                    chunk.len()
                );
                Vec::new()
            }
        };
        for quote in quotes {
            map.insert(quote.code, quote.change_percent);
        }
    }
    map
}

fn tickets_for_source(
    db: &DatabaseManager,
    date: &str,
    source: &str,
) -> Result<Vec<(String, String)>, String> {
    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Nullable<Text>)]
        note: Option<String>,
    }
    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT code, note FROM ticket_lifecycle \
          WHERE open_date = ?1 AND source = ?2 ORDER BY code",
    )
    .bind::<Text, _>(date.to_owned())
    .bind::<Text, _>(source.to_owned())
    .load(&mut *conn)
    .map_err(|error| format!("盘中票查询失败: {error}"))?;
    Ok(rows
        .into_iter()
        .map(|row| (row.code, row.note.unwrap_or_default()))
        .collect())
}

/// 推送正文：只列本轮新开的票，附板块与实时涨幅；无票返回空串。
pub fn render_push(db: &DatabaseManager, report: &IntradayReport) -> String {
    if report.picks.is_empty() {
        return String::new();
    }
    let mut names: HashMap<String, String> = HashMap::new();
    if let Ok(mut conn) = db.get_conn() {
        for (code, _) in &report.picks {
            if let Some(name) = crate::monitor::buy_zone::resolve_name(&mut conn, code) {
                names.insert(code.clone(), name);
            }
        }
    }
    let mut text = format!(
        "【盘中选票 14:50】{} 只（板块 {} / 成分 {} / 有行情 {}）\n",
        report.picks.len(),
        report.boards,
        report.member_codes,
        report.quoted
    );
    for (code, note) in &report.picks {
        let name = names.get(code).map(|value| value.as_str()).unwrap_or("-");
        text.push_str(&format!("· {code} {name} ｜ {note}\n"));
    }
    text.push_str("口径：盘中按实时涨幅选；15:40 盘后还会再选一次（来源 board_rotation_pm / board_rotation 分开记账）。\n");
    text.push_str("假设执行、非真实收益。");
    text
}
