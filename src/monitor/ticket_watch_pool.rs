//! 观察池生成 —— 票的一等公民第 2 步 (2026-10-17).
//!
//! 口径（operator 2026-10-17 定）：**按板块轮动、按新闻催化**。
//!
//! 本模块先落地"新闻催化"这一路：
//!   `news_ai_assessment`（新闻 AI 评估）里 `impact ∈ {positive, major_positive}`
//!   且 `target_code` 非空的记录 → 按票去重 → 开 ticket(stage=observing,
//!   source=news_catalyst)。
//!
//! 为什么能直接做：该表近 30 天 890 条记录 **全部带 target_code**（实测），
//! 等于现成的"新闻→个股"映射，不需要另外做实体识别。
//!
//! "板块轮动"那一路暂未接：`board_rotation_daily` 表结构完整（板块 + 新闻标题 +
//! 成分股 JSON），但 `save_board_rotations` **只有定义与测试、零生产调用**，
//! 数据停在 2026-07-16。要接它需要组装三源（板块排名 + 板块成分 + 新闻），
//! 属独立工作项，不在本模块内假装已支持。
//!
//! 幂等与写入安全：
//! - 已活跃的票不重复开票，只追加一条 catalyst 事件；
//! - `dry_run=true` 时只打印、不写库（首次运行先看它会开哪些票）。

use crate::database::ticket_lifecycle;
use crate::database::DatabaseManager;
use crate::monitor::ticket_decision::SourceHealth;
use diesel::{QueryableByName, RunQueryDsl};

/// 默认回看天数（新闻催化）。
const DEFAULT_LOOKBACK_DAYS: i64 = 5;
/// 板块轮动取当日板块榜前几名。
const BOARD_TOP_N: usize = 10;
/// 每个热门板块取涨幅前几只成分股。
const BOARD_PER_BOARD: usize = 3;
/// note 摘要截断长度（`core_logic` 可能数百字，只留开头）。
const NOTE_MAX_CHARS: usize = 120;

#[derive(Debug, Clone)]
pub struct PoolCandidate {
    pub code: String,
    /// 最新一条催化的时间（板块路径填当日）
    pub latest_at: String,
    /// 命中的催化条数（板块路径填该票所属热门板块数）
    pub hits: u32,
    /// 最新一条 `core_logic`（已截断）
    pub note: String,
    /// 入池来源：`news_catalyst` / `board_rotation`
    pub source: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct WatchPoolReport {
    /// 新闻评估表扫描行数
    pub scanned_rows: usize,
    /// 新闻路候选数（去重后）
    pub news_candidates: usize,
    /// 板块榜命中板块数
    pub board_scanned_rows: usize,
    /// 板块路原始候选数
    pub board_candidates: usize,
    /// 板块路新增（不与新闻重合）的票数
    pub board_added: usize,
    /// 资金流路（f62）命中板块数
    pub mf_scanned_rows: usize,
    /// 资金流路原始候选数
    pub mf_candidates: usize,
    /// 资金流路新增（不与其他两路重合）的票数
    pub mf_added: usize,
    /// 三条来源各自的健康状态；未执行/失败时必须显式标记。
    pub news_health: Option<SourceHealth>,
    pub board_health: Option<SourceHealth>,
    pub mf_health: Option<SourceHealth>,
    /// 合并后总候选数
    pub candidates: usize,
    pub opened: usize,
    pub appended: usize,
    pub skipped_invalid_code: usize,
    pub dry_run: bool,
    pub errors: Vec<String>,
}

impl WatchPoolReport {
    pub fn log(&self) {
        log::info!(
            "[观察池] news_scanned={} news_candidates={} boards={} board_candidates={} board_added={} \
             mf_boards={} mf_candidates={} mf_added={} total={} opened={} appended={} \
             skipped_invalid={} dry_run={} news_health={} board_health={} mf_health={}",
            self.scanned_rows,
            self.news_candidates,
            self.board_scanned_rows,
            self.board_candidates,
            self.board_added,
            self.mf_scanned_rows,
            self.mf_candidates,
            self.mf_added,
            self.candidates,
            self.opened,
            self.appended,
            self.skipped_invalid_code,
            self.dry_run,
            health_label(self.news_health),
            health_label(self.board_health),
            health_label(self.mf_health)
        );
        for error in &self.errors {
            log::warn!("[观察池] {error}");
        }
    }
}

/// 只接受 6 位数字的 A 股代码（把 `target_code` 的脏值挡在池外）。
fn is_valid_code(code: &str) -> bool {
    code.len() == 6 && code.chars().all(|c| c.is_ascii_digit())
}

fn source_label(source: &str) -> &'static str {
    match source {
        "board_rotation" => "板块轮动",
        "money_flow" => "资金流",
        "p01_preopen" => "P01 盘前",
        "holding" => "持仓",
        _ => "新闻",
    }
}

fn health_label(health: Option<SourceHealth>) -> &'static str {
    health.unwrap_or(SourceHealth::Unavailable).as_str()
}

fn truncate_chars(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}

/// 从新闻催化生成观察池候选（纯查询，不写库）。
pub fn collect_news_catalysts(
    db: &DatabaseManager,
    business_date: &str,
    lookback_days: i64,
) -> Result<(usize, Vec<PoolCandidate>), String> {
    let since = chrono::NaiveDate::parse_from_str(business_date, "%Y-%m-%d")
        .map_err(|error| format!("business_date 非法 ({business_date}): {error}"))?
        - chrono::Duration::days(lookback_days.max(1));
    let since_text = since.format("%Y-%m-%d").to_string();

    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = diesel::sql_types::Text)]
        target_code: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        impact: String,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
        core_logic: Option<String>,
        #[diesel(sql_type = diesel::sql_types::Text)]
        created_at: String,
    }

    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    // 按时间倒序，第一次见到某只票即为"最新催化"。
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT target_code, impact, core_logic, created_at FROM news_ai_assessment \
         WHERE created_at >= ?1 \
         AND impact IN ('positive','major_positive') \
         AND target_code IS NOT NULL AND trim(target_code) <> '' \
         ORDER BY created_at DESC",
    )
    .bind::<diesel::sql_types::Text, _>(&since_text)
    .load(&mut *conn)
    .map_err(|e| format!("news catalyst query failed: {e}"))?;

    let scanned = rows.len();
    let mut ordered: Vec<PoolCandidate> = Vec::new();
    let mut invalid = 0usize;
    for row in rows {
        let code = row.target_code.trim().to_string();
        if !is_valid_code(&code) {
            invalid += 1;
            continue;
        }
        match ordered.iter_mut().find(|item| item.code == code) {
            Some(existing) => existing.hits += 1,
            None => ordered.push(PoolCandidate {
                code,
                latest_at: row.created_at.clone(),
                hits: 1,
                note: truncate_chars(row.core_logic.as_deref().unwrap_or(""), NOTE_MAX_CHARS),
                source: "news_catalyst",
            }),
        }
    }
    // 出现在更早的重复行里不影响排序；总体保持"最新催化在前"。
    let _ = &mut invalid;
    Ok((scanned, ordered))
}

/// 板块轮动候选：当日板块榜 top N → 反查成分股 → 按上一完整交易日涨幅挑每板块前 M。
///
/// 数据依赖 `board_member`（板块←成分股反查表，由 `board_member_build` 建）。
/// 板块榜的 `board_name` 与 TDX 板块名同名（「大飞机」「半导体」…），按名 join。
///
/// 为什么成分股用上一完整交易日涨幅排序而不是当日：`stock_daily` 当日只有活跃票
/// 样本（2026-10-19 仅 9 只），拿它排序会把 1241 只里的强票挡在外面。
/// 涨停判定阈值按板分档（2026-10-23 批 1.2）：20cm 板 19.5%、其余 9.85%，见
/// [`crate::monitor::ticket_regime::limit_up_pct`]。用 pct_chg 判，不依赖
/// `stock_daily.is_limit_up` —— 那个字段在 kline.rs:650 硬编码 false，用它等于永远判不出涨停。
/// 板块路候选的通用实现：`board_date` 的榜（`rank_fid`）top N → 成分股 → 每板块前 M。
///
/// `source` / `label` 由调用方给：涨幅榜 `f3` 记 `board_rotation`，主力净流入榜 `f62`
/// 记 `money_flow`（批 2.3）——两者是独立信号，不能混成一个来源，否则复盘归因分不开。
/// 盘中选票用的实时快照（2026-10-24：operator 要求 15:05 前推一次、盘后再一次）。
///
/// 盘前/盘中的问题：当日 `stock_daily` 还没落库，`collect_board_picks` 只能 join
/// 最近一个样本足够的交易日 —— 2026-10-19 那批因此用了 10-16 的涨幅（note 里可查），
/// 选出来的是「3 天前涨过一波」的票。这个结构用来把实时涨幅注入排序。
#[derive(Debug, Clone)]
pub struct LiveSnapshot {
    /// code → 当日实时涨跌幅（%）
    pub pct_by_code: std::collections::HashMap<String, f64>,
    /// note 里的口径标签，例如「盘中14:50」
    pub label: String,
    /// 写进 `ticket_lifecycle.source`，与盘后 `board_rotation` 分开记，方便复盘对比
    pub source: &'static str,
}

/// 涨幅口径：有实时值就用实时值，否则回落到日线；两者都没有返回 None（不补 0）。
fn effective_pct(live: Option<&LiveSnapshot>, code: &str, daily: Option<f64>) -> Option<f64> {
    live.and_then(|snapshot| snapshot.pct_by_code.get(code).copied())
        .or(daily)
}

#[allow(clippy::too_many_arguments)]
fn collect_board_picks(
    db: &DatabaseManager,
    board_date: &str,
    rank_fid: &str,
    source: &'static str,
    label: &str,
    top_boards: usize,
    per_board: usize,
    limit_up_codes: &std::collections::HashSet<String>,
    leader_codes: &std::collections::HashSet<String>,
    eastmoney_members: &std::collections::HashMap<String, Vec<String>>,
    live: Option<&LiveSnapshot>,
) -> Result<(usize, Vec<PoolCandidate>), String> {
    // 盘中路用独立的 source 记账（board_rotation_pm），盘后仍用调用方传入的值。
    let source = live.map(|snapshot| snapshot.source).unwrap_or(source);

    #[derive(QueryableByName)]
    struct BoardRow {
        #[diesel(sql_type = diesel::sql_types::Text)]
        board_name: String,
        #[diesel(sql_type = diesel::sql_types::Integer)]
        rank_position: i32,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
        change_pct: Option<f64>,
    }

    let mut conn = db.get_conn().map_err(|e| e.to_string())?;

    let boards: Vec<BoardRow> = diesel::sql_query(
        "SELECT board_name, MIN(rank_position) AS rank_position, MAX(change_pct) AS change_pct \
           FROM board_ranking_daily \
          WHERE trade_date = ?1 AND rank_fid = ?2 AND rank_position < ?3 \
          GROUP BY board_name \
          ORDER BY rank_position",
    )
    .bind::<diesel::sql_types::Text, _>(board_date.to_owned())
    .bind::<diesel::sql_types::Text, _>(rank_fid.to_owned())
    .bind::<diesel::sql_types::Integer, _>(top_boards as i32)
    .load(&mut *conn)
    .map_err(|e| format!("{label}榜查询失败: {e}"))?;

    let scanned = boards.len();
    if boards.is_empty() {
        return Ok((0, Vec::new()));
    }

    #[derive(QueryableByName)]
    struct MemberRow {
        #[diesel(sql_type = diesel::sql_types::Text)]
        board_name: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        instrument_code: String,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
        pct_chg: Option<f64>,
    }

    // 两种取成员方式：
    // - 盘后（live=None）：join 当日（或最近样本足够的）日线取涨幅；
    // - 盘中（live=Some）：当日日线还没落库，join 会返回 0 行 —— 直接读 board_member，
    //   涨幅用实时行情覆盖（缺失留 None，不补 0）。
    let (rows, price_label): (Vec<MemberRow>, String) = match live {
        Some(snapshot) => {
            #[derive(QueryableByName)]
            struct MemberCodeRow {
                #[diesel(sql_type = diesel::sql_types::Text)]
                board_name: String,
                #[diesel(sql_type = diesel::sql_types::Text)]
                instrument_code: String,
            }
            let base: Vec<MemberCodeRow> =
                diesel::sql_query("SELECT board_name, instrument_code FROM board_member")
                    .load(&mut *conn)
                    .map_err(|e| format!("{label}板块成分查询失败: {e}"))?;
            let rows = base
                .into_iter()
                .map(|row| MemberRow {
                    pct_chg: effective_pct(Some(snapshot), &row.instrument_code, None),
                    board_name: row.board_name,
                    instrument_code: row.instrument_code,
                })
                .collect();
            (rows, snapshot.label.clone())
        }
        None => {
            #[derive(QueryableByName)]
            struct DateRow {
                #[diesel(sql_type = diesel::sql_types::Text)]
                date: String,
            }
            let price_date: Option<String> = diesel::sql_query(
                "SELECT date FROM stock_daily WHERE date <= ?1 \
                  GROUP BY date HAVING COUNT(*) >= 50 ORDER BY date DESC LIMIT 1",
            )
            .bind::<diesel::sql_types::Text, _>(board_date.to_owned())
            .load::<DateRow>(&mut *conn)
            .map_err(|e| format!("日线日期查询失败: {e}"))?
            .into_iter()
            .next()
            .map(|row| row.date);
            let Some(price_date) = price_date else {
                return Ok((scanned, Vec::new()));
            };
            let rows: Vec<MemberRow> = diesel::sql_query(
                "SELECT m.board_name, m.instrument_code, d.pct_chg \
                   FROM board_member m \
                   JOIN stock_daily d ON d.code = m.instrument_code AND d.date = ?1",
            )
            .bind::<diesel::sql_types::Text, _>(price_date.clone())
            .load(&mut *conn)
            .map_err(|e| format!("板块成分查询失败: {e}"))?;
            (rows, price_date)
        }
    };

    // code → 当日涨跌幅，给东财成分股补 pct_chg 用（那些票不在 board_member 里）。
    let pct_by_code: std::collections::HashMap<String, f64> = rows
        .iter()
        .filter_map(|row| row.pct_chg.map(|pct| (row.instrument_code.clone(), pct)))
        .collect();

    let wanted: std::collections::HashSet<&str> = boards
        .iter()
        .map(|board| board.board_name.as_str())
        .collect();

    let mut by_board: std::collections::HashMap<String, Vec<MemberRow>> =
        std::collections::HashMap::new();
    for row in rows {
        if !wanted.contains(row.board_name.as_str()) {
            continue;
        }
        by_board
            .entry(row.board_name.clone())
            .or_default()
            .push(row);
    }

    // 东财板块成分（BK 码 → ExternalV1 Eastmoney 臂）：board_member 是 TDX 体系，
    // 板块名和行业榜（生猪养殖/电视广播Ⅲ）对不上，实测交集为 0。这条来源由调用方
    // 按当日榜单 top 板块实时拉取，pct_chg 从同一天日线补（缺失留 None，BR-222）。
    for (board_name, codes) in eastmoney_members {
        if !wanted.contains(board_name.as_str()) {
            continue;
        }
        let entry = by_board.entry(board_name.clone()).or_default();
        let existing: std::collections::HashSet<String> = entry
            .iter()
            .map(|row| row.instrument_code.clone())
            .collect();
        for code in codes {
            if existing.contains(code) {
                continue;
            }
            let pct_chg = pct_by_code.get(code).copied();
            entry.push(MemberRow {
                board_name: board_name.clone(),
                instrument_code: code.clone(),
                pct_chg,
            });
        }
    }

    let mut candidates: Vec<PoolCandidate> = Vec::new();
    for board in &boards {
        let Some(mut members) = by_board.remove(&board.board_name) else {
            continue;
        };
        // 排序：今日涨停优先，其次按涨幅降序，涨幅缺失的排最后（BR-222：不补 0）。
        //
        // 2026-10-21 改动原因：原逻辑只看「上一交易日涨幅」。stock_daily 当天常常
        // 只有几十只票（10-19 47 只 / 10-20 46 只），price_date 会退到 5 天前，
        // 于是选出来的是「5 天前涨过一波」的票——10-21 选中 002976 瑞玛精密
        //（10-16 +9.52%，之后两天跌 9%），而当天真正封板的 002929/002313/002792
        // 一只都没进。涨停集合由调用方实时拉 LimitPools 传入，不依赖日线。
        members.sort_by(|a, b| {
            let a_limit = a
                .pct_chg
                .map(|v| v >= crate::monitor::ticket_regime::limit_up_pct(&a.instrument_code))
                .unwrap_or(false);
            let b_limit = b
                .pct_chg
                .map(|v| v >= crate::monitor::ticket_regime::limit_up_pct(&b.instrument_code))
                .unwrap_or(false);
            let a_today = limit_up_codes.contains(&a.instrument_code);
            let b_today = limit_up_codes.contains(&b.instrument_code);
            // 板块领涨股（BoardFlows.leader_code）：不要求它今天涨停，
            // 但它是板块里资金认的那一只，优先级仅次于今日涨停。
            let a_leader = leader_codes.contains(&a.instrument_code);
            let b_leader = leader_codes.contains(&b.instrument_code);
            // 今日实时涨停 > 板块龙头 > 日线涨停 > 涨幅高 > 涨幅缺失
            b_today
                .cmp(&a_today)
                .then(b_leader.cmp(&a_leader))
                .then(b_limit.cmp(&a_limit))
                .then(
                    b.pct_chg
                        .partial_cmp(&a.pct_chg)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        });
        let pct_text = board
            .change_pct
            .map(|value| format!("{value:+.2}%"))
            .unwrap_or_else(|| "涨幅缺失".to_owned());
        for member in members.into_iter().take(per_board) {
            if !is_valid_code(&member.instrument_code) {
                continue;
            }
            let leader_pct = member
                .pct_chg
                .map(|value| format!("{value:+.2}%"))
                .unwrap_or_else(|| "无日线".to_owned());
            let note = format!(
                "{label} #{} {} ({}) 成分股 {} 涨幅 {}",
                board.rank_position + 1,
                board.board_name,
                pct_text,
                price_label,
                leader_pct
            );
            match candidates
                .iter_mut()
                .find(|item| item.code == member.instrument_code)
            {
                Some(existing) => {
                    existing.hits += 1;
                    existing.note.push_str(&format!(
                        "；亦属 #{} {}",
                        board.rank_position + 1,
                        board.board_name
                    ));
                }
                None => candidates.push(PoolCandidate {
                    code: member.instrument_code.clone(),
                    latest_at: board_date.to_owned(),
                    hits: 1,
                    note,
                    source,
                }),
            }
        }
    }

    Ok((scanned, candidates))
}

/// 板块轮动路：当日涨幅榜（`f3`）top N → 成分股 → 按上一完整交易日涨幅挑每板块前 M。
pub fn collect_board_rotation(
    db: &DatabaseManager,
    business_date: &str,
    top_boards: usize,
    per_board: usize,
    limit_up_codes: &std::collections::HashSet<String>,
    leader_codes: &std::collections::HashSet<String>,
    eastmoney_members: &std::collections::HashMap<String, Vec<String>>,
    live: Option<&LiveSnapshot>,
) -> Result<(usize, Vec<PoolCandidate>), String> {
    collect_board_picks(
        db,
        business_date,
        "f3",
        "board_rotation",
        "板块轮动",
        top_boards,
        per_board,
        limit_up_codes,
        leader_codes,
        eastmoney_members,
        live,
    )
}

/// f62 资金流榜要求的上一交易日；解析失败显式报错，不静默回退。
fn previous_trading_day_text(business_date: &str) -> Result<String, String> {
    let business_day = chrono::NaiveDate::parse_from_str(business_date, "%Y-%m-%d")
        .map_err(|e| format!("业务日期非法 {business_date}: {e}"))?;
    Ok(crate::calendar::prev_trading_day(business_day).to_string())
}

/// 资金流路（批 2.3 / 拓 7）：`f62` 主力净流入榜 top N → 成分股 → 每板块前 M。
///
/// 取**当日**榜（`board_ranking_daily` 15:30 已写）；若最新 f62 不是上一交易日，
/// 显式报 source unavailable，**不**拿旧榜当当日资金流。
/// 10-21 之前 f62 无数据 → 返回 0，不补造。
pub fn collect_money_flow_boards(
    db: &DatabaseManager,
    business_date: &str,
    top_boards: usize,
    per_board: usize,
    limit_up_codes: &std::collections::HashSet<String>,
    leader_codes: &std::collections::HashSet<String>,
    eastmoney_members: &std::collections::HashMap<String, Vec<String>>,
    live: Option<&LiveSnapshot>,
) -> Result<(usize, Vec<PoolCandidate>), String> {
    #[derive(QueryableByName)]
    struct DateRow {
        #[diesel(sql_type = diesel::sql_types::Text)]
        trade_date: String,
    }
    let prev: Option<String> = {
        let mut conn = db.get_conn().map_err(|e| e.to_string())?;
        diesel::sql_query(
            "SELECT MAX(trade_date) AS trade_date FROM board_ranking_daily \
              WHERE rank_fid = 'f62' AND trade_date <= ?1",
        )
        .bind::<diesel::sql_types::Text, _>(business_date.to_owned())
        .get_result::<DateRow>(&mut *conn)
        .ok()
        .map(|row| row.trade_date)
    };
    let Some(prev) = prev else {
        return Ok((0, Vec::new()));
    };
    // f62 必须正好是上一交易日：10/23 的榜不能冒充 10/28 的资金流。
    // 数据缺口应显式报 source unavailable，而不是静默用旧榜。
    let expected = previous_trading_day_text(business_date)?;
    if prev != expected {
        return Err(format!(
            "f62 榜最新 {prev}，需要上一交易日 {expected}（陈旧，不能当当日资金流）"
        ));
    }
    collect_board_picks(
        db,
        &prev,
        "f62",
        "money_flow",
        "资金流",
        top_boards,
        per_board,
        limit_up_codes,
        leader_codes,
        eastmoney_members,
        live,
    )
}

/// 同日同票同摘要的 catalyst 事件只写一次（2026-10-22）。
///
/// 观察池块在重启/补跑时会整轮重放：`open_ticket` 已按 code 去重，但「追加催化」
/// 这条路径原来没有判重，同一批票的 catalyst 会再写一遍（10-22 重启后 3 只票各多一条）。
/// 返回 `true` 表示本次真的写入了。
fn append_catalyst_once(
    conn: &mut diesel::SqliteConnection,
    ticket_id: i64,
    event_at: &str,
    business_date: &str,
    summary: &str,
) -> Result<bool, String> {
    if ticket_lifecycle::event_exists_same_day(conn, ticket_id, "catalyst", business_date, summary)?
    {
        return Ok(false);
    }
    ticket_lifecycle::append_event(
        conn,
        ticket_id,
        event_at,
        "catalyst",
        None,
        None,
        Some(summary),
    )?;
    Ok(true)
}

/// 把候选写进观察池。`dry_run=true` 只打印。
pub fn build_watch_pool(
    db: &DatabaseManager,
    business_date: &str,
    lookback_days: Option<i64>,
    dry_run: bool,
    limit_up_codes: &std::collections::HashSet<String>,
    leader_codes: &std::collections::HashSet<String>,
    eastmoney_members: &std::collections::HashMap<String, Vec<String>>,
    live: Option<&LiveSnapshot>,
) -> Result<WatchPoolReport, String> {
    let lookback = lookback_days.unwrap_or(DEFAULT_LOOKBACK_DAYS);
    let mut source_errors: Vec<String> = Vec::new();
    let (scanned_rows, mut candidates, news_health) = match collect_news_catalysts(db, business_date, lookback) {
        Ok((rows, candidates)) => (rows, candidates, SourceHealth::Available),
        Err(error) => {
            source_errors.push(format!("新闻来源不可用: {error}"));
            (0, Vec::new(), SourceHealth::Unavailable)
        }
    };
    let news_count = candidates.len();

    // 第二路：板块轮动（board_ranking_daily top N → board_member 成分股）。
    let (board_scanned, board_candidates, board_health) = match collect_board_rotation(
        db,
        business_date,
        BOARD_TOP_N,
        BOARD_PER_BOARD,
        limit_up_codes,
        leader_codes,
        eastmoney_members,
        live,
    ) {
        Ok((rows, candidates)) => (rows, candidates, SourceHealth::Available),
        Err(error) => {
            source_errors.push(format!("板块轮动来源不可用: {error}"));
            (0, Vec::new(), SourceHealth::Unavailable)
        }
    };
    let board_count_raw = board_candidates.len();
    let mut board_added = 0usize;
    for candidate in board_candidates {
        match candidates
            .iter_mut()
            .find(|item| item.code == candidate.code)
        {
            // 新闻已在池中：把板块信息并进同一条 note，不另开一条。
            Some(existing) => {
                existing.hits += candidate.hits;
                existing.note.push_str(" | ");
                existing.note.push_str(&candidate.note);
            }
            None => {
                candidates.push(candidate);
                board_added += 1;
            }
        }
    }

    // 第三路：资金流（f62 前一交易日主力净流入榜）。与板块路同一套成分/排序口径，
    // 只是榜不同 —— 两条榜可能选到同一只票，合并进同一条 note（hits 累加）。
    let (mf_scanned, mf_candidates, mf_health) = match collect_money_flow_boards(
        db,
        business_date,
        BOARD_TOP_N,
        BOARD_PER_BOARD,
        limit_up_codes,
        leader_codes,
        eastmoney_members,
        live,
    ) {
        Ok((rows, candidates)) => (rows, candidates, SourceHealth::Available),
        Err(error) => {
            source_errors.push(format!("资金流来源不可用: {error}"));
            (0, Vec::new(), SourceHealth::Unavailable)
        }
    };
    let mf_count_raw = mf_candidates.len();
    let mut mf_added = 0usize;
    for candidate in mf_candidates {
        match candidates
            .iter_mut()
            .find(|item| item.code == candidate.code)
        {
            Some(existing) => {
                existing.hits += candidate.hits;
                existing.note.push_str(" | ");
                existing.note.push_str(&candidate.note);
            }
            None => {
                candidates.push(candidate);
                mf_added += 1;
            }
        }
    }

    let mut report = WatchPoolReport {
        scanned_rows,
        candidates: candidates.len(),
        news_candidates: news_count,
        board_scanned_rows: board_scanned,
        board_candidates: board_count_raw,
        board_added,
        mf_scanned_rows: mf_scanned,
        mf_candidates: mf_count_raw,
        mf_added,
        news_health: Some(news_health),
        board_health: Some(board_health),
        mf_health: Some(mf_health),
        dry_run,
        errors: source_errors,
        ..Default::default()
    };

    if dry_run {
        for item in &candidates {
            println!(
                "[dry-run] [{source}] {code} hits={hits} latest={latest} | {note}",
                source = item.source,
                code = item.code,
                hits = item.hits,
                latest = item.latest_at,
                note = item.note
            );
        }
        report.log();
        return Ok(report);
    }

    let now = chrono::Local::now().to_rfc3339();
    for item in &candidates {
        let mut conn = db.get_conn().map_err(|e| e.to_string())?;
        match ticket_lifecycle::find_active_ticket(&mut *conn, &item.code) {
            Ok(Some(active)) => {
                // 已在池中：只追加催化事件，不重复开票。
                let summary = format!(
                    "{} 催化 +{} 条, 最新 {} | {}",
                    source_label(item.source),
                    item.hits,
                    item.latest_at,
                    item.note
                );
                match append_catalyst_once(&mut *conn, active.id, &now, business_date, &summary) {
                    Ok(true) => report.appended += 1,
                    Ok(false) => {} // 同日同摘要已记过（重启/补跑重放）
                    Err(error) => report.errors.push(error),
                }
            }
            Ok(None) => {
                let note = format!(
                    "{} 入池: {} 条, 最新 {} | {}",
                    source_label(item.source),
                    item.hits,
                    item.latest_at,
                    item.note
                );
                match ticket_lifecycle::open_ticket(
                    &mut *conn,
                    &item.code,
                    business_date,
                    "observing",
                    item.source,
                    Some(&note),
                ) {
                    Ok(ticket_id) => {
                        report.opened += 1;
                        let first_summary = format!("首次入池, 命中 {} 条催化", item.hits);
                        if let Err(error) = append_catalyst_once(
                            &mut *conn,
                            ticket_id,
                            &now,
                            business_date,
                            &first_summary,
                        ) {
                            report.errors.push(error);
                        }
                    }
                    Err(error) => report.errors.push(error),
                }
            }
            Err(error) => report.errors.push(error),
        }
    }

    report.log();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_snapshot(value: f64) -> LiveSnapshot {
        let mut pct_by_code = std::collections::HashMap::new();
        pct_by_code.insert("000001".to_string(), value);
        LiveSnapshot {
            pct_by_code,
            label: "盘中14:50".to_string(),
            source: "board_rotation_pm",
        }
    }

    #[test]
    fn live_pct_overrides_daily_and_missing_stays_none() {
        let live = live_snapshot(3.5);
        // 实时值优先于日线
        assert_eq!(effective_pct(Some(&live), "000001", Some(-1.0)), Some(3.5));
        // 实时缺失时回落到日线
        assert_eq!(effective_pct(Some(&live), "600000", Some(-1.0)), Some(-1.0));
        // 两者都缺失：None，不补 0（BR-222）
        assert_eq!(effective_pct(Some(&live), "600000", None), None);
        // 盘后口径：只用日线
        assert_eq!(effective_pct(None, "000001", Some(2.0)), Some(2.0));
        assert_eq!(effective_pct(None, "000001", None), None);
    }

    #[test]
    fn missing_health_is_unavailable_not_silent_success() {
        assert_eq!(health_label(None), "unavailable");
        assert_eq!(health_label(Some(SourceHealth::Available)), "available");
        assert_eq!(health_label(Some(SourceHealth::Partial)), "partial");
        assert_eq!(health_label(Some(SourceHealth::Unavailable)), "unavailable");
    }

    #[test]
    fn f62_requires_exact_previous_trading_day() {
        assert_eq!(
            previous_trading_day_text("2026-10-28").unwrap(),
            "2026-10-27"
        );
        assert!(previous_trading_day_text("not-a-date").is_err());
    }
}
