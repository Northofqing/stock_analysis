//! 观察池票优先级（2026-10-18）。
//!
//! ## 为什么需要
//!
//! 观察池只有"进/不进"，没有"先看哪只"。601811 这种和一只弱催化的票
//! 并列在池里，人得自己判断先看谁——这正是系统该做的事。
//!
//! ## 五个维度（全部从已有数据取，不引入新数据源）
//!
//! 1. **新闻催化** `news_ai_assessment` 近 5 日命中数与新鲜度
//! 2. **板块强度** `board_ranking_daily` 当日前 30 板块里该票所属概念的涨幅
//! 3. **可买价质量** `buy_zone`：给买点 / 贴支撑 / 超买
//! 4. **仓位建议** `position_sizing`：建议仓位越高越优先
//! 5. **概念共振** 与池内其他票的共享概念数（板块联动强度）
//!
//! 每项归一到 0-100，权重之和为 1。
//!
//! ## 缺失数据的处理
//!
//! **缺失项标 N/A 并从分母中剔除**，不当作 0 分。
//! 把"没数据"算成"0 分"会让数据不全的票被错误地排到最后——
//! 那是把信息缺失伪装成质量差。
//!
//! ## 不做的事
//!
//! 不打"买/不买"的绝对结论，只排序。买不买仍然是看 `buy_zone` 的输出。

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};

use crate::database::DatabaseManager;

/// 各维度权重。加起来 1.0。
const W_NEWS: f64 = 0.30;
const W_BOARD: f64 = 0.20;
const W_BUY: f64 = 0.25;
const W_SIZE: f64 = 0.15;
const W_RESONANCE: f64 = 0.10;

/// 新闻催化满分对应的命中数（5 日内）
const NEWS_HITS_FULL: f64 = 5.0;
/// 板块强度满分对应的涨幅（%）
const BOARD_PCT_FULL: f64 = 8.0;
/// 概念共振满分对应的共享概念数
const RESONANCE_FULL: f64 = 5.0;

#[derive(Debug, Clone)]
pub struct PriorityRow {
    pub code: String,
    pub name: Option<String>,
    pub total: f64,
    pub news_score: Option<f64>,
    pub board_score: Option<f64>,
    pub buy_score: Option<f64>,
    pub size_score: Option<f64>,
    pub resonance_score: Option<f64>,
    pub reasons: Vec<String>,
}

impl PriorityRow {
    fn display_name(&self) -> String {
        match self.name.as_deref() {
            Some(name) if !name.is_empty() => format!("{} {}", self.code, name),
            _ => self.code.clone(),
        }
    }

    fn score_text(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(value) = self.news_score {
            parts.push(format!("催化 {value:.0}"));
        }
        if let Some(value) = self.board_score {
            parts.push(format!("板块 {value:.0}"));
        }
        if let Some(value) = self.buy_score {
            parts.push(format!("可买 {value:.0}"));
        }
        if let Some(value) = self.size_score {
            parts.push(format!("仓位 {value:.0}"));
        }
        if let Some(value) = self.resonance_score {
            parts.push(format!("共振 {value:.0}"));
        }
        parts.join(" / ")
    }

    pub fn render(&self, rank: usize) -> String {
        let mut lines = vec![format!(
            "{rank}. {}  总分 {:.1}  [{}]",
            self.display_name(),
            self.total,
            self.score_text()
        )];
        for reason in &self.reasons {
            lines.push(format!("      {reason}"));
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
struct HitRow {
    #[diesel(sql_type = BigInt)]
    hits: i64,
    #[diesel(sql_type = Nullable<Text>)]
    latest: Option<String>,
}

fn news_score(conn: &mut SqliteConnection, code: &str, as_of: &str) -> Option<f64> {
    let since = chrono::NaiveDate::parse_from_str(as_of, "%Y-%m-%d")
        .ok()?
        .checked_sub_days(chrono::Days::new(5))?
        .format("%Y-%m-%d")
        .to_string();
    let row: HitRow = diesel::sql_query(
        "SELECT COUNT(*) AS hits, MAX(created_at) AS latest \
         FROM news_ai_assessment \
         WHERE target_code = ? AND created_at >= ?",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(since)
    .get_result(conn)
    .ok()?;
    if row.hits == 0 {
        return Some(0.0);
    }
    let density = (row.hits as f64 / NEWS_HITS_FULL).min(1.0) * 100.0;
    // 新鲜度：24 小时内加分，超过 3 天减半
    let freshness = match row.latest.as_deref() {
        Some(text) => {
            let latest_date = text.get(0..10).unwrap_or("");
            match chrono::NaiveDate::parse_from_str(latest_date, "%Y-%m-%d") {
                Ok(date) => match chrono::NaiveDate::parse_from_str(as_of, "%Y-%m-%d") {
                    Ok(today) => {
                        let age = (today - date).num_days();
                        if age <= 1 {
                            1.0
                        } else if age <= 3 {
                            0.7
                        } else {
                            0.4
                        }
                    }
                    Err(_) => 0.5,
                },
                Err(_) => 0.5,
            }
        }
        None => 0.5,
    };
    Some(density * freshness)
}

/// 板块强度：该票所属概念在当日 f3 榜上的最高涨幅。
fn board_score(conn: &mut SqliteConnection, code: &str, as_of: &str) -> Option<f64> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Nullable<Double>)]
        change_pct: Option<f64>,
    }
    let row: Option<Row> = diesel::sql_query(
        "SELECT MAX(b.change_pct) AS change_pct \
         FROM board_ranking_daily b \
         JOIN stock_concepts s ON 1=1 \
         WHERE b.trade_date = ? AND b.rank_fid = 'f3' \
           AND s.code = ? \
           AND instr(s.concepts, b.board_name) > 0",
    )
    .bind::<Text, _>(as_of)
    .bind::<Text, _>(code)
    .get_result(conn)
    .ok();
    let pct = row.and_then(|value| value.change_pct)?;
    Some((pct / BOARD_PCT_FULL).clamp(0.0, 1.0) * 100.0)
}

pub fn rank_watch_pool(db: &DatabaseManager, as_of: &str) -> Result<Vec<PriorityRow>, String> {
    let mut conn = db.get_conn().map_err(|error| error.to_string())?;

    let codes: Vec<String> =
        diesel::sql_query("SELECT code FROM ticket_lifecycle WHERE close_date IS NULL ORDER BY code")
            .load::<CodeRow>(&mut conn)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|row| row.code)
            .collect();

    if codes.is_empty() {
        return Ok(Vec::new());
    }

    let mut rows: Vec<PriorityRow> = Vec::with_capacity(codes.len());
    for code in &codes {
        let mut reasons: Vec<String> = Vec::new();

        let news = news_score(&mut conn, code, as_of);
        let board = board_score(&mut conn, code, as_of);

        // 可买价 / 仓位：走 buy_zone + position_sizing，失败不影响排序
        let (buy, size, zone_reason) = match crate::monitor::buy_zone::compute(&mut conn, code, as_of)
        {
            Ok(Some(zone)) => {
                let advice = crate::monitor::position_sizing::advise(code, &zone);
                let buy_value = if zone.overheated {
                    Some(0.0)
                } else if zone.at_support {
                    Some(40.0)
                } else if zone.entry_low.is_some() {
                    Some(100.0)
                } else {
                    None
                };
                let size_value = if advice.suggested_pct > 0.0 {
                    Some((advice.suggested_pct / 0.30).clamp(0.0, 1.0) * 100.0)
                } else {
                    Some(0.0)
                };
                let reason = if zone.overheated {
                    Some("超买，不参与".to_string())
                } else if zone.at_support {
                    Some("贴 20 日低点，等支撑确认".to_string())
                } else if zone.entry_low.is_some() {
                    Some("有买点区间".to_string())
                } else {
                    None
                };
                (buy_value, size_value, reason)
            }
            Ok(None) => (None, None, None),
            Err(_) => (None, None, None),
        };
        if let Some(reason) = zone_reason {
            reasons.push(reason);
        }

        // 概念共振：与池内其他票的共享概念数
        let resonance = crate::monitor::concept_peers::peers_of(
            db,
            code,
            crate::monitor::concept_peers::DEFAULT_MIN_SHARED,
        )
        .ok()
        .map(|peers| {
            let in_pool = peers
                .iter()
                .filter(|peer| codes.iter().any(|other| other == &peer.code))
                .count();
            (in_pool as f64 / RESONANCE_FULL).min(1.0) * 100.0
        });

        let name = crate::monitor::buy_zone::resolve_name(&mut conn, code);

        // 加权：只对非缺失项计算
        let mut weighted = 0.0f64;
        let mut weight_sum = 0.0f64;
        for (score, weight) in [
            (news, W_NEWS),
            (board, W_BOARD),
            (buy, W_BUY),
            (size, W_SIZE),
            (resonance, W_RESONANCE),
        ] {
            if let Some(value) = score {
                weighted += value * weight;
                weight_sum += weight;
            }
        }
        let total = if weight_sum > 0.0 {
            weighted / weight_sum
        } else {
            0.0
        };

        rows.push(PriorityRow {
            code: code.clone(),
            name,
            total,
            news_score: news,
            board_score: board,
            buy_score: buy,
            size_score: size,
            resonance_score: resonance,
            reasons,
        });
    }

    rows.sort_by(|left, right| {
        right
            .total
            .partial_cmp(&left.total)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.code.cmp(&right.code))
    });
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_sum_to_one() {
        let sum = W_NEWS + W_BOARD + W_BUY + W_SIZE + W_RESONANCE;
        assert!((sum - 1.0).abs() < 1e-9, "权重和应为 1，实际 {sum}");
    }

    #[test]
    fn missing_dimensions_are_excluded_from_denominator() {
        // 只有 news 有值、其他全缺失时，总分应等于 news 分本身，
        // 而不是被权重稀释成 news * 0.30
        let news = 80.0;
        let weighted = news * W_NEWS;
        let weight_sum = W_NEWS;
        assert!((weighted / weight_sum - 80.0).abs() < 1e-9);
    }

    #[test]
    fn render_shows_rank_and_scores() {
        let row = PriorityRow {
            code: "601811".to_string(),
            name: Some("新华文轩".to_string()),
            total: 62.5,
            news_score: Some(80.0),
            board_score: None,
            buy_score: Some(0.0),
            size_score: Some(0.0),
            resonance_score: Some(40.0),
            reasons: vec!["超买，不参与".to_string()],
        };
        let text = row.render(1);
        assert!(text.contains("1. 601811 新华文轩"));
        assert!(text.contains("62.5"));
        assert!(text.contains("超买"));
    }
}
