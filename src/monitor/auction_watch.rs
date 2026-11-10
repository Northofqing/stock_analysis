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
use std::path::PathBuf;

use diesel::prelude::*;
use diesel::sql_types::Text;
use serde_json::json;

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
    /// 原始回执证据：用于 09:25 后独立复算，不参与渲染/筛选。
    pub provider: String,
    pub source_at: Option<String>,
    pub observed_at: String,
    pub batch_id: String,
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
        // 920xxx 是北交所新代码段；900xxx 仍是上海 B 股，不能把 9 全部改北交所。
        Some('9') if code.starts_with("92") => Exchange::Beijing,
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

/// ExternalV1 `CurrentAuctionObservations`（op 61）单请求标的上限。
///
/// 实测：2026-10-27 活跃票 82 + 持仓去重后 **83 只成功**；2026-10-28 起活跃票
/// 101/118/116 只，全部 `invalid_request`。边界落在 83~101 之间，按 100 处理；
/// 这里按 50 分片留余量。`auction_watch` 过去把全部活跃票一次塞进请求，
/// 观察池涨到 101 只后就再也没有拿到过竞价数据。
const MAX_AUCTION_INSTRUMENTS_PER_REQUEST: usize = 50;

/// 写当日原始竞价证据；成功后再由调用方写 `data/auction_price/<date>` marker。
/// 文件包含每只票的原始值 + provider/source_at/observed_at/batch_id，供事后复算。
pub fn write_raw(date: &str, rows: &[AuctionRow]) -> Result<PathBuf, String> {
    let dir = PathBuf::from("data/auction_raw");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("创建 {} 失败: {error}", dir.display()))?;
    let path = dir.join(format!("{date}.json"));
    let records: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            json!({
                "code": row.code,
                "name": row.name,
                "auction_price": row.auction_price,
                "auction_pct": row.auction_pct,
                "auction_volume_ratio": row.auction_volume_ratio,
                "provider": row.provider,
                "source_at": row.source_at,
                "observed_at": row.observed_at,
                "batch_id": row.batch_id,
            })
        })
        .collect();
    let payload = json!({
        "date": date,
        "count": records.len(),
        "records": records,
    });
    let text = serde_json::to_string_pretty(&payload)
        .map_err(|error| format!("竞价原始证据序列化失败: {error}"))?;
    // 先写临时文件再 rename，避免半截 JSON 被当成当日证据。
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|error| format!("写入 {} 失败: {error}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|error| {
        format!(
            "重命名 {} -> {} 失败: {error}",
            tmp.display(),
            path.display()
        )
    })?;
    Ok(path)
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

    let gateway = CurrentAuctionObservationsGateway::new();
    let mut rows = Vec::new();
    for (index, chunk) in instruments
        .chunks(MAX_AUCTION_INSTRUMENTS_PER_REQUEST)
        .enumerate()
    {
        let request = CurrentAuctionRequest::new(chunk.to_vec(), AuctionStage::Final)
            .map_err(|error| format!("op 61 请求构造失败: {error}"))?;
        let batch = gateway
            .fetch(&request)
            .await
            .map_err(|error| format!("op 61 竞价观测获取失败 (chunk {index}): {error}"))?;
        rows.extend(batch.records().iter().map(|observation| AuctionRow {
            code: observation.instrument.code().to_string(),
            name: observation.name.clone().unwrap_or_default(),
            auction_price: observation.auction_price,
            auction_pct: observation.auction_pct,
            auction_volume_ratio: observation.auction_volume_ratio,
            provider: format!("{:?}", observation.evidence.provider()),
            source_at: observation.evidence.source_at().map(str::to_owned),
            observed_at: observation.evidence.observed_at().to_owned(),
            batch_id: observation.evidence.batch_id().to_owned(),
        }));
    }

    Ok(rows)
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
        assert_eq!(exchange_for("920249"), Exchange::Beijing);
        assert_eq!(exchange_for("900901"), Exchange::Shanghai);
    }

    #[test]
    fn render_uses_dash_for_missing_values() {
        let row = AuctionRow {
            code: "601811".to_string(),
            name: "新华文轩".to_string(),
            auction_price: None,
            auction_pct: None,
            auction_volume_ratio: None,
            provider: "Tonghuashun".to_string(),
            source_at: None,
            observed_at: "test".to_string(),
            batch_id: "test".to_string(),
        };
        let text = row.render();
        assert!(text.contains("601811"));
        assert!(text.contains("—"));
        assert!(!text.contains("0.00"));
    }
}
