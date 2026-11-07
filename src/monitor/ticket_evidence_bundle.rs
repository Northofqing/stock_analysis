//! 6.3 决策时证据包（2026-11-07）。
//!
//! 与 `ticket_evidence.rs`（事后只读审计包）区分：这里只写「当次决策看到了什么」，
//! 不重算、不补缺失、不推断成交概率。
//!
//! 约定：
//! - 文件 `data/ticket_evidence/<business_date>/<code>-<occurrence>.json`；
//! - `ticket_event(kind='evidence_bundle')` 的 summary 只放
//!   `path|sha256|level=L1|missing=...`，便于按 ticket 检索；
//! - 写入失败只记录错误，不阻断推送——证据包是旁路，不是交易闸门。

use std::fs;
use std::path::{Path, PathBuf};

use diesel::SqliteConnection;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::database::ticket_lifecycle;

pub const SCHEMA_VERSION: &str = "ticket-evidence-v1";

#[derive(Debug, Clone, Serialize)]
pub struct Selection {
    pub source: String,
    pub concept: Option<String>,
    pub limit_pool_batch_id: Option<String>,
    pub streak_raw: Option<u32>,
    pub recommend_price_raw: Option<f64>,
    pub news: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pricing {
    pub auction_batch_id: Option<String>,
    pub auction_source_at: Option<String>,
    pub auction_price: Option<f64>,
    pub auction_pct: Option<f64>,
    pub tier: Option<String>,
    pub quoted_price: Option<f64>,
    pub streak: Option<u32>,
    pub streak_source_date: Option<String>,
    pub phase: Option<String>,
    pub phase_source_date: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Portfolio {
    pub position_quantity: Option<u64>,
    pub cost_price: Option<f64>,
    pub market_value: Option<f64>,
    pub weight_pct: Option<f64>,
    /// `user_confirmed_snapshot` / `read_failed`。
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Fundamentals {
    pub status: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvidenceBundle {
    pub schema_version: String,
    pub business_date: String,
    pub code: String,
    pub name: String,
    pub ticket_id: i64,
    pub captured_at: String,
    pub level: String,
    pub selection: Selection,
    pub pricing: Pricing,
    pub portfolio: Portfolio,
    pub fundamentals: Fundamentals,
    pub missing: Vec<String>,
}

impl EvidenceBundle {
    /// 09:25 竞价路径的最小包：定价事实来自 `AuctionPriceReport`，
    /// 选入事实只有来源与 streak，不做补充推断。
    #[allow(clippy::too_many_arguments)]
    pub fn for_auction(
        business_date: &str,
        captured_at: &str,
        fact: &crate::monitor::ticket_auction_price::EvidenceFact,
    ) -> Self {
        let portfolio = match &fact.position_error {
            Some(_) => Portfolio {
                position_quantity: None,
                cost_price: None,
                market_value: None,
                weight_pct: None,
                source: "read_failed".to_owned(),
            },
            None => Portfolio {
                position_quantity: fact.position_quantity,
                cost_price: fact.cost_price,
                market_value: None,
                weight_pct: None,
                source: "user_confirmed_snapshot".to_owned(),
            },
        };
        Self {
            schema_version: SCHEMA_VERSION.to_owned(),
            business_date: business_date.to_owned(),
            code: fact.code.clone(),
            name: fact.name.clone(),
            ticket_id: fact.ticket_id,
            captured_at: captured_at.to_owned(),
            level: "L1".to_owned(),
            selection: Selection {
                source: "auction_0925".to_owned(),
                concept: None,
                limit_pool_batch_id: None,
                streak_raw: fact.streak,
                recommend_price_raw: None,
                news: Vec::new(),
            },
            pricing: Pricing {
                auction_batch_id: None,
                auction_source_at: None,
                auction_price: Some(fact.auction_price),
                auction_pct: Some(fact.auction_pct),
                tier: Some(fact.tier.clone()),
                quoted_price: fact.quoted_price,
                streak: fact.streak,
                streak_source_date: fact.streak_source_date.clone(),
                phase: fact.phase.clone(),
                phase_source_date: fact.phase_source_date.clone(),
            },
            portfolio,
            fundamentals: Fundamentals {
                status: "unavailable".to_owned(),
                reason: Some("financial_statements_not_fetched".to_owned()),
            },
            missing: vec![
                "official_filing".to_owned(),
                "independent_financial_source".to_owned(),
                "raw_auction_depth".to_owned(),
                "financial_statements".to_owned(),
            ],
        }
    }
}

/// 写文件 + 追加 `ticket_event(kind='evidence_bundle')`，返回 (路径, sha256)。
pub fn write_bundle(
    root: &Path,
    conn: &mut SqliteConnection,
    bundle: &EvidenceBundle,
    occurrence: u32,
) -> Result<(PathBuf, String), String> {
    let dir = root
        .join("data")
        .join("ticket_evidence")
        .join(&bundle.business_date);
    fs::create_dir_all(&dir).map_err(|error| format!("创建 {} 失败: {error}", dir.display()))?;
    let path = dir.join(format!("{}-{occurrence:02}.json", bundle.code));
    let payload = serde_json::to_vec_pretty(bundle).map_err(|error| error.to_string())?;
    fs::write(&path, &payload).map_err(|error| format!("写入 {} 失败: {error}", path.display()))?;

    let mut hasher = Sha256::new();
    hasher.update(&payload);
    let sha256 = format!("{:x}", hasher.finalize());
    let missing = if bundle.missing.is_empty() {
        "none".to_owned()
    } else {
        bundle.missing.join(",")
    };
    let summary = format!(
        "{}|{}|level={}|missing={}",
        path.display(),
        sha256,
        bundle.level,
        missing
    );
    ticket_lifecycle::append_event(
        conn,
        bundle.ticket_id,
        &bundle.captured_at,
        "evidence_bundle",
        None,
        Some(&format!(
            "evidence:{}:{}:{occurrence:02}",
            bundle.business_date, bundle.code
        )),
        Some(&summary),
    )?;
    Ok((path, sha256))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_none_is_explicit() {
        let bundle = EvidenceBundle {
            schema_version: SCHEMA_VERSION.to_owned(),
            business_date: "2026-11-07".to_owned(),
            code: "001216".to_owned(),
            name: "华瓷股份".to_owned(),
            ticket_id: 1,
            captured_at: "2026-11-07T09:25:00+08:00".to_owned(),
            level: "L1".to_owned(),
            selection: Selection {
                source: "auction_0925".to_owned(),
                concept: None,
                limit_pool_batch_id: None,
                streak_raw: None,
                recommend_price_raw: None,
                news: Vec::new(),
            },
            pricing: Pricing {
                auction_batch_id: None,
                auction_source_at: None,
                auction_price: Some(26.6),
                auction_pct: Some(-5.41),
                tier: Some("bargain".to_owned()),
                quoted_price: Some(26.6),
                streak: None,
                streak_source_date: None,
                phase: None,
                phase_source_date: None,
            },
            portfolio: Portfolio {
                position_quantity: Some(500),
                cost_price: Some(26.61),
                market_value: None,
                weight_pct: None,
                source: "user_confirmed_snapshot".to_owned(),
            },
            fundamentals: Fundamentals {
                status: "unavailable".to_owned(),
                reason: Some("financial_statements_not_fetched".to_owned()),
            },
            missing: Vec::new(),
        };
        let json = serde_json::to_string(&bundle).unwrap();
        // 未知就是 null，不写成 0 或空串（BR-222）。
        assert!(json.contains("\"streak\":null"));
        assert!(json.contains("\"phase\":null"));
        assert!(json.contains("\"position_quantity\":500"));
    }
}
