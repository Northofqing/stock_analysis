//! BR-146: complete user-confirmed snapshots are atomic and latest-wins.
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserPositionItemInput {
    pub code: String,
    pub name: String,
    pub quantity: u64,
    pub cost_price: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserPositionSnapshotInput {
    pub snapshot_id: String,
    pub effective_at: DateTime<FixedOffset>,
    pub confirmed_at: DateTime<FixedOffset>,
    pub source: String,
    pub confirm_empty: bool,
    pub evidence_sha256: String,
    pub items: Vec<UserPositionItemInput>,
}

#[derive(Deserialize)]
struct Raw {
    schema_version: u32,
    effective_at: DateTime<FixedOffset>,
    confirm_empty: bool,
    items: Vec<UserPositionItemInput>,
}
#[derive(Serialize)]
struct Canon<'a> {
    schema_version: u32,
    effective_at: String,
    confirm_empty: bool,
    items: &'a [UserPositionItemInput],
}

pub fn user_position_snapshot_input_from_json(
    json: &str,
    confirmed_at: DateTime<FixedOffset>,
) -> Result<UserPositionSnapshotInput, String> {
    let mut raw: Raw =
        serde_json::from_str(json).map_err(|e| format!("invalid snapshot JSON: {e}"))?;
    if raw.schema_version != 1 {
        return Err("schema_version must be 1".into());
    }
    if raw.items.is_empty() != raw.confirm_empty {
        return Err("confirm_empty must match item emptiness".into());
    }
    for item in &raw.items {
        if item.code.trim().is_empty() || item.name.trim().is_empty() {
            return Err("code/name cannot be blank".into());
        }
        if item.quantity == 0 || !item.cost_price.is_finite() || item.cost_price <= 0.0 {
            return Err("quantity/cost_price invalid".into());
        }
    }
    raw.items.sort_by(|a, b| a.code.cmp(&b.code));
    if raw.items.windows(2).any(|w| w[0].code == w[1].code) {
        return Err("duplicate code".into());
    }
    let canonical = serde_json::to_vec(&Canon {
        schema_version: 1,
        effective_at: raw.effective_at.to_rfc3339(),
        confirm_empty: raw.confirm_empty,
        items: &raw.items,
    })
    .map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    h.update(b"stock_analysis.user_position_snapshot.v1\0");
    h.update(&canonical);
    let evidence_sha256 = format!("{:x}", h.finalize());
    Ok(UserPositionSnapshotInput {
        snapshot_id: format!("ups_v1_{evidence_sha256}"),
        effective_at: raw.effective_at,
        confirmed_at,
        source: "user_confirmed_full_snapshot".into(),
        confirm_empty: raw.confirm_empty,
        evidence_sha256,
        items: raw.items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_snapshot_is_canonical_and_stable() {
        let t = DateTime::parse_from_rfc3339("2026-07-22T16:00:00+08:00").unwrap();
        let j = r#"{"schema_version":1,"effective_at":"2026-07-22T15:00:00+08:00","confirm_empty":false,"items":[{"code":"TEST_CODE_600000","name":"乙","quantity":150,"cost_price":20.0},{"code":"TEST_CODE_000001","name":"甲","quantity":100,"cost_price":10.0}]}"#;
        let a = user_position_snapshot_input_from_json(j, t).unwrap();
        let b = user_position_snapshot_input_from_json(j, t).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.items[0].code, "TEST_CODE_000001");
        assert!(a.evidence_sha256.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
