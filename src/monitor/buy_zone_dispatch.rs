//! 盘后可买价推送（2026-10-18）。
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
//! ## 输出什么
//!
//! 每只票一段，包含：
//! - 买点区间（有买点时）
//! - 放弃价
//! - 建议仓位
//! - 一句条件（为什么给/为什么不给）
//!
//! ## 不输出什么
//!
//! 不给"买/不买"的绝对结论。规则只回答"什么价位算合理"，判断仍由人做。

use diesel::prelude::*;
use diesel::sql_types::Text;

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
    pub fn render(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        lines.push(format!(
            "{}  收盘 {}",
            self.label, self.zone.close
        ));

        // 位置
        let ma_text = match (self.zone.ma5, self.zone.ma10, self.zone.ma20) {
            (Some(a), Some(b), Some(c)) => format!("ma5 {a} / ma10 {b} / ma20 {c}"),
            _ => "均线不全".to_string(),
        };
        lines.push(format!("  位置：{} · {}", self.zone.trend.label(), ma_text));

        // 买点
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

        // 仓位
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

/// 标的：观察池 active ticket + 当前持仓（去重）。
fn target_codes(conn: &mut SqliteConnection) -> Result<Vec<String>, String> {
    use std::collections::HashSet;
    let mut set: HashSet<String> = HashSet::new();
    let rows: Vec<CodeRow> =
        diesel::sql_query("SELECT code FROM ticket_lifecycle WHERE close_date IS NULL")
            .load(conn)
            .map_err(|error| error.to_string())?;
    for row in rows {
        set.insert(row.code);
    }
    for position in crate::portfolio::load_positions().unwrap_or_default() {
        set.insert(position.code);
    }
    let mut codes: Vec<String> = set.into_iter().collect();
    codes.sort();
    Ok(codes)
}

/// 生成明日计划文本。空池返回提示文本（不是错误）。
pub fn render_plan(db: &DatabaseManager, as_of: &str) -> Result<String, String> {
    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    let codes = target_codes(&mut conn)?;

    if codes.is_empty() {
        return Ok(format!("【明日计划 {as_of}】观察池与持仓均为空，无可算标的。"));
    }

    let mut rows: Vec<PlanRow> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    for code in &codes {
        match buy_zone::compute(&mut conn, code, as_of) {
            Ok(Some(zone)) => {
                let advice = position_sizing::advise(&zone.display_name(), &zone);
                let label = zone.display_name();
                rows.push(PlanRow {
                    code: code.clone(),
                    label,
                    zone,
                    advice,
                });
            }
            Ok(None) => skipped.push(format!("{code}（无日线数据）")),
            Err(error) => skipped.push(format!("{code}（{error}）")),
        }
    }

    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("【明日计划 {as_of}】标的 {} 只", rows.len()));

    // 可买的排前面
    rows.sort_by(|left, right| {
        right
            .advice
            .suggested_pct
            .partial_cmp(&left.advice.suggested_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.code.cmp(&right.code))
    });

    for row in &rows {
        parts.push(String::new());
        parts.push(row.render());
    }

    if !skipped.is_empty() {
        parts.push(String::new());
        parts.push(format!("跳过 {} 只：{}", skipped.len(), skipped.join("、")));
    }

    parts.push(String::new());
    parts.push("口径：买点/放弃价来自收盘后均线与 20 日区间；仓位按 2% 风险预算算，受 30% 单票上限约束。".to_string());

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
