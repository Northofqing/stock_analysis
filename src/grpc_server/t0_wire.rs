use crate::data_gateway::{
    MagicTdxT0Batch, MagicTdxT0DailyBar, MagicTdxT0Evidence, MagicTdxT0FiveMinuteBar,
    MagicTdxT0Quote, MagicTdxT0Rejection,
};
use crate::market_domain::InstrumentId;
use chrono::{FixedOffset, SecondsFormat, TimeZone};
use serde::Serialize;

const CHINA_OFFSET_SECONDS: i32 = 8 * 60 * 60;

#[derive(Serialize)]
struct T0EvidenceBatchWireV2<'a> {
    requested_at: String,
    source_at: String,
    observed_at: String,
    batch_id: &'a str,
    time_untrustworthy: bool,
    records: Vec<T0EvidenceRecordWireV2<'a>>,
    rejections: &'a [MagicTdxT0Rejection],
}

#[derive(Serialize)]
struct T0EvidenceRecordWireV2<'a> {
    instrument: &'a InstrumentId,
    code: &'a str,
    requested_at: String,
    source_at: String,
    observed_at: String,
    batch_id: &'a str,
    quote: &'a MagicTdxT0Quote,
    settled_daily: &'a [MagicTdxT0DailyBar],
    completed_five_minute: Vec<T0FiveMinuteBarWireV2>,
    intraday_average_price: f64,
}

#[derive(Serialize)]
struct T0FiveMinuteBarWireV2 {
    at: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    amount: f64,
}

fn china_session_at(bar: &MagicTdxT0FiveMinuteBar) -> String {
    let offset = FixedOffset::east_opt(CHINA_OFFSET_SECONDS).expect("static +08:00 offset");
    offset
        .from_local_datetime(&bar.at)
        .single()
        .expect("fixed offset has one local mapping")
        .to_rfc3339_opts(SecondsFormat::Secs, false)
}

pub(super) fn encode_t0_batch_v2(batch: &MagicTdxT0Batch) -> Result<Vec<u8>, serde_json::Error> {
    let records = batch
        .records
        .iter()
        .map(T0EvidenceRecordWireV2::from)
        .collect();
    serde_json::to_vec(&T0EvidenceBatchWireV2 {
        requested_at: batch.requested_at.to_rfc3339(),
        source_at: batch.source_at.to_rfc3339(),
        observed_at: batch.observed_at.to_rfc3339(),
        batch_id: &batch.batch_id,
        time_untrustworthy: batch.time_untrustworthy,
        records,
        rejections: &batch.rejections,
    })
}

impl<'a> From<&'a MagicTdxT0Evidence> for T0EvidenceRecordWireV2<'a> {
    fn from(record: &'a MagicTdxT0Evidence) -> Self {
        Self {
            instrument: &record.instrument,
            code: &record.code,
            requested_at: record.requested_at.to_rfc3339(),
            source_at: record.source_at.to_rfc3339(),
            observed_at: record.observed_at.to_rfc3339(),
            batch_id: &record.batch_id,
            quote: &record.quote,
            settled_daily: &record.settled_daily,
            completed_five_minute: record
                .completed_five_minute
                .iter()
                .map(|bar| T0FiveMinuteBarWireV2 {
                    at: china_session_at(bar),
                    open: bar.open,
                    high: bar.high,
                    low: bar.low,
                    close: bar.close,
                    volume: bar.volume,
                    amount: bar.amount,
                })
                .collect(),
            intraday_average_price: record.intraday_average_price,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_gateway::{
        MagicTdxT0Batch, MagicTdxT0Evidence, MagicTdxT0FiveMinuteBar, MagicTdxT0Quote, T0BookLevel,
    };
    use crate::market_domain::{AssetClass, Exchange, InstrumentId, ProviderId};
    use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};

    #[test]
    fn encodes_batch_identity_and_china_session_offset() {
        let batch = sample_batch_with_bar(
            NaiveDate::from_ymd_opt(2026, 8, 27)
                .unwrap()
                .and_hms_opt(13, 5, 0)
                .unwrap(),
        );
        let value: serde_json::Value =
            serde_json::from_slice(&encode_t0_batch_v2(&batch).unwrap()).unwrap();

        assert_eq!(value["requested_at"], batch.requested_at.to_rfc3339());
        assert_eq!(value["source_at"], batch.source_at.to_rfc3339());
        assert_eq!(value["observed_at"], batch.observed_at.to_rfc3339());
        assert_eq!(value["batch_id"], "TEST_CODE_T0_BATCH_001");
        assert_eq!(value["time_untrustworthy"], false);
        assert_eq!(
            value["records"][0]["completed_five_minute"][0]["at"],
            "2026-08-27T13:05:00+08:00"
        );
    }

    fn sample_batch_with_bar(at: NaiveDateTime) -> MagicTdxT0Batch {
        let requested_at = Utc.with_ymd_and_hms(2026, 8, 27, 5, 4, 59).unwrap();
        let source_at = Utc.with_ymd_and_hms(2026, 8, 27, 5, 5, 0).unwrap();
        let observed_at = source_at + chrono::Duration::milliseconds(250);
        let book = || {
            std::array::from_fn(|index| T0BookLevel {
                price: 9.95 + index as f64 * 0.01,
                volume: 100.0,
            })
        };
        let record = MagicTdxT0Evidence {
            instrument: InstrumentId::new(
                Exchange::Shanghai,
                "TEST_CODE_T0_001",
                AssetClass::Equity,
            )
            .unwrap(),
            code: "TEST_CODE_T0_001".to_owned(),
            requested_at,
            source_at,
            observed_at,
            batch_id: "TEST_CODE_T0_BATCH_001".to_owned(),
            quote: MagicTdxT0Quote {
                price: 10.0,
                last_close: 9.9,
                open: 9.95,
                high: 10.1,
                low: 9.8,
                volume: 1_000.0,
                amount: 10_000.0,
                bids: book(),
                asks: book(),
            },
            settled_daily: Vec::new(),
            completed_five_minute: vec![MagicTdxT0FiveMinuteBar {
                at,
                open: 10.0,
                high: 10.1,
                low: 9.9,
                close: 10.0,
                volume: 1_000.0,
                amount: 10_000.0,
            }],
            intraday_average_price: 10.0,
        };
        MagicTdxT0Batch {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_magic_tdx_t0".to_owned(),
            requested_at,
            source_at,
            observed_at,
            batch_id: "TEST_CODE_T0_BATCH_001".to_owned(),
            records: vec![record],
            rejections: Vec::new(),
            time_untrustworthy: false,
        }
    }
}
