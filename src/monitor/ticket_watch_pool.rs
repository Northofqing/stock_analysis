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
/// note 摘要截断长度（`core_logic` 可能数百字，只留开头）。
const NOTE_MAX_CHARS: usize = 120;

#[derive(Debug, Clone)]
pub struct PoolCandidate {
    pub code: String,
    /// 最新一条催化的时间
    pub latest_at: String,
    /// 命中的催化条数
    pub hits: u32,
    /// 最新一条 `core_logic`（已截断）
    pub note: String,
}

#[derive(Debug, Clone, Default)]
pub struct WatchPoolReport {
    pub scanned_rows: usize,
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
            "[观察池] news_scanned={} candidates={} opened={} appended={} skipped_invalid={} dry_run={}",
            self.scanned_rows,
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
            }),
        }
    }
    // 出现在更早的重复行里不影响排序；总体保持"最新催化在前"。
    let _ = &mut invalid;
    Ok((scanned, ordered))
}

/// 把候选写进观察池。`dry_run=true` 只打印。
pub fn build_watch_pool(
    db: &DatabaseManager,
    business_date: &str,
    lookback_days: Option<i64>,
    dry_run: bool,
) -> Result<WatchPoolReport, String> {
    let lookback = lookback_days.unwrap_or(DEFAULT_LOOKBACK_DAYS);
    let (scanned_rows, candidates) = collect_news_catalysts(db, business_date, lookback)?;
    let mut report = WatchPoolReport {
        scanned_rows,
        candidates: candidates.len(),
        dry_run,
        ..Default::default()
    };

    if dry_run {
        for item in &candidates {
            println!(
                "[dry-run] {code} hits={hits} latest={latest} | {note}",
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
                    "新闻催化 +{} 条, 最新 {} | {}",
                    item.hits, item.latest_at, item.note
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
                    "新闻催化 {} 条 (window={}d), 最新 {} | {}",
                    item.hits, lookback, item.latest_at, item.note
                );
                match ticket_lifecycle::open_ticket(
                    &mut *conn,
                    &item.code,
                    business_date,
                    "observing",
                    "news_catalyst",
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
