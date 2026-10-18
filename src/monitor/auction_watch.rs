//! 观察池竞价观测（2026-10-18）。
//!
//! ## 为什么加这个
//!
//! 已有的 op 61 竞价链路只服务 P-02 竞价量能榜（全市场榜单），
//! 不覆盖观察池单票。用户要求在 09:25 就能看到 601811 这类票的开盘方向，
//! 比等 09:30 的实时扫描早 5 分钟。
//!
//! ## 为什么用 stage=Final
//!
//! 09:25 之后竞价撮合已完成，`Final` 是终态且实测稳定可得
//! （probe 2026-10-16: admission=ADMITTED records=1）。
//! `live` 只在窗口内有效，09:25 之后可能已切走。
//!
//! ## 边界
//!
//! - 只读观察池 active ticket + 当前持仓，不写库、不改 stage
//! - 竞价价缺失时显式显示 "—"，不填 0（BR-222）
//! - 停牌票拿不到竞价数据，自动不出现在结果里

use std::collections::HashSet;

use diesel::prelude::*;
use diesel::sql_types::Text;

use crate::data_gateway::current_auction_observations::{
    AuctionStage, CurrentAuctionObservationsGateway, CurrentAuctionRequest,
};
use crate::database::DatabaseManager;
use crate::market_domain::{AssetClass, Exchange, InstrumentId};

#[derive(Debug, Clone)]
pub struct AuctionRow {
    pub code: String,
    pub name: String,
    pub auction_price: Option<f64>,
    pub auction_pct: Option<f64>,
    pub auction_volume_ratio: Option<f64>,
}

impl AuctionRow {
    pub fn render(&self) -> String {
        let price = self
            .auction_price
            .map(|value| format!("{value:.2}"))
            .unwrap_or_else(|| "—".to_string());
        let pct = self
            .auction_pct
            .map(|value| format!("{value:+.2}%"))
            .unwrap_or_else(|| "—".to_string());
        let ratio = self
            .auction_volume_ratio
            .map(|value| format!("量比 {value:.2}"))
            .unwrap_or_else(|| "量比 —".to_string());
        format!("{} {}  竞价 {price}  {pct}  {ratio}", self.code, self.name)
    }
}

fn exchange_for(code: &str) -> Exchange {
    match code.chars().next() {
        Some('6') => Exchange::Shanghai,
        Some('0') | Some('3') => Exchange::Shenzhen,
        Some('8') | Some('4') => Exchange::Beijing,
        _ => Exchange::Shanghai,
    }
}

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

/// 拉观察池 + 持仓的竞价终态。
pub async fn observe(db: &DatabaseManager) -> Result<Vec<AuctionRow>, String> {
    let codes = {
        let mut conn = db.get_conn().map_err(|e| e.to_string())?;
        watch_codes(&mut conn)?
    };
    if codes.is_empty() {
        return Ok(Vec::new());
    }

    let mut instruments = Vec::with_capacity(codes.len());
    for code in &codes {
        match InstrumentId::new(exchange_for(code), code, AssetClass::Equity) {
            Ok(instrument) => instruments.push(instrument),
            Err(error) => log::warn!("[竞价观察] {code} 构造标的失败: {error}"),
        }
    }
    if instruments.is_empty() {
        return Ok(Vec::new());
    }

    let request = CurrentAuctionRequest::new(instruments, AuctionStage::Final)
        .map_err(|error| format!("op 61 请求构造失败: {error}"))?;
    let batch = CurrentAuctionObservationsGateway::new()
        .fetch(&request)
        .await
        .map_err(|error| format!("op 61 竞价观测获取失败: {error}"))?;

    Ok(batch
        .records()
        .iter()
        .map(|observation| AuctionRow {
            code: observation.instrument.code().to_string(),
            name: observation.name.clone().unwrap_or_default(),
            auction_price: observation.auction_price,
            auction_pct: observation.auction_pct,
            auction_volume_ratio: observation.auction_volume_ratio,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exchange_mapping_covers_main_boards() {
        assert_eq!(exchange_for("600396"), Exchange::Shanghai);
        assert_eq!(exchange_for("601811"), Exchange::Shanghai);
        assert_eq!(exchange_for("000338"), Exchange::Shenzhen);
        assert_eq!(exchange_for("300750"), Exchange::Shenzhen);
        assert_eq!(exchange_for("830799"), Exchange::Beijing);
        assert_eq!(exchange_for("430047"), Exchange::Beijing);
    }

    #[test]
    fn render_uses_dash_for_missing_values() {
        let row = AuctionRow {
            code: "601811".to_string(),
            name: "新华文轩".to_string(),
            auction_price: None,
            auction_pct: None,
            auction_volume_ratio: None,
        };
        let text = row.render();
        assert!(text.contains("601811"));
        assert!(text.contains("—"));
        assert!(!text.contains("0.00"));
    }
}
