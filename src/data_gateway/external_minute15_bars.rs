//! R12 endpoint research from the existing Tdx/Minute15 tail contract.
//! Observations are retained before interpretation; receipts grant no PIT,
//! outcome, lifecycle or interval-start authority.
use crate::grpc_client::client::external_historical_read::{
    ExternalHistoricalObservation, ExternalHistoricalReadClient, WindowTransportEvidence,
};
use crate::grpc_client::envelope::{AcquisitionProvenance, QueryAdmission, QueryResult};
use crate::grpc_client::errors::GrpcError;
use crate::grpc_client::external_v1::build_external_minute15_tail_request;
use crate::market_domain::{AssetClass, Exchange, InstrumentId, SecurityBar};
use chrono::{Datelike, NaiveDateTime, Timelike};
use prost::Message as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

pub(crate) const CAPTURE_DOMAIN: &[u8] = b"gateway-observed-external-minute15-tail-v1";
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Minute15Receipt {
    pub(crate) code: String,
    pub(crate) provider: String,
    pub(crate) batch_id: String,
    pub(crate) source_at: String,
    pub(crate) observed_at: String,
    pub(crate) request_sha256: String,
    pub(crate) response_sha256: String,
    pub(crate) capture_sha256: String,
    pub(crate) artifact_sha256: String,
    pub(crate) descriptor_sha256: String,
    pub(crate) server_source_revision: String,
    pub(crate) first_boundary: String,
    pub(crate) last_boundary: String,
    pub(crate) date_counts: BTreeMap<String, usize>,
    pub(crate) requested_limit: usize,
    pub(crate) bars_sha256: String,
}

impl Minute15Receipt {
    pub fn validate_binding(&self) -> std::result::Result<(), String> {
        let valid_hash = |s: &str| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        if self.provider != "Tdx"
            || self.code.len() != 6
            || !self.code.bytes().all(|b| b.is_ascii_digit())
            || self.batch_id.is_empty()
            || self.observed_at.is_empty()
            || self.date_counts.is_empty()
            || self.first_boundary.is_empty()
            || self.last_boundary.is_empty()
            || ![
                &self.request_sha256,
                &self.response_sha256,
                &self.capture_sha256,
                &self.artifact_sha256,
                &self.descriptor_sha256,
                &self.bars_sha256,
            ]
            .into_iter()
            .all(|h| valid_hash(h))
            || self.server_source_revision.len() != 40
            || !self
                .server_source_revision
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err("R12 Minute15 retained source binding invalid".into());
        }
        Ok(())
    }
    pub(crate) fn verify_bars(&self, bars: &[SecurityBar]) -> std::result::Result<(), String> {
        self.validate_binding()?;
        let bytes = serde_json::to_vec(bars).map_err(|_| "R12 Minute15 bars encoding failed")?;
        if sha(&bytes) != self.bars_sha256 {
            return Err("R12 Minute15 bars/source binding conflict".into());
        }
        Ok(())
    }
}
#[derive(Debug)]
pub struct ReceivedMinute15Bars {
    bars: Vec<SecurityBar>,
    receipt: Minute15Receipt,
}
impl ReceivedMinute15Bars {
    pub fn bars(&self) -> &[SecurityBar] {
        &self.bars
    }
    pub fn receipt(&self) -> &Minute15Receipt {
        &self.receipt
    }
}
/// Preserve upstream typed reasons and retry flags without widening the
/// frozen &'static GatewayError reason catalog.
#[derive(Debug, Clone)]
pub struct Minute15Error {
    pub(crate) source_error: Option<GrpcError>,
    reason: &'static str,
    retryable: bool,
}
impl Minute15Error {
    fn local(reason: &'static str, retryable: bool) -> Self {
        Self {
            source_error: None,
            reason,
            retryable,
        }
    }
    fn source(error: GrpcError) -> Self {
        let retryable = error.details().retryable.unwrap_or(matches!(
            error,
            GrpcError::Unavailable { .. }
                | GrpcError::DeadlineExceeded { .. }
                | GrpcError::ResourceExhausted { .. }
        ));
        Self {
            source_error: Some(error),
            reason: "minute15_source_failure",
            retryable,
        }
    }
    pub fn retryable(&self) -> bool {
        self.retryable
    }
    pub fn reason_code(&self) -> &str {
        self.source_error
            .as_ref()
            .and_then(|e| e.details().reason_code.as_deref())
            .unwrap_or(self.reason)
    }
}
impl fmt::Display for Minute15Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "R12 Minute15 reason_code={} retryable={}",
            self.reason_code(),
            self.retryable
        )
    }
}
impl std::error::Error for Minute15Error {}
type Result<T> = std::result::Result<T, Minute15Error>;
fn require(ok: bool, reason: &'static str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Minute15Error::local(reason, false))
    }
}
fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) struct Minute15ObservationCapture {
    parts: [Vec<u8>; 16],
    hash: String,
}
impl Minute15ObservationCapture {
    pub(crate) fn parts(&self) -> &[Vec<u8>; 16] {
        &self.parts
    }
    pub(crate) fn hash(&self) -> &str {
        &self.hash
    }
}
fn capture(
    code: &str,
    count: usize,
    issued: &[u8],
    retained: &WindowTransportEvidence,
    observed: Option<&ExternalHistoricalObservation>,
    failure: Option<&GrpcError>,
) -> Result<Minute15ObservationCapture> {
    let encode = |value: &serde_json::Value| {
        serde_json::to_vec(value)
            .map_err(|_| Minute15Error::local("minute15_capture_encoding", false))
    };
    let mut parts: [Vec<u8>; 16] = std::array::from_fn(|_| Vec::new());
    parts[0] = CAPTURE_DOMAIN.to_vec();
    parts[1] = encode(
        &serde_json::json!({"code":code,"interval":"Minute15","provider":"Tdx","start":null,"end":null,"limit":count,"coverage":"ActualReturnedTailOnly"}),
    )?;
    parts[2] = serde_json::to_vec(&retained.connection_identity)
        .map_err(|_| Minute15Error::local("minute15_capture_encoding", false))?;
    parts[3] = retained
        .health_hex
        .as_ref()
        .map(|s| hex::decode(s))
        .transpose()
        .map_err(|_| Minute15Error::local("minute15_capture_encoding", false))?
        .unwrap_or_default();
    parts[6] = retained
        .capabilities_hex
        .as_ref()
        .map(|s| hex::decode(s))
        .transpose()
        .map_err(|_| Minute15Error::local("minute15_capture_encoding", false))?
        .unwrap_or_default();
    parts[10] = issued.to_vec();
    parts[12] = serde_json::to_vec(&retained.wire)
        .map_err(|_| Minute15Error::local("minute15_capture_encoding", false))?;
    parts[13] = serde_json::to_vec(retained)
        .map_err(|_| Minute15Error::local("minute15_capture_encoding", false))?;
    parts[14] = encode(
        &serde_json::json!({"stage":retained.stage,"outcome":"PreCallOrReadFailure","reason_code":failure.and_then(|e| e.details().reason_code.as_deref()),"retryable":failure.and_then(|e| e.details().retryable)}),
    )?;
    if let Some(o) = observed {
        parts[2] = serde_json::to_vec(&o.connection_identity)
            .map_err(|_| Minute15Error::local("minute15_capture_encoding", false))?;
        parts[3] = o.health_wire.clone();
        parts[4] = o.health.encode_to_vec();
        parts[5] = o.server_build_identity.encode_to_vec();
        parts[6] = o.capabilities_wire.clone();
        parts[7] = o.capabilities_response.encode_to_vec();
        parts[8] = o.capability.encode_to_vec();
        parts[9] = o.request_id_correlation.as_bytes().to_vec();
        parts[11] = o.request_bytes.clone();
        parts[12] = serde_json::to_vec(&o.wire)
            .map_err(|_| Minute15Error::local("minute15_capture_encoding", false))?;
        if let Some(status) = &o.status {
            parts[13] = encode(
                &serde_json::json!({"code":status.code,"message":status.raw_status.message(),"details":status.details,"trailer":status.error_detail_trailer,"encoded_trailer":status.raw_status.metadata().get_bin("magic-error-detail-bin").map(|v| v.as_encoded_bytes())}),
            )?;
        }
        parts[14] = match &o.result {
            Ok(q) => encode(
                &serde_json::json!({"outcome":"EnvelopeObserved","provider":q.selected_provider,"batch_id":q.batch_id,"source_at":q.source_at,"observed_at":q.observed_at,"complete":q.complete,"diagnostic_blocker":q.diagnostic_blocker}),
            )?,
            Err(e) => encode(
                &serde_json::json!({"outcome":"Rejected","reason_code":e.details().reason_code,"retryable":e.details().retryable,"code":e.details().code}),
            )?,
        };
    }
    parts[15] = encode(
        &serde_json::json!({"request_binding_matched":observed.is_some_and(|o| o.request_bytes == issued),"scope":"ObservedOnly"}),
    )?;
    let hash = super::external_historical_bars::hash_capture_parts_v1(&parts);
    Ok(Minute15ObservationCapture { parts, hash })
}

pub(crate) fn receive_tail(code: &str, count: usize) -> Result<ReceivedMinute15Bars> {
    require(
        code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()) && (1..=800).contains(&count),
        "minute15_request_invalid",
    )?;
    let exchange = if code.starts_with('6') {
        Exchange::Shanghai
    } else if code.starts_with('0') || code.starts_with('3') {
        Exchange::Shenzhen
    } else {
        return Err(Minute15Error::local(
            "minute15_instrument_unsupported",
            false,
        ));
    };
    let instrument = InstrumentId::new(exchange, code, AssetClass::Equity)
        .map_err(|_| Minute15Error::local("minute15_request_invalid", false))?;
    let bundle = std::env::var_os("GRPC_MARKET_CLIENT_BUNDLE")
        .map(PathBuf::from)
        .ok_or_else(|| Minute15Error::local("minute15_bundle_unconfigured", false))?;
    let output = std::env::var_os("R12_MINUTE15_EVIDENCE_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| Minute15Error::local("minute15_evidence_unconfigured", false))?;
    // Reuse the bounded owning runtime; retain the source error in the inner
    // result rather than projecting it into the legacy gateway reason table.
    super::grpc_source::block_on_gateway(async {
        Ok(receive_at(&bundle, &output, &instrument, count).await)
    })
    .map_err(|e| Minute15Error::local("minute15_bounded_runtime_failed", e.retryable()))?
}
async fn receive_at(
    bundle: &Path,
    output: &Path,
    instrument: &InstrumentId,
    count: usize,
) -> Result<ReceivedMinute15Bars> {
    require(bundle.is_absolute(), "minute15_bundle_invalid")?;
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (output, instrument, count);
        return Err(Minute15Error::local("minute15_store_unsupported", false));
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let bundle_path = std::fs::canonicalize(bundle)
            .map_err(|_| Minute15Error::local("minute15_bundle_invalid", false))?;
        let forbidden = if bundle_path.is_dir() {
            bundle_path
        } else {
            bundle_path
                .parent()
                .ok_or_else(|| Minute15Error::local("minute15_bundle_invalid", false))?
                .to_path_buf()
        };
        let store = super::historical_observed_store::HistoricalObservedStore::open_existing(
            output,
            &[forbidden],
        )
        .map_err(|_| Minute15Error::local("minute15_evidence_directory_invalid", false))?;
        let query = build_external_minute15_tail_request(instrument, count as u32)
            .map_err(|_| Minute15Error::local("minute15_request_invalid", false))?;
        let issued = query.wire_bytes();
        let mut retained = WindowTransportEvidence::default();
        let mut reader = match ExternalHistoricalReadClient::connect_client_bundle(bundle).await {
            Ok(reader) => reader,
            Err(error) => {
                retained.stage = "ConnectionBeforeHistoricalBarsRpc".into();
                let raw = capture(
                    instrument.code(),
                    count,
                    &issued,
                    &retained,
                    None,
                    Some(&error),
                )?;
                store.persist_minute15(&raw).map_err(|_| {
                    Minute15Error::local("minute15_evidence_persistence_failed", true)
                })?;
                return Err(Minute15Error::source(error));
            }
        };
        let result = reader.query_minute15(query, &mut retained).await;
        let raw = capture(
            instrument.code(),
            count,
            &issued,
            &retained,
            result.as_ref().ok(),
            result.as_ref().err(),
        )?;
        let (artifact, _) = store
            .persist_minute15(&raw)
            .map_err(|_| Minute15Error::local("minute15_evidence_persistence_failed", true))?;
        store
            .read_checked(&artifact)
            .map_err(|_| Minute15Error::local("minute15_evidence_readback_failed", true))?;
        let observation = result.map_err(Minute15Error::source)?;
        require(
            observation.request_bytes == issued,
            "minute15_request_binding_conflict",
        )?;
        let q = observation
            .result
            .as_ref()
            .map_err(|e| Minute15Error::source(e.clone()))?;
        let bars = project_records(instrument, count, q)?;
        let response_sha256 = sha(observation
            .wire
            .payload()
            .ok_or_else(|| Minute15Error::local("minute15_wire_missing", false))?);
        let mut date_counts = BTreeMap::new();
        for bar in &bars {
            *date_counts
                .entry(bar.datetime[..10].to_owned())
                .or_insert(0) += 1;
        }
        let receipt = Minute15Receipt {
            code: instrument.code().to_owned(),
            provider: q.selected_provider.clone(),
            batch_id: q.batch_id.clone(),
            source_at: q.source_at.clone(),
            observed_at: q.observed_at.clone(),
            request_sha256: sha(&issued),
            response_sha256,
            capture_sha256: raw.hash,
            artifact_sha256: artifact.file_sha256().to_owned(),
            descriptor_sha256: observation.connection_identity.descriptor_sha256,
            server_source_revision: observation.server_build_identity.source_revision,
            first_boundary: bars.first().unwrap().datetime.clone(),
            last_boundary: bars.last().unwrap().datetime.clone(),
            date_counts,
            requested_limit: count,
            bars_sha256: sha(&serde_json::to_vec(&bars)
                .map_err(|_| Minute15Error::local("minute15_capture_encoding", false))?),
        };
        receipt
            .verify_bars(&bars)
            .map_err(|_| Minute15Error::local("minute15_receipt_binding_invalid", false))?;
        Ok(ReceivedMinute15Bars { bars, receipt })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMinute15Bar {
    instrument: InstrumentId,
    interval: String,
    bar_start: String,
    bar_end: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    amount: f64,
    adjustment: String,
    source_at: Option<String>,
    observed_at: String,
    provider: String,
    batch_id: String,
}
fn project_records(
    instrument: &InstrumentId,
    limit: usize,
    q: &QueryResult,
) -> Result<Vec<SecurityBar>> {
    project_records_at(instrument, limit, q, chrono::Utc::now())
}

fn project_records_at(
    instrument: &InstrumentId,
    limit: usize,
    q: &QueryResult,
    client_now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<SecurityBar>> {
    require(
        q.admission == QueryAdmission::Admitted && q.complete && q.diagnostic_blocker.is_empty(),
        "minute15_envelope_rejected",
    )?;
    require(
        matches!(&q.provenance, AcquisitionProvenance::ExternalMtlsAuthority(s) if !s.is_empty()),
        "minute15_authority_missing",
    )?;
    require(
        q.selected_provider == "Tdx" && !q.batch_id.is_empty() && !q.observed_at.is_empty(),
        "minute15_batch_evidence_invalid",
    )?;
    require(
        !q.records.is_empty() && q.records.len() <= limit,
        "minute15_batch_count_invalid",
    )?;
    let observed = q
        .observed_at
        .parse::<i64>()
        .ok()
        .and_then(|v| chrono::DateTime::from_timestamp(v, 0))
        .filter(|v| v.timestamp() > 0)
        .ok_or_else(|| Minute15Error::local("minute15_observed_at_invalid", false))?;
    let observed_local = observed
        .naive_utc()
        .checked_add_signed(chrono::Duration::hours(8))
        .ok_or_else(|| Minute15Error::local("minute15_observed_at_invalid", false))?;
    require(
        observed <= client_now + chrono::Duration::seconds(2),
        "minute15_observed_at_future",
    )?;
    let mut bars = Vec::with_capacity(q.records.len());
    let mut last_source = String::new();
    for record in &q.records {
        require(
            record.schema == "magic.market.bar"
                && record.schema_version == 1
                && record.content_type == "application/json; charset=utf-8",
            "minute15_record_contract_invalid",
        )?;
        let row: RawMinute15Bar = serde_json::from_slice(&record.data)
            .map_err(|_| Minute15Error::local("minute15_record_invalid", false))?;
        require(
            row.instrument == *instrument
                && row.interval == "Minute15"
                && row.adjustment == "Unadjusted",
            "minute15_record_identity_conflict",
        )?;
        require(
            row.provider == q.selected_provider
                && row.batch_id == q.batch_id
                && row.observed_at == q.observed_at,
            "minute15_record_evidence_conflict",
        )?;
        // Delivered Tdx normalization repeats its endpoint; never infer start.
        require(
            row.bar_start == row.bar_end,
            "minute15_boundary_semantics_conflict",
        )?;
        let time = NaiveDateTime::parse_from_str(&row.bar_end, "%Y-%m-%d %H:%M:%S")
            .map_err(|_| Minute15Error::local("minute15_boundary_invalid", false))?;
        require(
            row.bar_end.len() == 19
                && row.bar_end == time.format("%Y-%m-%d %H:%M:%S").to_string()
                && time.second() == 0
                && time <= observed_local,
            "minute15_boundary_invalid",
        )?;
        if let Some(source) = row.source_at.as_deref().filter(|s| !s.is_empty()) {
            require(
                source == &row.bar_end[..16],
                "minute15_source_boundary_conflict",
            )?;
        }
        last_source = row.source_at.unwrap_or_default();
        bars.push(SecurityBar {
            open: row.open,
            close: row.close,
            high: row.high,
            low: row.low,
            vol: row.volume,
            amount: row.amount,
            year: time.year() as u32,
            month: time.month(),
            day: time.day(),
            hour: time.hour(),
            minute: time.minute(),
            datetime: row.bar_end,
        });
    }
    require(q.source_at == last_source, "minute15_outer_source_conflict")?;
    crate::review::backtest::validate_technical_bars(&bars)
        .map_err(|_| Minute15Error::local("minute15_grid_or_ohlcv_invalid", false))?;
    Ok(bars)
}

#[cfg(test)]
#[path = "external_minute15_bars_tests.rs"]
mod tests;
