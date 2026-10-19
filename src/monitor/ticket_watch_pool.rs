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
             total={} opened={} appended={} skipped_invalid={} dry_run={}",
            self.scanned_rows,
            self.news_candidates,
            self.board_scanned_rows,
            self.board_candidates,
            self.board_added,
            self.candidates,
            self.opened,
            self.appended,
            self.skipped_invalid_code,
            self.dry_run
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
        _ => "新闻",
    }
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
pub fn collect_board_rotation(
    db: &DatabaseManager,
    business_date: &str,
    top_boards: usize,
    per_board: usize,
) -> Result<(usize, Vec<PoolCandidate>), String> {
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
          WHERE trade_date = ?1 AND rank_position < ?2 \
          GROUP BY board_name \
          ORDER BY rank_position",
    )
    .bind::<diesel::sql_types::Text, _>(business_date.to_owned())
    .bind::<diesel::sql_types::Integer, _>(top_boards as i32)
    .load(&mut *conn)
    .map_err(|e| format!("板块榜查询失败: {e}"))?;

    let scanned = boards.len();
    if boards.is_empty() {
        return Ok((0, Vec::new()));
    }

    // 最近一个样本足够的交易日（当日只有活跃票时退到上一交易日）。
    #[derive(QueryableByName)]
    struct DateRow {
        #[diesel(sql_type = diesel::sql_types::Text)]
        date: String,
    }
    let price_date: Option<String> = diesel::sql_query(
        "SELECT date FROM stock_daily WHERE date <= ?1 \
          GROUP BY date HAVING COUNT(*) >= 50 ORDER BY date DESC LIMIT 1",
    )
    .bind::<diesel::sql_types::Text, _>(business_date.to_owned())
    .load::<DateRow>(&mut *conn)
    .map_err(|e| format!("日线日期查询失败: {e}"))?
    .into_iter()
    .next()
    .map(|row| row.date);

    let Some(price_date) = price_date else {
        return Ok((scanned, Vec::new()));
    };

    #[derive(QueryableByName)]
    struct MemberRow {
        #[diesel(sql_type = diesel::sql_types::Text)]
        board_name: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        instrument_code: String,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
        pct_chg: Option<f64>,
    }

    // 一次取当日全部成分股行情，Rust 侧按板块名过滤。
    //
    // 不用 `IN (?,?,…)` 动态绑定：diesel 的 `query = query.bind(..)` 会改变返回类型
    // （`SqlQuery` → `UncheckedBind`），循环里赋值编不过。board_member 只有万级行，
    // 全量 join 一次足够。
    let rows: Vec<MemberRow> = diesel::sql_query(
        "SELECT m.board_name, m.instrument_code, d.pct_chg \
           FROM board_member m \
           JOIN stock_daily d ON d.code = m.instrument_code AND d.date = ?1",
    )
    .bind::<diesel::sql_types::Text, _>(price_date.clone())
    .load(&mut *conn)
    .map_err(|e| format!("板块成分查询失败: {e}"))?;

    let wanted: std::collections::HashSet<&str> =
        boards.iter().map(|board| board.board_name.as_str()).collect();

    let mut by_board: std::collections::HashMap<String, Vec<MemberRow>> =
        std::collections::HashMap::new();
    for row in rows {
        if !wanted.contains(row.board_name.as_str()) {
            continue;
        }
        by_board.entry(row.board_name.clone()).or_default().push(row);
    }

    let mut candidates: Vec<PoolCandidate> = Vec::new();
    for board in &boards {
        let Some(mut members) = by_board.remove(&board.board_name) else {
            continue;
        };
        // 涨幅缺失的排最后（BR-222：不补 0）
        members.sort_by(|a, b| {
            b.pct_chg
                .partial_cmp(&a.pct_chg)
                .unwrap_or(std::cmp::Ordering::Equal)
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
                "板块轮动 #{} {} ({}) 成分股 {} 涨幅 {}",
                board.rank_position + 1,
                board.board_name,
                pct_text,
                price_date,
                leader_pct
            );
            match candidates.iter_mut().find(|item| item.code == member.instrument_code) {
                Some(existing) => {
                    existing.hits += 1;
                    existing.note.push_str(&format!("；亦属 #{} {}", board.rank_position + 1, board.board_name));
                }
                None => candidates.push(PoolCandidate {
                    code: member.instrument_code.clone(),
                    latest_at: business_date.to_owned(),
                    hits: 1,
                    note,
                    source: "board_rotation",
                }),
            }
        }
    }

    Ok((scanned, candidates))
}


/// 把候选写进观察池。`dry_run=true` 只打印。
pub fn build_watch_pool(
    db: &DatabaseManager,
    business_date: &str,
    lookback_days: Option<i64>,
    dry_run: bool,
) -> Result<WatchPoolReport, String> {
    let lookback = lookback_days.unwrap_or(DEFAULT_LOOKBACK_DAYS);
    let (scanned_rows, mut candidates) = collect_news_catalysts(db, business_date, lookback)?;
    let news_count = candidates.len();

    // 第二路：板块轮动（board_ranking_daily top N → board_member 成分股）。
    let (board_scanned, board_candidates) = collect_board_rotation(
        db,
        business_date,
        BOARD_TOP_N,
        BOARD_PER_BOARD,
    )?;
    let board_count_raw = board_candidates.len();
    let mut board_added = 0usize;
    for candidate in board_candidates {
        match candidates.iter_mut().find(|item| item.code == candidate.code) {
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

    let mut report = WatchPoolReport {
        scanned_rows,
        candidates: candidates.len(),
        news_candidates: news_count,
        board_scanned_rows: board_scanned,
        board_candidates: board_count_raw,
        board_added,
        dry_run,
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
                match ticket_lifecycle::append_event(
                    &mut *conn,
                    active.id,
                    &now,
                    "catalyst",
                    None,
                    None,
                    Some(&summary),
                ) {
                    Ok(()) => report.appended += 1,
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
                        if let Err(error) = ticket_lifecycle::append_event(
                            &mut *conn,
                            ticket_id,
                            &now,
                            "catalyst",
                            None,
                            None,
                            Some(&format!("首次入池, 命中 {} 条催化", item.hits)),
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
