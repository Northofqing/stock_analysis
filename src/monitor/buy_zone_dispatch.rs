//! 盘后明日计划（2026-10-18 起，2026-10-19 重写）。
//!
//! ## 为什么做这个
//!
//! 买点规则（`buy_zone`）、仓位规则（`position_sizing`）、买点超买门
//! 全部做完了，但只有 CLI 能看。盘后不会自动收到任何东西——
//! 规则做完了却没人用，等于没做。
//!
//! ## 时间选择 16:00
//!
//! 收盘 15:00。等 15:50 价格带、15:55 均线都落库后，16:00 数据齐了再算。
//! 算出来的东西是"明日计划"——今天收盘定的价位，明天开盘照着执行。
//!
//! ## 2026-10-19 重写：从"清单"改成"决策"
//!
//! 之前把持仓票和观察池票混成一个列表，每只都给"买点"，等于让操作者
//! 去追自己已经持有的票，也看不出今天该买谁。现在分三段：
//!
//! 1. **板块**：当日主线/新进/退潮（`board_rotation_signal`）
//! 2. **持仓**：续持 + 加仓价 + 止损 + T+1 锁定，不给"买点"
//! 3. **新买入**：观察池票，给买点区间 + 来源板块
//!
//! 另外两条硬规则：
//! - **买点 >= 放弃价的直接剔除**：矛盾的建议不能推给人执行
//! - **已清仓的票不出现**：只读 `status='open'` 与 `close_date IS NULL`
//!
//! ## 不输出什么
//!
//! 不给"买/不买"的绝对结论。规则只回答"什么价位算合理"，判断仍由人做。

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Integer, Nullable, Text};

use crate::database::DatabaseManager;
use crate::monitor::buy_zone;
use crate::monitor::position_sizing;

#[derive(Debug, Clone)]
pub struct PlanRow {
    pub code: String,
    pub label: String,
    pub zone: buy_zone::BuyZone,
    pub advice: position_sizing::SizingAdvice,
    /// 提案日（= 次日「昨高」）的最高价；只用于突破确认参考，缺失就不写。
    pub prev_high: Option<f64>,
}

/// 突破确认行的文案。两个价格是两种执行方式：买点是回踩区间，
/// 突破确认是站上昨高才追；缺失时明写缺失，不用 0 代替（BR-222）。
fn breakout_confirmation_line(prev_high: Option<f64>) -> String {
    match prev_high {
        Some(value) => format!("  突破确认：站上 {value:.2} 才追"),
        None => "  突破确认：缺失（无日线）".to_string(),
    }
}

/// 截至 `as_of` 的最后一根日线最高价；下一交易日触发时它就是「昨高」。
fn prev_high_on_or_before(conn: &mut SqliteConnection, code: &str, as_of: &str) -> Option<f64> {
    #[derive(QueryableByName)]
    struct HighRow {
        #[diesel(sql_type = Nullable<Double>)]
        high: Option<f64>,
    }
    diesel::sql_query(
        "SELECT high FROM stock_daily \
          WHERE code = ?1 AND date <= ?2 AND high IS NOT NULL \
          ORDER BY date DESC LIMIT 1",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(as_of)
    .get_result::<HighRow>(conn)
    .optional()
    .ok()
    .flatten()
    .and_then(|row| row.high)
}

impl PlanRow {
    /// 观察池票的渲染（新买入）。
    pub fn render(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        lines.push(format!("{}  收盘 {}", self.label, self.zone.close));

        let ma_text = match (self.zone.ma5, self.zone.ma10, self.zone.ma20) {
            (Some(a), Some(b), Some(c)) => format!("ma5 {a} / ma10 {b} / ma20 {c}"),
            _ => "均线不全".to_string(),
        };
        lines.push(format!("  位置：{} · {}", self.zone.trend.label(), ma_text));

        match (self.zone.entry_low, self.zone.entry_high) {
            (Some(low), Some(high)) => {
                lines.push(format!("  买点：{low}-{high}"));
                lines.push(breakout_confirmation_line(self.prev_high));
            }
            _ if self.zone.overheated => {
                lines.push("  买点：不给（超买/异动，不参与）".to_string())
            }
            _ if self.zone.at_support => {
                lines.push("  买点：不给（已在 20 日低点附近，不追）".to_string())
            }
            _ => lines.push("  买点：不给（均线数据不足）".to_string()),
        }

        if !self.zone.trigger.is_empty() {
            lines.push(format!("  条件：{}", self.zone.trigger));
        }
        if let Some(abandon) = self.zone.abandon {
            lines.push(format!("  放弃：跌破 {abandon}"));
        }

        if self.advice.suggested_pct > 0.0 {
            lines.push(format!(
                "  仓位：建议 {:.0}%（上限 {:.0}%），止损距离 {}",
                self.advice.suggested_pct * 100.0,
                self.advice.max_pct * 100.0,
                self.advice
                    .stop_distance_pct
                    .map(|value| format!("{:.1}%", value * 100.0))
                    .unwrap_or_else(|| "—".to_string())
            ));
        } else {
            lines.push(format!("  仓位：不建议（{}）", self.advice.basis));
        }

        lines.join("\n")
    }
}

#[derive(QueryableByName)]
struct CodeRow {
    #[diesel(sql_type = Text)]
    code: String,
}

#[derive(QueryableByName)]
struct WatchRow {
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = Text)]
    open_date: String,
}

#[derive(QueryableByName)]
struct RotationRow {
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = Text)]
    board_name: String,
    #[diesel(sql_type = BigInt)]
    rank_today: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    rank_prev: Option<i64>,
    #[diesel(sql_type = Nullable<Double>)]
    pct_today: Option<f64>,
}

#[derive(QueryableByName)]
struct SectorRow {
    #[diesel(sql_type = Text)]
    instrument_code: String,
    #[diesel(sql_type = Text)]
    board_name: String,
}

/// 排雷摘要（最近 5 天的 `risk_flag` 事件，每票取最新一条）。
#[derive(QueryableByName)]
struct RiskRow {
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Nullable<Text>)]
    summary: Option<String>,
}

/// 只取板块名（退潮板块集合用；`SectorRow` 两列都有，不能混用）。
#[derive(QueryableByName)]
struct BoardNameRow {
    #[diesel(sql_type = Text)]
    board_name: String,
}

/// 最近 `bars` 根日线的最低价（移动止盈线用）。数据不足返回 None（不补值）。
fn recent_low(conn: &mut SqliteConnection, code: &str, as_of: &str, bars: i64) -> Option<f64> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Nullable<Double>)]
        low: Option<f64>,
    }
    diesel::sql_query(
        "SELECT MIN(low) AS low FROM ( \
           SELECT low FROM stock_daily \
            WHERE code = ?1 AND date <= ?2 AND low IS NOT NULL \
            ORDER BY date DESC LIMIT ?3)",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(as_of)
    .bind::<BigInt, _>(bars)
    .get_result::<Row>(conn)
    .ok()
    .and_then(|row| row.low)
}

/// 当日板块头部：主线 / 新进 / 退潮。
///
/// 数据来自 `board_rotation_signal`（15:34 调度写入）。没有数据时明确写
/// 「板块数据缺失」，不留空白让人以为没有板块行情。
fn render_board_header(conn: &mut SqliteConnection, as_of: &str) -> String {
    let rows: Vec<RotationRow> = match diesel::sql_query(
        "SELECT status, board_name, rank_today, rank_prev, pct_today \
           FROM board_rotation_signal WHERE trade_date = ? \
          ORDER BY status, rank_today",
    )
    .bind::<Text, _>(as_of.to_owned())
    .load(conn)
    {
        Ok(rows) => rows,
        Err(error) => return format!("📊 板块：读取失败（{error}）"),
    };

    if rows.is_empty() {
        return "📊 板块：今日无轮动数据（板块榜或轮动信号未生成）".to_string();
    }

    let fmt_pct = |value: Option<f64>| {
        value
            .map(|v| format!("{v:+.2}%"))
            .unwrap_or_else(|| "—".to_string())
    };

    let mut lines: Vec<String> = vec!["📊 板块".to_string()];

    let mainline: Vec<&RotationRow> = rows.iter().filter(|r| r.status == "mainline").collect();
    if mainline.is_empty() {
        lines.push("  主线：无（没有板块连续两天留在前 10）".to_string());
    } else {
        let text = mainline
            .iter()
            .map(|r| {
                format!(
                    "{} {}（第{}）",
                    r.board_name,
                    fmt_pct(r.pct_today),
                    r.rank_today + 1
                )
            })
            .collect::<Vec<_>>()
            .join("、");
        lines.push(format!("  主线：{text}"));
    }

    let new_boards: Vec<&RotationRow> = rows.iter().filter(|r| r.status == "new").take(6).collect();
    if new_boards.is_empty() {
        lines.push("  新进：无".to_string());
    } else {
        let text = new_boards
            .iter()
            .map(|r| format!("{} {}", r.board_name, fmt_pct(r.pct_today)))
            .collect::<Vec<_>>()
            .join("、");
        lines.push(format!("  新进：{text}"));
    }

    let fading: Vec<&RotationRow> = rows
        .iter()
        .filter(|r| r.status == "fading")
        .take(5)
        .collect();
    if fading.is_empty() {
        lines.push("  退潮：无".to_string());
    } else {
        let text = fading
            .iter()
            .map(|r| match r.rank_prev {
                Some(prev) => format!("{}（第{}→第{}）", r.board_name, prev + 1, r.rank_today + 1),
                None => format!("{}（第{}）", r.board_name, r.rank_today + 1),
            })
            .collect::<Vec<_>>()
            .join("、");
        lines.push(format!("  退潮：{text}"));
    }

    lines.join("\n")
}

/// 每只票所属的板块集合（用于 C 规则：板块退潮即退出候选）。
///
/// 不再按 `kind='concept'` 过滤——`board_ranking_daily` 的榜单里概念与行业
/// 混排，只查 concept 会让行业票永远匹配不到 top10。
fn load_sectors(conn: &mut SqliteConnection) -> std::collections::HashMap<String, Vec<String>> {
    let rows: Vec<SectorRow> =
        diesel::sql_query("SELECT instrument_code, board_name FROM board_member")
            .load(conn)
            .unwrap_or_default();

    let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for row in rows {
        map.entry(row.instrument_code)
            .or_default()
            .push(row.board_name);
    }
    map
}

/// 当日榜单 top N 的板块名（C 规则的「仍在榜」集合）。
fn active_boards(conn: &mut SqliteConnection, as_of: &str, top_n: i64) -> Vec<String> {
    #[derive(QueryableByName)]
    struct NameRow {
        #[diesel(sql_type = Text)]
        board_name: String,
    }
    let rows: Vec<NameRow> = diesel::sql_query(
        "SELECT board_name FROM board_ranking_daily \
          WHERE trade_date = ? ORDER BY rank_position LIMIT ?",
    )
    .bind::<Text, _>(as_of.to_owned())
    .bind::<BigInt, _>(top_n)
    .load(conn)
    .unwrap_or_default();
    rows.into_iter().map(|row| row.board_name).collect()
}

/// `open_date` 到 `as_of` 之间经过的交易日数（B 规则 TTL 用）。
///
/// 用 `board_ranking_daily` 的 distinct trade_date 当日历——榜单每天盘后
/// 必写，比再建一张交易日表可靠；查不到时返回 0（不误杀）。
fn trading_days_since(conn: &mut SqliteConnection, open_date: &str, as_of: &str) -> i64 {
    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        n: i64,
    }
    let rows: Vec<CountRow> = diesel::sql_query(
        "SELECT COUNT(*) AS n FROM (SELECT DISTINCT trade_date \
           FROM board_ranking_daily WHERE trade_date > ? AND trade_date <= ?)",
    )
    .bind::<Text, _>(open_date.to_owned())
    .bind::<Text, _>(as_of.to_owned())
    .load(conn)
    .unwrap_or_default();
    rows.first().map(|row| row.n).unwrap_or(0)
}

/// 观察池候选的最长存活交易日数（B 规则）。
///
/// 板块轮动快时票可能在买点出现前就被 C 规则移出；给 3 个交易日让票发酵，
/// 超过仍未触发买点就退出候选，避免池子无限膨胀。
const WATCH_TTL_TRADING_DAYS: i64 = 3;

/// 板块榜取前多少名算「仍在榜」（C 规则）。
const ACTIVE_BOARD_TOP_N: i64 = 10;

/// 观察池「仍可展示」的行：只有最新一次提案生命周期事件仍是
/// `entry_proposed` 才保留。触发/失效/到期/被取代的票不能再以新买入候选出现；
/// `observing` 且没有提案事件的票不受影响。
const ACTIVE_WATCH_ROWS_SQL: &str = "\
WITH latest_entry AS (
    SELECT e.ticket_id, e.event_kind,
           ROW_NUMBER() OVER (
               PARTITION BY e.ticket_id
               ORDER BY e.event_at DESC, e.id DESC
           ) AS rn
      FROM ticket_event e
     WHERE e.event_kind IN ('entry_proposed','entry_triggered','entry_invalidated',
                            'entry_expired','entry_superseded')
)
SELECT t.code, MIN(t.source) AS source, MIN(t.open_date) AS open_date
  FROM ticket_lifecycle t
 WHERE t.close_date IS NULL
   AND t.stage IN ('observing','entry_proposed')
   AND NOT EXISTS (
       SELECT 1 FROM latest_entry l
        WHERE l.ticket_id = t.id
          AND l.rn = 1
          AND l.event_kind <> 'entry_proposed'
   )
 GROUP BY t.code";

/// 生成明日计划文本。空池返回提示文本（不是错误）。
pub fn render_plan(db: &DatabaseManager, as_of: &str) -> Result<String, String> {
    let mut conn = db.get_conn().map_err(|error| error.to_string())?;

    // 持仓：只读 open。`load_positions` 已按 status 过滤，这里不再重复判断
    // （`PositionStatus` 只有 Holding/Watching，没有 Open 变体）。
    let holdings: Vec<crate::portfolio::Position> =
        crate::portfolio::load_positions().unwrap_or_default();
    let holding_codes: std::collections::HashSet<String> =
        holdings.iter().map(|p| p.code.clone()).collect();

    // 观察池：active ticket 里排除已持仓的（持仓走另一段）。
    // 2026-10-22: 按 code 去重——同票跨日重复入选时（历史遗留的多活跃票）
    // 不能在同一份计划里出现两行；取最早 open_date 作为「首次入池」上下文。
    let watch_rows: Vec<WatchRow> = diesel::sql_query(ACTIVE_WATCH_ROWS_SQL)
        .load(&mut conn)
        .map_err(|error| error.to_string())?;
    let watching: Vec<(String, String, String)> = watch_rows
        .into_iter()
        .filter(|row| !holding_codes.contains(&row.code))
        .map(|row| (row.code, row.source, row.open_date))
        .collect();

    // T+1 锁定：今日买入股数。
    let t1_locked = crate::monitor::trade_ledger::today_bought_quantities(db).unwrap_or_default();

    let sectors = load_sectors(&mut conn);
    // 批 2.8（拓 4.1）：当日退潮板块名。持仓/观察票若属于这些板块，正文逐票标注
    // 「所属板块退潮」——这是「板块还在不在」的直接证据，不额外算分。
    let fading_boards: std::collections::HashSet<String> = diesel::sql_query(
        "SELECT board_name FROM board_rotation_signal \
          WHERE trade_date = ? AND status = 'fading'",
    )
    .bind::<Text, _>(as_of)
    .load::<BoardNameRow>(&mut conn)
    .map(|rows| rows.into_iter().map(|row| row.board_name).collect())
    .unwrap_or_default();
    // 批 2.7：最近 5 天的排雷标注（15:52 写入）。只标注，不参与买点与仓位计算。
    let risk_flags: std::collections::HashMap<String, String> = diesel::sql_query(
        "SELECT t.code AS code, e.summary AS summary \
           FROM ticket_event e JOIN ticket_lifecycle t ON t.id = e.ticket_id \
          WHERE e.event_kind = 'risk_flag' AND date(e.event_at) >= date(?1, '-5 day') \
          ORDER BY e.event_at ASC, e.id ASC",
    )
    .bind::<Text, _>(as_of)
    .load::<RiskRow>(&mut conn)
    .map(|rows| {
        rows.into_iter()
            .filter_map(|row| row.summary.map(|summary| (row.code, summary)))
            .collect()
    })
    .unwrap_or_default();
    // C 规则：今日榜单 top10 的板块名，票所属板块全不在其中 → 退出候选。
    let active_set: std::collections::HashSet<String> =
        active_boards(&mut conn, as_of, ACTIVE_BOARD_TOP_N)
            .into_iter()
            .collect();
    let mut parts: Vec<String> = Vec::new();

    // 新买入的最终数量要等 C/B 过滤后才确定——标题只报持仓与观察池规模，
    // 真实候选数在下面的「🎯 新买入候选」行。
    parts.push(format!(
        "【明日计划 {as_of}】持仓 {} 只 · 观察池 {} 只",
        holdings.len(),
        watching.len()
    ));
    parts.push(String::new());
    parts.push(render_board_header(&mut conn, as_of));

    // 批 3.1（2026-10-24）：本周预算。口径 C：快照差分 / 台账买入取较大者。
    // 拿不到总资产时只报「不可用」，不按预算排序、不猜数（BR-222）。
    let budget = crate::monitor::weekly_budget::load(db, as_of).ok();
    match &budget {
        Some(state) => parts.push(state.summary_line()),
        None => parts.push(
            "💰 本周预算：不可用（缺少账户总资产或持仓快照），本轮不按预算排队".to_string(),
        ),
    }

    // ---- 持仓段 ----
    if holdings.is_empty() {
        parts.push(String::new());
        parts.push("💼 持仓：无".to_string());
    } else {
        parts.push(String::new());
        parts.push(format!(
            "💼 持仓 {} 只（只给加仓/止损，不给买点）",
            holdings.len()
        ));
        let mut ordered = holdings.clone();
        ordered.sort_by(|a, b| a.code.cmp(&b.code));
        for position in &ordered {
            let locked = t1_locked.get(&position.code).copied().unwrap_or(0);
            let zone = buy_zone::compute(&mut conn, &position.code, as_of)
                .ok()
                .flatten();

            parts.push(String::new());
            let close_text = zone
                .as_ref()
                .map(|z| format!("{:.2}", z.close))
                .unwrap_or_else(|| "—".to_string());
            let pnl_text = zone
                .as_ref()
                .map(|z| format!("{:+.1}%", (z.close / position.cost_price - 1.0) * 100.0))
                .unwrap_or_else(|| "—".to_string());
            parts.push(format!(
                "{} {}  {} 股  成本 {:.3}  现价 {}  浮盈亏 {}",
                position.code,
                position.name,
                position.shares,
                position.cost_price,
                close_text,
                pnl_text
            ));

            if locked > 0 {
                parts.push(format!("  ⚠ 今日买入 {locked} 股，T+1 锁定，今日不可卖"));
            }
            if let Some(flag) = risk_flags.get(&position.code) {
                parts.push(format!("  ⚠ 有雷：{flag}（标注，不挡操作）"));
            }

            if let Some(boards) = sectors.get(&position.code) {
                let faded: Vec<&String> = boards
                    .iter()
                    .filter(|board| fading_boards.contains(board.as_str()))
                    .collect();
                if !faded.is_empty() {
                    parts.push(format!(
                        "  ⚠ 所属板块退潮：{}",
                        faded
                            .iter()
                            .map(|board| board.as_str())
                            .collect::<Vec<_>>()
                            .join("、")
                    ));
                }
            }

            if let Some(zone) = &zone {
                // 加仓价：现价回踩 3% 以内，且不低于 20 日低点。
                let add_low = (zone.close * 0.97).max(zone.low_20d.unwrap_or(0.0));
                parts.push(format!(
                    "  加仓价：{:.2}-{:.2}（回踩不破 20 日低点）",
                    add_low, zone.close
                ));
                if let Some(abandon) = zone.abandon {
                    parts.push(format!("  止损：跌破 {abandon}"));
                }
                // 批 2.10（拓 3.1）：止盈侧只给参考线，不做自动卖。
                if let Some(high) = zone.high_20d {
                    parts.push(format!("  目标价：{high:.2}（20 日最高，参考）"));
                }
                if let Some(low5) = recent_low(&mut conn, &position.code, as_of, 5) {
                    let trail = low5.max(position.cost_price);
                    parts.push(format!(
                        "  移动止盈：跌破 {trail:.2} 提示（成本与近 5 日低取高）"
                    ));
                }
            } else {
                parts.push("  加仓价：不给（日线数据缺失）".to_string());
            }
        }
    }

    // ---- 新买入段 ----
    parts.push(String::new());
    if watching.is_empty() {
        parts.push("🎯 新买入：观察池为空".to_string());
    } else {
        let mut rows: Vec<PlanRow> = Vec::new();
        let mut rejected: Vec<String> = Vec::new();
        let mut no_entry: Vec<String> = Vec::new();
        let mut no_sector: Vec<String> = Vec::new();
        let mut dropped: Vec<String> = Vec::new();
        let mut expired: Vec<String> = Vec::new();

        for (code, source, open_date) in &watching {
            // B 规则 TTL：开票后超过 N 个交易日仍未触发买点 → 退出候选。
            let used_days = trading_days_since(&mut conn, open_date, as_of);
            if used_days > WATCH_TTL_TRADING_DAYS {
                expired.push(format!(
                    "{code}（观察 {used_days} 个交易日未触发买点，超过 {WATCH_TTL_TRADING_DAYS} 日上限）"
                ));
                continue;
            }
            // C 规则：所属板块全部退出当日 top N → 退出候选。
            // 新闻催化票不受板块约束（它的理由是新闻，不是板块轮动）。
            if source.as_str() != "news_catalyst" {
                // 查不到板块记录时**保留**：`board_member` 只覆盖成分拉取成功的
                // 板块（今天 top10 里水域改革/鸿蒙概念/基因测序/科创50 四个拉取
                // 失败），把「查不到」当成「已退潮」会误杀这些板块的票。
                let still_active = sectors
                    .get(code)
                    .map(|boards| boards.iter().any(|board| active_set.contains(board)))
                    .unwrap_or(true);
                if !still_active {
                    // 2.2：把「理由失效」记进票的事件流（只标注，不改 stage）。
                    // C 规则是唯一触发点，避免第三条判据。
                    if let Some(boards) = sectors.get(code) {
                        let _ = crate::database::ticket_lifecycle::mark_thesis_invalidated(
                            &mut conn,
                            code,
                            as_of,
                            boards,
                        );
                    }
                    dropped.push(code.clone());
                    continue;
                }
            }
            match buy_zone::compute(&mut conn, code, as_of) {
                Ok(Some(zone)) => {
                    // 正常关系：**放弃价 < 买点下限**——跌到放弃价以下就不买了，
                    // 在买点区间里买。矛盾只有一种：放弃价 >= 买点下限，
                    // 意思是还没到买点就已经该放弃（601811 那种）。
                    if let (Some(low), Some(_high), Some(abandon)) =
                        (zone.entry_low, zone.entry_high, zone.abandon)
                    {
                        if abandon >= low {
                            rejected.push(format!(
                                "{code}（放弃价 {abandon} 不低于买点下限 {low}，口径矛盾）"
                            ));
                            continue;
                        }
                    }
                    if let Some(boards) = sectors.get(code) {
                        if let Some(board) = boards.first() {
                            no_sector.push(format!("{code}〔{board}〕"));
                        }
                    }
                    // 没有买点的票不进正文：它们不可能被买入，逐只展开是噪音。
                    // 归进"不建议买入"汇总，写清原因。
                    if zone.entry_low.is_none() {
                        let why = if zone.overheated {
                            "超买/异动".to_string()
                        } else if zone.at_support {
                            "已在 20 日低点".to_string()
                        } else {
                            "均线数据不足".to_string()
                        };
                        no_entry.push(format!("{code}（{why}）"));
                        continue;
                    }
                    let advice = position_sizing::advise(&zone.display_name(), &zone);
                    let label = zone.display_name();
                    let prev_high = prev_high_on_or_before(&mut conn, code, as_of);
                    rows.push(PlanRow {
                        code: code.clone(),
                        label,
                        zone,
                        advice,
                        prev_high,
                    });
                    let _ = source;
                }
                Ok(None) => rejected.push(format!("{code}（无日线数据）")),
                Err(error) => rejected.push(format!("{code}（{error}）")),
            }
        }

        // 批 2.11（拓 8）：按 `ticket_priority` 五维加权分数排序，而不是按建议仓位。
        // 仓位是「该放多少钱」，排序该回答「先买谁」。取不到分数（例如当日 15:44
        // 的优先级还没跑）时退回建议仓位排序，并在正文写明。
        let priority: std::collections::HashMap<String, f64> =
            match crate::monitor::ticket_priority::rank_watch_pool(db, as_of) {
                Ok(rows) => rows.into_iter().map(|row| (row.code, row.total)).collect(),
                Err(_) => std::collections::HashMap::new(),
            };
        let priority_used = rows.iter().any(|row| priority.contains_key(&row.code));
        rows.sort_by(|left, right| {
            let left_score = priority.get(&left.code).copied();
            let right_score = priority.get(&right.code).copied();
            right_score
                .partial_cmp(&left_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    right
                        .advice
                        .suggested_pct
                        .partial_cmp(&left.advice.suggested_pct)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| left.code.cmp(&right.code))
        });

        parts.push(format!("🎯 新买入候选 {} 只", rows.len()));
        if priority_used {
            parts.push("按优先级分数排序（①最强），仓位另列。".to_string());
        }

        // 批 3.2/3.3（2026-10-24）：预算与板块集中度。
        // 逐票算占用 = 建议仓位 × 总资产；超「本周剩余」或「板块上限（总资产 15%）」
        // 的票标排队并挪到后面 —— 只标注与排序，不挡买入。
        let mut budget_state: std::collections::HashMap<String, (f64, bool, f64)> =
            std::collections::HashMap::new();
        if let Some(state) = &budget {
            let mut remaining = state.remaining();
            let mut board_extra: std::collections::HashMap<String, f64> =
                std::collections::HashMap::new();
            for row in &rows {
                let cost = row.advice.suggested_pct / 100.0 * state.total_assets;
                let board = sectors.get(&row.code).and_then(|boards| boards.first());
                let board_used = board
                    .map(|name| {
                        state.board_used(name) + board_extra.get(name).copied().unwrap_or(0.0)
                    })
                    .unwrap_or(0.0);
                let over_budget = cost > remaining;
                let over_board = board
                    .map(|_| board_used + cost > state.board_cap())
                    .unwrap_or(false);
                let queued = over_budget || over_board;
                if !queued {
                    remaining -= cost;
                    if let Some(name) = board {
                        *board_extra.entry(name.clone()).or_insert(0.0) += cost;
                    }
                }
                budget_state.insert(row.code.clone(), (cost, queued, remaining));
            }
        }
        let ordered: Vec<&PlanRow> = rows
            .iter()
            .filter(|row| !budget_state.get(&row.code).map(|state| state.1).unwrap_or(false))
            .chain(
                rows.iter()
                    .filter(|row| budget_state.get(&row.code).map(|state| state.1).unwrap_or(false)),
            )
            .collect();
        for (index, row) in ordered.into_iter().enumerate() {
            parts.push(String::new());
            let board = sectors
                .get(&row.code)
                .and_then(|boards| boards.first())
                .map(|b| format!("  〔{b}〕"))
                .unwrap_or_default();
            // ①②③ 标建议顺序；第四名之后用数字（圆圈字符只到 ③ 好读）。
            let marker = match index {
                0 => "①".to_string(),
                1 => "②".to_string(),
                2 => "③".to_string(),
                other => format!("{}.", other + 1),
            };
            let score_text = priority
                .get(&row.code)
                .map(|score| format!("  优先级 {score:.1}"))
                .unwrap_or_default();
            parts.push(format!("{marker} {}{}{score_text}", row.label, board));
            if let Some(flag) = risk_flags.get(&row.code) {
                parts.push(format!("  ⚠ 有雷：{flag}（标注，不挡买入）"));
            }
            if let Some((cost, queued, remaining)) = budget_state.get(&row.code) {
                if *queued {
                    parts.push(format!(
                        "  ⏸ 板块集中度高/超预算·排队（本票占用 {cost:.0}，本周剩余 {remaining:.0}）"
                    ));
                } else {
                    parts.push(format!(
                        "  占用预算 {cost:.0}（本周剩余 {remaining:.0}）"
                    ));
                }
            }
            // 去掉第一行（重复的 label + 收盘），其余照旧。
            let body = row.render();
            let mut lines = body.lines();
            lines.next();
            for line in lines {
                parts.push(line.to_string());
            }
        }

        if !no_entry.is_empty() {
            parts.push(String::new());
            parts.push(format!(
                "不建议买入 {} 只：{}",
                no_entry.len(),
                no_entry.join("、")
            ));
        }

        if !dropped.is_empty() {
            parts.push(String::new());
            let preview = if dropped.len() > 6 {
                format!(
                    "{} 等",
                    dropped
                        .iter()
                        .take(6)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("、")
                )
            } else {
                dropped.join("、")
            };
            parts.push(format!(
                "板块退出 top{ACTIVE_BOARD_TOP_N} 移出 {} 只：{preview}",
                dropped.len()
            ));
        }

        if !expired.is_empty() {
            parts.push(String::new());
            parts.push(format!(
                "观察超期 {} 只：{}",
                expired.len(),
                expired.join("、")
            ));
        }

        if !rejected.is_empty() {
            parts.push(String::new());
            parts.push(format!(
                "剔除 {} 只：{}",
                rejected.len(),
                rejected.join("；")
            ));
        }
        let _ = no_sector;
    }

    parts.push(String::new());
    parts.push(
        "口径：买点/放弃价来自收盘后均线与 20 日区间；仓位按 2% 风险预算算，\
         受 30% 单票上限约束。持仓段只给加仓/止损，买点仅对新买入候选给出。"
            .to_string(),
    );

    Ok(parts.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakout_confirmation_line_marks_missing_without_zero() {
        assert!(breakout_confirmation_line(Some(13.88)).contains("站上 13.88"));
        assert!(breakout_confirmation_line(None).contains("缺失"));
        assert!(!breakout_confirmation_line(None).contains("0.00"));
    }

    #[test]
    fn empty_pool_returns_message_not_error() {
        // render_plan 对空池应返回提示文本而不是 Err，
        // 否则盘后推送会把"没有标的"当成故障报出来
        let text = "【明日计划 2026-10-16】观察池与持仓均为空，无可算标的。";
        assert!(text.contains("无可算标的"));
    }

    #[test]
    fn active_watch_rows_exclude_non_open_proposal_states() {
        let mut conn = diesel::SqliteConnection::establish(":memory:").expect("in-memory sqlite");
        diesel::sql_query(
            "CREATE TABLE ticket_lifecycle (\
                 id INTEGER PRIMARY KEY, code TEXT NOT NULL, source TEXT NOT NULL,\
                 open_date TEXT NOT NULL, close_date TEXT, stage TEXT NOT NULL)",
        )
        .execute(&mut conn)
        .unwrap();
        diesel::sql_query(
            "CREATE TABLE ticket_event (\
                 id INTEGER PRIMARY KEY AUTOINCREMENT, ticket_id INTEGER NOT NULL,\
                 event_at TEXT NOT NULL, event_kind TEXT NOT NULL)",
        )
        .execute(&mut conn)
        .unwrap();

        for (id, code, close_date) in [
            (1, "A", None),
            (2, "B", None),
            (3, "C", None),
            (4, "D", None),
            (5, "E", None),
            (6, "F", None),
            (7, "G", None),
            (8, "I", Some("2026-10-29")),
        ] {
            diesel::sql_query(
                "INSERT INTO ticket_lifecycle (id, code, source, open_date, close_date, stage) \
                 VALUES (?, ?, 'test', '2026-10-27', ?, \
                         CASE WHEN ? IS NULL THEN 'entry_proposed' ELSE 'exited' END)",
            )
            .bind::<diesel::sql_types::Integer, _>(id)
            .bind::<diesel::sql_types::Text, _>(code)
            .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(close_date)
            .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(close_date)
            .execute(&mut conn)
            .unwrap();
        }

        for (ticket_id, at, kind) in [
            (2, "2026-10-30 10:00:00", "entry_proposed"),
            (3, "2026-10-30 10:00:00", "entry_triggered"),
            (4, "2026-10-30 10:00:00", "entry_invalidated"),
            (5, "2026-10-30 10:00:00", "entry_expired"),
            (6, "2026-10-30 10:00:00", "entry_superseded"),
            (7, "2026-10-29 10:00:00", "entry_invalidated"),
            (7, "2026-10-30 10:00:00", "entry_proposed"),
        ] {
            diesel::sql_query(
                "INSERT INTO ticket_event (ticket_id, event_at, event_kind) VALUES (?, ?, ?)",
            )
            .bind::<diesel::sql_types::Integer, _>(ticket_id)
            .bind::<diesel::sql_types::Text, _>(at)
            .bind::<diesel::sql_types::Text, _>(kind)
            .execute(&mut conn)
            .unwrap();
        }

        let rows: Vec<WatchRow> = diesel::sql_query(ACTIVE_WATCH_ROWS_SQL)
            .load(&mut conn)
            .unwrap();
        let mut codes: Vec<&str> = rows.iter().map(|row| row.code.as_str()).collect();
        codes.sort_unstable();
        assert_eq!(codes, vec!["A", "B", "G"]);
    }
}
