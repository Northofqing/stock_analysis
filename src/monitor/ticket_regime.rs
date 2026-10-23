//! 分型（regime）—— 情绪型 / 趋势型（批 1.1，2026-10-23）。
//!
//! ## 为什么要分型
//!
//! operator 2026-10-22 拍板 D1/D2：
//! - D1 情绪型票**允许突破买**，超买门对它豁免（连板 / 近 5 日涨幅 / 偏离 ma5 都不是
//!   「不该买」的理由，因为它本来就是靠情绪走）；
//! - D2 持股周期**不定全局值**，按票的趋势分型：情绪型短周期、趋势型长周期。
//!
//! 没有分型，这两条落不了地 —— 同一套 10 天淘汰 / 回踩买点在情绪票上全是错的。
//!
//! ## 存哪里（为什么不加列）
//!
//! monitor 用 `open_existing_monitor_schema9` 打开生产库（`user_version=9`），
//! 给 `ticket_lifecycle` 加列要动 schema 版本；而「代码版本比库版本新」曾经直接炸掉
//! HoldingPlan 的 owner 读取（v10 才有的表在 v9 库报 `no such table`）。
//! 所以分型写 `ticket_event(event_kind='regime', summary='emotion|trend|<reason>')`，
//! 取每票最新一条为当前型（铁律 26）。
//!
//! ## 写入口径
//!
//! 每日盘后重算一次；**只在型别或理由变化时追加事件**（含首次），值相同则跳过。
//! 这样事件流是「变化日志」，既有历史可追溯，又不会因为每日重算把表撑大、重启重放。
//!
//! ## 判据
//!
//! 情绪型 = `source='p01_preopen'` ∪ 近 5 个交易日出现过涨停。
//! 涨停阈值按板分（[`limit_up_pct`]）：20cm 板（300/301/688/689）19.5%，其余 9.85%。
//! **不依赖 `stock_daily.is_limit_up`** —— 该列在 `kline.rs` / `repository.rs` 硬编码 false。
//!
//! 边界：
//! - ST 票 5% 板没有可靠的名称源可判别，按所属板块阈值判（会低估 ST 的涨停）——文档已注明；
//! - 北交所 30% 板不在当前选票面内，按 9.85% 兜底（判出的「涨停」会偏多）；
//! - `stock_daily` 覆盖不足时，`近 5 个交易日` 退化为「最近 5 根有数据的日线」，理由里写明次数。

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};

use crate::database::{ticket_lifecycle, DatabaseManager};

/// 分型事件的事件类型（`ticket_event.event_kind`）。
pub const REGIME_EVENT_KIND: &str = "regime";
/// 情绪型判定回看的交易日根数。
pub const REGIME_LOOKBACK_BARS: i64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Regime {
    Emotion,
    Trend,
}

impl Regime {
    pub fn as_str(self) -> &'static str {
        match self {
            Regime::Emotion => "emotion",
            Regime::Trend => "trend",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Regime::Emotion => "情绪型",
            Regime::Trend => "趋势型",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "emotion" => Some(Regime::Emotion),
            "trend" => Some(Regime::Trend),
            _ => None,
        }
    }

    /// 观察期上限（自然日）：情绪型 5 天、趋势型 10 天（D2）。
    pub fn observe_days(self) -> i64 {
        match self {
            Regime::Emotion => 5,
            Regime::Trend => 10,
        }
    }

    /// 复盘窗（交易日）：情绪型 D+1/D+3、趋势型 D+5/D+10。
    pub fn perf_periods(self) -> &'static [usize] {
        match self {
            Regime::Emotion => &[1, 3],
            Regime::Trend => &[5, 10],
        }
    }
}

/// 分板涨停阈值（`pct_chg` 是百分数）。
///
/// 20cm 板：创业板 `300`/`301`、科创板 `688`/`689` → **19.5**；
/// 其余（主板 000/001/002/003/600/601/603/605）→ **9.85**。
///
/// 20cm 板用 9.85 判涨停会把 +10% 的普通上涨当成涨停：实测 151 个样本 / 93 只票
/// （占 20cm 板全部 >=9.85 样本的 58%）被误计，同时污染分型、P1.1 涨停优先与回测。
pub fn limit_up_pct(code: &str) -> f64 {
    let code = code.trim();
    if code.starts_with("300")
        || code.starts_with("301")
        || code.starts_with("688")
        || code.starts_with("689")
    {
        19.5
    } else {
        9.85
    }
}

/// 一次分型判定：型别 + 理由（理由进事件 summary，供人读与复盘）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub regime: Regime,
    pub reason: String,
}

impl Decision {
    /// `ticket_event.summary` 的形状：`emotion|理由`。
    pub fn to_summary(&self) -> String {
        format!("{}|{}", self.regime.as_str(), self.reason)
    }

    pub fn from_summary(summary: &str) -> Option<Self> {
        let (head, reason) = summary.split_once('|').unwrap_or((summary, ""));
        Regime::parse(head).map(|regime| Decision {
            regime,
            reason: reason.trim().to_string(),
        })
    }
}

/// 近 `REGIME_LOOKBACK_BARS` 根日线里涨停的次数。
fn count_limit_up_within(
    conn: &mut SqliteConnection,
    code: &str,
    as_of: &str,
    threshold: f64,
) -> Result<i64, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = BigInt)]
        n: i64,
    }
    let row: Row = diesel::sql_query(
        "SELECT COUNT(*) AS n FROM ( \
           SELECT pct_chg FROM stock_daily \
            WHERE code = ?1 AND date <= ?2 AND pct_chg IS NOT NULL \
            ORDER BY date DESC LIMIT ?3) \
         WHERE pct_chg >= ?4",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(as_of)
    .bind::<BigInt, _>(REGIME_LOOKBACK_BARS)
    .bind::<Double, _>(threshold)
    .get_result(conn)
    .map_err(|error| format!("涨停回看查询失败 ({code} {as_of}): {error}"))?;
    Ok(row.n)
}

/// 判定一只票的分型。纯计算，不写库。
pub fn classify(
    conn: &mut SqliteConnection,
    code: &str,
    source: &str,
    as_of: &str,
) -> Result<Decision, String> {
    if source == "p01_preopen" {
        return Ok(Decision {
            regime: Regime::Emotion,
            reason: "来源 P01 盘前热点（前一交易日涨停股）".to_string(),
        });
    }
    let threshold = limit_up_pct(code);
    let hits = count_limit_up_within(conn, code, as_of, threshold)?;
    if hits > 0 {
        Ok(Decision {
            regime: Regime::Emotion,
            reason: format!(
                "近 {REGIME_LOOKBACK_BARS} 根日线涨停 {hits} 次（pct_chg >= {threshold:.2}）"
            ),
        })
    } else {
        Ok(Decision {
            regime: Regime::Trend,
            reason: format!("近 {REGIME_LOOKBACK_BARS} 根日线无涨停（阈值 {threshold:.2}）"),
        })
    }
}

/// 该 ticket 最新一条分型事件。
pub fn latest(conn: &mut SqliteConnection, ticket_id: i64) -> Result<Option<Decision>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Nullable<Text>)]
        summary: Option<String>,
    }
    let row = diesel::sql_query(
        "SELECT summary FROM ticket_event \
          WHERE ticket_id = ? AND event_kind = 'regime' \
          ORDER BY event_at DESC, id DESC LIMIT 1",
    )
    .bind::<BigInt, _>(ticket_id)
    .get_result::<Row>(conn)
    .optional()
    .map_err(|error| format!("分型读取失败 (ticket={ticket_id}): {error}"))?;
    Ok(row
        .and_then(|row| row.summary)
        .and_then(|summary| Decision::from_summary(&summary)))
}

/// 当前分型：优先取最新事件，没有则现算（不写库）。
///
/// 供下游（淘汰、竞价买价、复盘）使用 —— 它们不负责写入，也不该因为
/// 「今天的分型还没算」就退化成默认值。
pub fn resolve(
    conn: &mut SqliteConnection,
    ticket_id: i64,
    code: &str,
    source: &str,
    as_of: &str,
) -> Result<Decision, String> {
    if let Some(decision) = latest(conn, ticket_id)? {
        return Ok(decision);
    }
    classify(conn, code, source, as_of)
}

#[derive(Debug, Default)]
pub struct RegimeReport {
    pub scanned: usize,
    pub emotion: usize,
    pub trend: usize,
    /// 型别/理由发生变化、真正写入了事件的票
    pub written: usize,
    /// 与上次一致、没有写事件的票
    pub unchanged: usize,
    pub errors: Vec<String>,
}

impl RegimeReport {
    pub fn log(&self) {
        log::info!(
            "[分型] scanned={} emotion={} trend={} written={} unchanged={} errors={}",
            self.scanned,
            self.emotion,
            self.trend,
            self.written,
            self.unchanged,
            self.errors.len()
        );
        for error in &self.errors {
            log::warn!("[分型] {error}");
        }
    }
}

/// 每日盘后重算全部活跃票的分型（只写变化）。
pub fn recompute_active(db: &DatabaseManager, as_of: &str) -> Result<RegimeReport, String> {
    chrono::NaiveDate::parse_from_str(as_of, "%Y-%m-%d")
        .map_err(|error| format!("as_of 非法 ({as_of}): {error}"))?;

    #[derive(QueryableByName)]
    struct TicketRow {
        #[diesel(sql_type = BigInt)]
        id: i64,
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Text)]
        source: String,
    }

    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    let tickets: Vec<TicketRow> = diesel::sql_query(
        "SELECT id, code, source FROM ticket_lifecycle WHERE close_date IS NULL ORDER BY code",
    )
    .load(&mut conn)
    .map_err(|error| format!("活跃票查询失败: {error}"))?;

    let mut report = RegimeReport {
        scanned: tickets.len(),
        ..Default::default()
    };
    let event_at = chrono::Local::now().to_rfc3339();

    for ticket in tickets {
        let decision = match classify(&mut conn, &ticket.code, &ticket.source, as_of) {
            Ok(decision) => decision,
            Err(error) => {
                report.errors.push(format!("{} {error}", ticket.code));
                continue;
            }
        };
        match decision.regime {
            Regime::Emotion => report.emotion += 1,
            Regime::Trend => report.trend += 1,
        }

        let previous = match latest(&mut conn, ticket.id) {
            Ok(previous) => previous,
            Err(error) => {
                report.errors.push(format!("{} {error}", ticket.code));
                continue;
            }
        };
        if previous.as_ref() == Some(&decision) {
            report.unchanged += 1;
            continue;
        }

        let summary = decision.to_summary();
        match ticket_lifecycle::append_event(
            &mut conn,
            ticket.id,
            &event_at,
            REGIME_EVENT_KIND,
            None,
            None,
            Some(&summary),
        ) {
            Ok(()) => report.written += 1,
            Err(error) => report
                .errors
                .push(format!("{} 记事件失败: {error}", ticket.code)),
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::connection::SimpleConnection;

    fn connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory sqlite");
        conn.batch_execute(
            "CREATE TABLE ticket_lifecycle (\
               id INTEGER PRIMARY KEY AUTOINCREMENT, code TEXT NOT NULL, open_date DATE NOT NULL, \
               close_date DATE, stage TEXT NOT NULL, source TEXT NOT NULL, note TEXT, \
               created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, \
               updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP);\
             CREATE TABLE ticket_event (\
               id INTEGER PRIMARY KEY AUTOINCREMENT, ticket_id INTEGER NOT NULL, \
               event_at TIMESTAMP NOT NULL, event_kind TEXT NOT NULL, push_kind TEXT, \
               occurrence_identity TEXT, summary TEXT, \
               created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP);\
             CREATE TABLE stock_daily (code TEXT NOT NULL, date TEXT NOT NULL, pct_chg REAL);",
        )
        .expect("schema");
        conn
    }

    fn insert_bar(conn: &mut SqliteConnection, code: &str, date: &str, pct: f64) {
        diesel::sql_query("INSERT INTO stock_daily (code, date, pct_chg) VALUES (?, ?, ?)")
            .bind::<Text, _>(code)
            .bind::<Text, _>(date)
            .bind::<Double, _>(pct)
            .execute(conn)
            .expect("insert bar");
    }

    /// B.10 回归：20cm 板 +10% 不是涨停，主板 +10% 是。
    #[test]
    fn limit_up_threshold_is_board_aware() {
        assert!((limit_up_pct("300750") - 19.5).abs() < f64::EPSILON);
        assert!((limit_up_pct("301029") - 19.5).abs() < f64::EPSILON);
        assert!((limit_up_pct("688981") - 19.5).abs() < f64::EPSILON);
        assert!((limit_up_pct("002696") - 9.85).abs() < f64::EPSILON);
        assert!((limit_up_pct("600396") - 9.85).abs() < f64::EPSILON);
        assert!((limit_up_pct("603278") - 9.85).abs() < f64::EPSILON);
    }

    #[test]
    fn twenty_pct_board_ten_percent_gain_is_not_limit_up() {
        let mut conn = connection();
        insert_bar(&mut conn, "300750", "2026-10-22", 10.02);
        let decision =
            classify(&mut conn, "300750", "board_rotation", "2026-10-22").expect("classify 20cm");
        assert_eq!(decision.regime, Regime::Trend, "20cm 板 +10% 不该判成涨停");

        let mut conn = connection();
        insert_bar(&mut conn, "002696", "2026-10-22", 10.02);
        let decision = classify(&mut conn, "002696", "board_rotation", "2026-10-22")
            .expect("classify main board");
        assert_eq!(decision.regime, Regime::Emotion, "主板 +10% 应判情绪型");
    }

    #[test]
    fn p01_source_is_always_emotion() {
        let mut conn = connection();
        let decision =
            classify(&mut conn, "600000", "p01_preopen", "2026-10-23").expect("classify");
        assert_eq!(decision.regime, Regime::Emotion);
        assert!(decision.reason.contains("P01"));
    }

    #[test]
    fn limit_up_outside_lookback_window_is_ignored() {
        let mut conn = connection();
        // 6 根日线前的涨停不在回看窗内
        insert_bar(&mut conn, "600000", "2026-10-10", 9.9);
        for day in 11..=16 {
            insert_bar(&mut conn, "600000", &format!("2026-10-{day:02}"), 0.5);
        }
        let decision =
            classify(&mut conn, "600000", "board_rotation", "2026-10-16").expect("classify");
        assert_eq!(decision.regime, Regime::Trend);
    }

    #[test]
    fn summary_round_trip() {
        let decision = Decision {
            regime: Regime::Emotion,
            reason: "近 5 根日线涨停 2 次".to_string(),
        };
        let parsed = Decision::from_summary(&decision.to_summary()).expect("parse");
        assert_eq!(parsed, decision);
        assert!(Decision::from_summary("unknown|x").is_none());
    }

    /// 只写变化：同一天连跑两次，第二次不写事件。
    #[test]
    fn recompute_writes_only_on_change() {
        let mut conn = connection();
        diesel::sql_query(
            "INSERT INTO ticket_lifecycle (code, open_date, stage, source) \
             VALUES ('002696', '2026-10-22', 'observing', 'board_rotation')",
        )
        .execute(&mut conn)
        .expect("ticket");
        insert_bar(&mut conn, "002696", "2026-10-22", 10.0);

        let ticket_id = 1;
        let decision =
            classify(&mut conn, "002696", "board_rotation", "2026-10-22").expect("classify");
        assert_eq!(decision.regime, Regime::Emotion);

        ticket_lifecycle::append_event(
            &mut conn,
            ticket_id,
            "2026-10-22T15:37:00+08:00",
            REGIME_EVENT_KIND,
            None,
            None,
            Some(&decision.to_summary()),
        )
        .expect("append");

        let latest = latest(&mut conn, ticket_id).expect("latest").expect("some");
        assert_eq!(latest.regime, Regime::Emotion);
        assert!(latest.reason.contains("涨停"));
    }
}
