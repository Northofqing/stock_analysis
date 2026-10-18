//! 观察池定向监控（2026-10-18）。
//!
//! ## 为什么现在才做
//!
//! 技术条件一直齐备：`realtime_quotes_async` 存在、观察池 `ticket_lifecycle` 存在、
//! 主循环本来就是 30 秒一轮（`main.rs` 的 `sleep(Duration::from_secs(30))`）。
//! 之前把它列为"没做"只是因为没排优先级，不是做不了。
//!
//! ## 三条异动规则（任一命中）
//!
//! 1. **日内涨跌幅** 绝对值 >= 3%
//! 2. **相对上次观测价** 变动 >= 1%（捕捉盘中急拉急跌）
//! 3. **换手异常** —— 暂不实现，需要成交量字段，当前行情批次没带
//!
//! ## 边界
//!
//! - **只扫描**，不做决策、不改 `stage`、不写库
//! - 去重由调用方负责（同一只票反复触发同一条规则时不该反复推送）
//! - 行情批次不完整（`Partial`）时照常返回能拿到的部分，但调用方应记录缺口
//! - 停牌票拿不到行情，自动不出现在结果里，这是正确行为不是 bug

use std::collections::{HashMap, HashSet};

use diesel::prelude::*;
use diesel::sql_types::Text;

use crate::data_gateway::GatewayBatch;
use crate::database::DatabaseManager;

/// 日内涨跌幅阈值（%）
pub const DAY_CHANGE_THRESHOLD: f64 = 3.0;
/// 相对上次观测价的变动阈值（小数，0.01 = 1%）
pub const TICK_CHANGE_THRESHOLD: f64 = 0.01;

#[derive(Debug, Clone)]
pub struct WatchAlert {
    pub code: String,
    pub name: String,
    pub price: f64,
    pub change_percent: f64,
    /// 触发了哪几条规则，用"；"分隔
    pub reason: String,
}

impl WatchAlert {
    pub fn render(&self) -> String {
        format!(
            "{} {} 现价 {:.2}（{:+.2}%）— {}",
            self.code, self.name, self.price, self.change_percent, self.reason
        )
    }
}

/// 监控标的：观察池全部 active ticket + 当前持仓。
fn watch_codes(conn: &mut SqliteConnection) -> Result<Vec<String>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        code: String,
    }
    let mut set: HashSet<String> = HashSet::new();
    let rows: Vec<Row> =
        diesel::sql_query("SELECT code FROM ticket_lifecycle WHERE close_date IS NULL")
            .load(conn)
            .map_err(|e| e.to_string())?;
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

/// 扫描一轮。返回本轮触发的异动。
///
/// `last_prices` 是上一轮的观测价（调用方维护），用于判断盘中急变。
/// 首轮传空 map 即可 —— 此时只有"日内涨跌幅"规则能触发。
pub async fn scan(
    db: &DatabaseManager,
    last_prices: &HashMap<String, f64>,
) -> Result<Vec<WatchAlert>, String> {
    let codes = {
        let mut conn = db.get_conn().map_err(|e| e.to_string())?;
        watch_codes(&mut conn)?
    };
    if codes.is_empty() {
        return Ok(Vec::new());
    }

    let bridge = crate::data_gateway::grpc_source::bridge_for("RealtimeQuotes")
        .map_err(|error| error.to_string())?;
    let quotes = match bridge.realtime_quotes_async(&codes).await {
        Ok(GatewayBatch::Available { records, .. }) => records,
        Ok(GatewayBatch::VerifiedEmpty(_)) => return Ok(Vec::new()),
        Err(error) => return Err(format!("行情拉取失败: {error}")),
    };

    let mut alerts: Vec<WatchAlert> = Vec::new();
    for quote in quotes {
        let mut reasons: Vec<String> = Vec::new();

        if quote.change_percent.abs() >= DAY_CHANGE_THRESHOLD {
            reasons.push(format!("日内 {:+.2}%", quote.change_percent));
        }

        if let Some(previous) = last_prices.get(&quote.code) {
            if *previous > 0.0 {
                let tick = (quote.price - *previous) / *previous;
                if tick.abs() >= TICK_CHANGE_THRESHOLD {
                    reasons.push(format!("较上次 {:+.2}%", tick * 100.0));
                }
            }
        }

        if reasons.is_empty() {
            continue;
        }
        alerts.push(WatchAlert {
            code: quote.code,
            name: quote.name,
            price: quote.price,
            change_percent: quote.change_percent,
            reason: reasons.join("；"),
        });
    }

    alerts.sort_by(|a, b| {
        b.change_percent
            .abs()
            .partial_cmp(&a.change_percent.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(alerts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_are_documented_values() {
        assert!((DAY_CHANGE_THRESHOLD - 3.0).abs() < f64::EPSILON);
        assert!((TICK_CHANGE_THRESHOLD - 0.01).abs() < f64::EPSILON);
    }

    #[test]
    fn tick_detection_math() {
        let previous = 10.0_f64;
        let now = 10.15_f64;
        let tick = (now - previous) / previous;
        assert!(tick >= TICK_CHANGE_THRESHOLD);
        // 0.5% 变动不该触发
        let small = (10.05_f64 - previous) / previous;
        assert!(small < TICK_CHANGE_THRESHOLD);
    }

    #[test]
    fn alert_render_includes_code_and_name() {
        let alert = WatchAlert {
            code: "600703".to_string(),
            name: "三安光电".to_string(),
            price: 10.18,
            change_percent: -3.5,
            reason: "日内 -3.50%".to_string(),
        };
        let text = alert.render();
        assert!(text.contains("600703"));
        assert!(text.contains("三安光电"));
        assert!(text.contains("-3.50%"));
    }
}
