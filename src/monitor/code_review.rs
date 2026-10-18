//! 票级复盘归因（2026-10-18）。
//!
//! ## 数据来源
//!
//! `paper_trades` 有 2589 笔历史交易（2026 年 3 月以来），带 `code` / `name` /
//! `direction` / `price` / `quantity` / `fill_price` / `ts`。
//! 三层数据都有，之前误判"样本只有 601811 一只"是只看了 `ticket_lifecycle`。
//!
//! ## 口径
//!
//! **已实现盈亏 = Σ(卖出成交额) − Σ(买入成交额)**，按票聚合。
//!
//! 这是近似算法，不是逐笔配对（FIFO/LIFO）。它的局限在**持仓未清零**时会放大：
//! 买入成本全算进去、卖出收入只算已卖部分，看起来像亏。
//! 所以每行同时输出 `open_quantity`（未平数量），读者自己判断数字是否完整。
//!
//! 不用逐笔配对的原因：`paper_trades` 没有配对键（plan_id 不是配对锚点），
//! 强行配对会引入比净额法更大的错。宁可给一个口径明确、局限写清的近似值。
//!
//! ## 不做的事
//!
//! **不推测"为什么赚/为什么亏"。** 归因是人读数字后做的判断，代码编原因就是造假。
//! 本模块只输出客观事实：成交笔数、金额、未平数量、时间跨度、未成交率。

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};

use crate::database::DatabaseManager;

#[derive(Debug, Clone)]
pub struct CodeReview {
    pub code: String,
    pub name: String,
    pub buy_count: i64,
    pub sell_count: i64,
    pub bought_value: f64,
    pub sold_value: f64,
    /// 未平数量（买入手数 − 卖出手数）。非 0 说明盈亏数字不完整。
    pub open_quantity: i64,
    /// 未成交笔数（fill_price 为空）
    pub not_filled: i64,
    pub first_at: String,
    pub last_at: String,
}

impl CodeReview {
    pub fn realized_pnl(&self) -> f64 {
        self.sold_value - self.bought_value
    }

    pub fn render(&self) -> String {
        let pnl = self.realized_pnl();
        let total = self.buy_count + self.sell_count;
        let fill_rate = if total > 0 {
            (total - self.not_filled) as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        let mut line = format!(
            "{} {}  净额 {:+.2}  买{}笔/卖{}笔  成交率 {:.0}%  {} ~ {}",
            self.code,
            self.name,
            pnl,
            self.buy_count,
            self.sell_count,
            fill_rate,
            self.first_at.chars().take(10).collect::<String>(),
            self.last_at.chars().take(10).collect::<String>()
        );
        if self.open_quantity != 0 {
            line.push_str(&format!(
                "\n      未平 {} 股 —— 盈亏数字不完整，买入成本已计入但未卖出",
                self.open_quantity
            ));
        }
        line
    }
}

pub fn build_code_reviews(db: &DatabaseManager) -> Result<Vec<CodeReview>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Text)]
        name: String,
        #[diesel(sql_type = BigInt)]
        buy_count: i64,
        #[diesel(sql_type = BigInt)]
        sell_count: i64,
        #[diesel(sql_type = Nullable<Double>)]
        bought_value: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        sold_value: Option<f64>,
        #[diesel(sql_type = BigInt)]
        open_quantity: i64,
        #[diesel(sql_type = BigInt)]
        not_filled: i64,
        #[diesel(sql_type = Text)]
        first_at: String,
        #[diesel(sql_type = Text)]
        last_at: String,
    }

    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT code, \
            MAX(name) AS name, \
            SUM(CASE WHEN direction='buy' AND fill_price IS NOT NULL THEN 1 ELSE 0 END) AS buy_count, \
            SUM(CASE WHEN direction='sell' AND fill_price IS NOT NULL THEN 1 ELSE 0 END) AS sell_count, \
            SUM(CASE WHEN direction='buy' AND fill_price IS NOT NULL THEN fill_price*quantity ELSE 0 END) AS bought_value, \
            SUM(CASE WHEN direction='sell' AND fill_price IS NOT NULL THEN fill_price*quantity ELSE 0 END) AS sold_value, \
            SUM(CASE WHEN direction='buy' THEN quantity WHEN direction='sell' THEN -quantity ELSE 0 END) AS open_quantity, \
            SUM(CASE WHEN fill_price IS NULL THEN 1 ELSE 0 END) AS not_filled, \
            MIN(ts) AS first_at, \
            MAX(ts) AS last_at \
         FROM paper_trades \
         GROUP BY code \
         ORDER BY code",
    )
    .load(&mut conn)
    .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|row| CodeReview {
            code: row.code,
            name: row.name,
            buy_count: row.buy_count,
            sell_count: row.sell_count,
            bought_value: row.bought_value.unwrap_or(0.0),
            sold_value: row.sold_value.unwrap_or(0.0),
            open_quantity: row.open_quantity,
            not_filled: row.not_filled,
            first_at: row.first_at,
            last_at: row.last_at,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review(bought: f64, sold: f64, open: i64) -> CodeReview {
        CodeReview {
            code: "TEST".to_string(),
            name: "测试".to_string(),
            buy_count: 2,
            sell_count: 1,
            bought_value: bought,
            sold_value: sold,
            open_quantity: open,
            not_filled: 0,
            first_at: "2026-09-01T10:00:00".to_string(),
            last_at: "2026-09-10T14:00:00".to_string(),
        }
    }

    #[test]
    fn pnl_is_sold_minus_bought() {
        let r = review(1000.0, 1200.0, 0);
        assert!((r.realized_pnl() - 200.0).abs() < 1e-9);
    }

    #[test]
    fn open_position_is_flagged_in_render() {
        let r = review(1000.0, 400.0, 600);
        let text = r.render();
        assert!(text.contains("未平 600 股"));
        assert!(text.contains("盈亏数字不完整"));
    }

    #[test]
    fn closed_position_has_no_warning() {
        let r = review(1000.0, 1200.0, 0);
        assert!(!r.render().contains("未平"));
    }
}
