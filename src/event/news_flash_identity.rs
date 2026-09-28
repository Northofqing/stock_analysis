//! Legacy NewsFlash render and reservation digest material.
//!
//! These hashes identify gate decisions. They do not attest a Foundation
//! reservation binding.

use chrono::NaiveDate;
use sha2::{Digest, Sha256};

pub struct NewsFlashReservationIdentityFields<'a> {
    pub push_kind: &'a str,
    pub business_date: NaiveDate,
    pub decision_key: &'a str,
    pub event_id: Option<&'a str>,
    pub window: Option<&'a str>,
    pub evidence_sha256: &'a str,
    pub render_sha256: &'a str,
}

pub fn news_flash_render_sha256(rendered: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"stock_analysis.news_flash_render.v1");
    hasher.update([0]);
    hasher.update(rendered);
    format!("{:x}", hasher.finalize())
}

pub fn news_flash_reservation_sha256(fields: &NewsFlashReservationIdentityFields<'_>) -> String {
    let business_date = fields.business_date.to_string();
    let mut hasher = Sha256::new();
    hasher.update(b"stock_analysis.news_flash_reservation.v2");
    for value in [
        fields.push_kind,
        business_date.as_str(),
        fields.decision_key,
        fields.event_id.unwrap_or("<absent>"),
        fields.window.unwrap_or("<absent>"),
        fields.evidence_sha256,
        fields.render_sha256,
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n02_legacy_identity_fixed_critical_and_absent_vector() {
        let render_sha256 = news_flash_render_sha256(b"TEST_CODE rendered");
        assert_eq!(
            render_sha256,
            "73286f3b54c0f3c256c1e5841aef32fca9a07e2afee8a6a3a5fdb9b7747533db"
        );
        let day = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let fields = NewsFlashReservationIdentityFields {
            push_kind: "news_flash_critical_v1",
            business_date: day,
            decision_key: "event:TEST_CODE_EVENT_X",
            event_id: Some("TEST_CODE_EVENT_X"),
            window: None,
            evidence_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            render_sha256: &render_sha256,
        };
        assert_eq!(
            news_flash_reservation_sha256(&fields),
            "b3cb8402164dfd3805638e0ec7caa00d10409eded3b623c58b7b5ed079916da1"
        );
        assert_eq!(
            news_flash_reservation_sha256(&NewsFlashReservationIdentityFields {
                event_id: Some(""),
                ..fields
            }),
            "ff406579de35a25ba98b605c65c0e9212d00640705c57dc462d449f68bfcfa20"
        );
    }
}
