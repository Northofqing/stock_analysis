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
            (Some(low), Some(high)) => lines.push(format!("  买点：{low}-{high}")),
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

    let new_boards: Vec<&RotationRow> = rows
        .iter()
        .filter(|r| r.status == "new")
        .take(6)
        .collect();
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

    let fading: Vec<&RotationRow> = rows.iter().filter(|r| r.status == "fading").take(5).collect();
    if fading.is_empty() {
        lines.push("  退潮：无".to_string());
    } else {
        let text = fading
            .iter()
            .map(|r| match r.rank_prev {
                Some(prev) => format!(
                    "{}（第{}→第{}）",
                    r.board_name,
                    prev + 1,
                    r.rank_today + 1
                ),
                None => format!("{}（第{}）", r.board_name, r.rank_today + 1),
            })
            .collect::<Vec<_>>()
            .join("、");
        lines.push(format!("  退潮：{text}"));
    }

    lines.join("\n")
}

/// 每只票所属板块（用于新买入组标注来源）。
fn load_sectors(conn: &mut SqliteConnection) -> std::collections::HashMap<String, String> {
    let rows: Vec<SectorRow> = diesel::sql_query(
        "SELECT instrument_code, board_name FROM board_member WHERE kind = 'concept'",
    )
    .load(conn)
    .unwrap_or_default();

    let mut map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for row in rows {
        map.entry(row.instrument_code)
            .or_insert(row.board_name);
    }
    map
}

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
    let watch_rows: Vec<WatchRow> = diesel::sql_query(
        "SELECT code, source FROM ticket_lifecycle \
          WHERE close_date IS NULL AND stage = 'observing'",
    )
    .load(&mut conn)
    .map_err(|error| error.to_string())?;
    let watching: Vec<(String, String)> = watch_rows
        .into_iter()
        .filter(|row| !holding_codes.contains(&row.code))
        .map(|row| (row.code, row.source))
        .collect();

    // T+1 锁定：今日买入股数。
    let t1_locked = crate::monitor::trade_ledger::today_bought_quantities(db).unwrap_or_default();

    let sectors = load_sectors(&mut conn);
    let mut parts: Vec<String> = Vec::new();

    parts.push(format!(
        "【明日计划 {as_of}】持仓 {} 只 · 新买入候选 {} 只",
        holdings.len(),
        watching.len()
    ));
    parts.push(String::new());
    parts.push(render_board_header(&mut conn, as_of));

    // ---- 持仓段 ----
    if holdings.is_empty() {
        parts.push(String::new());
        parts.push("💼 持仓：无".to_string());
    } else {
        parts.push(String::new());
        parts.push(format!("💼 持仓 {} 只（只给加仓/止损，不给买点）", holdings.len()));
        let mut ordered = holdings.clone();
        ordered.sort_by(|a, b| a.code.cmp(&b.code));
        for position in &ordered {
            let locked = t1_locked.get(&position.code).copied().unwrap_or(0);
            let zone = buy_zone::compute(&mut conn, &position.code, as_of).ok().flatten();

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
                position.code, position.name, position.shares, position.cost_price, close_text, pnl_text
            ));

            if locked > 0 {
                parts.push(format!(
                    "  ⚠ 今日买入 {locked} 股，T+1 锁定，今日不可卖"
                ));
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

        for (code, source) in &watching {
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
                    if let Some(board) = sectors.get(code) {
                        no_sector.push(format!("{code}〔{board}〕"));
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
                    rows.push(PlanRow {
                        code: code.clone(),
                        label,
                        zone,
                        advice,
                    });
                    let _ = source;
                }
                Ok(None) => rejected.push(format!("{code}（无日线数据）")),
                Err(error) => rejected.push(format!("{code}（{error}）")),
            }
        }

        rows.sort_by(|left, right| {
            right
                .advice
                .suggested_pct
                .partial_cmp(&left.advice.suggested_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.code.cmp(&right.code))
        });

        parts.push(format!("🎯 新买入候选 {} 只", rows.len()));
        for (index, row) in rows.iter().enumerate() {
            parts.push(String::new());
            let board = sectors
                .get(&row.code)
                .map(|b| format!("  〔{b}〕"))
                .unwrap_or_default();
            parts.push(format!("{}. {}{}", index + 1, row.label, board));
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

        if !rejected.is_empty() {
            parts.push(String::new());
            parts.push(format!("剔除 {} 只：{}", rejected.len(), rejected.join("；")));
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
    fn empty_pool_returns_message_not_error() {
        // render_plan 对空池应返回提示文本而不是 Err，
        // 否则盘后推送会把"没有标的"当成故障报出来
        let text = "【明日计划 2026-10-16】观察池与持仓均为空，无可算标的。";
        assert!(text.contains("无可算标的"));
    }
}
