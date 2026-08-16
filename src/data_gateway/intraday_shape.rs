//! BR-187 evidence-preserving Magic TDX intraday-shape boundary.
//!
//! The shape is a pure projection of one already validated Magic TDX T0
//! batch. Prices, previous close, and five-minute bars are never joined from
//! different providers or batches.

use chrono::{DateTime, Local, NaiveDate, NaiveTime, SecondsFormat, Utc};
use crate::magic_compat::ProviderId;

use super::instrument_identity::resolve_production_equity;
#[cfg(test)]
use super::instrument_identity::resolve_test_equity;
use super::magic_tdx::MagicTdxGateway;
use super::magic_tdx_t0::{
    MagicTdxT0Batch, MagicTdxT0Evidence, MagicTdxT0FiveMinuteBar, T0_QUOTE_MAX_AGE_SECS,
};
use super::review::{
    acquisition_request_hash, audit_blocking_join_failure, audit_gateway_result, BatchEvidence,
    GatewayBatch, GatewayError,
};

const CAPABILITY: &str = "IntradayShape";
const SOURCE: &str = "magic_tdx_t0";
const MANUAL_CONFIRMATION_LIMIT_PERCENT: f64 = 20.0;

#[derive(Debug, Clone, PartialEq)]
pub struct IntradayShapeFact {
    pub date: String,
    pub pre_close: f64,
    pub open_pct: f64,
    pub high_pct: f64,
    pub low_pct: f64,
    pub close_pct: f64,
    pub amplitude: f64,
    pub tail_30m_pct: Option<f64>,
    pub shape_label: &'static str,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct IntradayShapeGateway;

impl IntradayShapeGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn current_shape(
        &self,
        code: &str,
    ) -> Result<GatewayBatch<IntradayShapeFact>, GatewayError> {
        let requested_code = code.to_owned();
        let request_hash = acquisition_request_hash(CAPABILITY, &requested_code);
        // P4 M4b: gRPC 桥 (DATA_GATEWAY_GRPC=1 时替换 transport; audit 留客户端)。
        // 先走本地 validate (非法 code → invalid_request 语义与 library 对等),
        // 再桥取形 (服务端 fetch_intraday_shape 按 codes 视图输出)。
        match super::grpc_source::bridge_for("IntradayShape") {
            Ok(Some(bridge)) => {
                let storage_code = match validate_requested_code(&requested_code) {
                    Ok(valid) => valid,
                    Err(error) => {
                        return audit_gateway_result(CAPABILITY, ProviderId::Tdx, &request_hash, Err(error))
                    }
                };
                let result = bridge.intraday_shape_async(&storage_code).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tdx);
                return audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result);
            }
            Ok(None) => {}
            Err(error) => {
                return audit_gateway_result(CAPABILITY, ProviderId::Tdx, &request_hash, Err(error));
            }
        }
        let worker_hash = request_hash.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let result = validate_requested_code(&requested_code).and_then(|storage_code| {
                let observed_at = Utc::now();
                MagicTdxGateway::new()
                    .get_t0_evidence_batch(std::slice::from_ref(&storage_code), observed_at)
                    .map_err(|error| {
                        GatewayError::unavailable(
                            CAPABILITY,
                            Some(ProviderId::Tdx),
                            true,
                            format!("Magic TDX T0 acquisition failed: {error:#}"),
                        )
                    })
                    .and_then(|batch| project_t0_batch(&storage_code, batch))
            });
            audit_gateway_result(CAPABILITY, ProviderId::Tdx, &worker_hash, result)
        })
        .await;
        match joined {
            Ok(result) => result,
            Err(error) => {
                audit_blocking_join_failure(
                    CAPABILITY,
                    ProviderId::Tdx,
                    request_hash,
                    error.to_string(),
                )
                .await
            }
        }
    }
}

fn validate_requested_code(code: &str) -> Result<String, GatewayError> {
    #[cfg(test)]
    let identity = if code.starts_with("TEST_CODE_") {
        resolve_test_equity(code, None)
    } else {
        resolve_production_equity(code, None)
    };
    #[cfg(not(test))]
    let identity = resolve_production_equity(code, None);
    let identity = identity.map_err(|error| {
        GatewayError::invalid_request(
            CAPABILITY,
            format!("invalid A-share code {code:?}: {error}"),
        )
    })?;
    identity.require_a_share().map_err(|error| {
        GatewayError::invalid_request(
            CAPABILITY,
            format!("invalid A-share code {code:?}: {error}"),
        )
    })?;
    Ok(identity.storage_code().to_owned())
}

fn project_t0_batch(
    required_code: &str,
    batch: MagicTdxT0Batch,
) -> Result<GatewayBatch<IntradayShapeFact>, GatewayError> {
    if batch.observed_at < batch.requested_at || batch.source_at > batch.observed_at {
        return invalid_evidence(format!(
            "batch evidence timestamps conflict code={required_code} requested_at={} source_at={} observed_at={}",
            batch.requested_at, batch.source_at, batch.observed_at
        ));
    }
    if let Some(rejection) = batch.rejections.first() {
        return Err(GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Tdx),
            "partial",
            rejection.reason_code,
            rejection.retryable,
            format!(
                "Magic TDX T0 rejected {}: {}",
                rejection.code, rejection.detail
            ),
        ));
    }
    if batch.records.len() != 1 {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Tdx),
            format!(
                "BR-187 requires exactly one admitted target record, actual={}",
                batch.records.len()
            ),
        ));
    }
    validate_capture_freshness(required_code, batch.source_at, batch.observed_at)?;

    let evidence = BatchEvidence {
        provider: ProviderId::Tdx,
        source: SOURCE.to_owned(),
        source_at: Some(rfc3339(batch.source_at)),
        observed_at: rfc3339(batch.observed_at),
        batch_id: batch.batch_id.clone(),
    };
    let record = batch
        .records
        .into_iter()
        .next()
        .expect("cardinality checked above");
    let fact = project_record(
        required_code,
        &batch.batch_id,
        batch.requested_at,
        batch.source_at,
        batch.observed_at,
        record,
    )?;
    Ok(GatewayBatch::Available {
        records: vec![fact],
        evidence,
    })
}

fn project_record(
    required_code: &str,
    batch_id: &str,
    requested_at: DateTime<Utc>,
    source_at: DateTime<Utc>,
    observed_at: DateTime<Utc>,
    record: MagicTdxT0Evidence,
) -> Result<IntradayShapeFact, GatewayError> {
    if record.code != required_code || record.instrument.code() != required_code {
        return invalid_evidence(format!(
            "target identity mismatch required={required_code} actual={} instrument={}",
            record.code,
            record.instrument.code()
        ));
    }
    if record.batch_id != batch_id
        || record.requested_at != requested_at
        || record.source_at != source_at
        || record.observed_at != observed_at
    {
        return invalid_evidence(format!(
            "record evidence differs from batch code={required_code}"
        ));
    }
    validate_capture_freshness(required_code, record.source_at, record.observed_at)?;

    let quote = record.quote;
    validate_quote_prices(
        required_code,
        quote.last_close,
        quote.open,
        quote.high,
        quote.low,
        quote.price,
    )?;

    let source_date = record.source_at.with_timezone(&Local).date_naive();
    let today_bars =
        select_source_day_bars(required_code, source_date, &record.completed_five_minute)?;
    let pre_close = quote.last_close;
    let open_pct = percentage_change(quote.open, pre_close);
    let high_pct = percentage_change(quote.high, pre_close);
    let low_pct = percentage_change(quote.low, pre_close);
    let close_pct = percentage_change(quote.price, pre_close);
    for (field, value) in [
        ("open_pct", open_pct),
        ("high_pct", high_pct),
        ("low_pct", low_pct),
        ("close_pct", close_pct),
    ] {
        validate_shape_percentage(required_code, field, value)?;
    }

    let tail_30m_pct =
        tail_anchor(&today_bars).map(|bar| percentage_change(quote.price, bar.close));
    if let Some(value) = tail_30m_pct {
        validate_shape_percentage(required_code, "tail_30m_pct", value)?;
    }
    let amplitude = high_pct - low_pct;
    if !amplitude.is_finite() || amplitude < 0.0 {
        return invalid_evidence(format!(
            "invalid intraday amplitude code={required_code} value={amplitude}"
        ));
    }
    let shape_label = classify_shape(
        open_pct,
        high_pct,
        low_pct,
        close_pct,
        amplitude,
        tail_30m_pct,
    );
    Ok(IntradayShapeFact {
        date: source_date.to_string(),
        pre_close,
        open_pct,
        high_pct,
        low_pct,
        close_pct,
        amplitude,
        tail_30m_pct,
        shape_label,
    })
}

fn validate_capture_freshness(
    code: &str,
    source_at: DateTime<Utc>,
    observed_at: DateTime<Utc>,
) -> Result<(), GatewayError> {
    let age_seconds = observed_at.signed_duration_since(source_at).num_seconds();
    if !(0..=T0_QUOTE_MAX_AGE_SECS).contains(&age_seconds) {
        return Err(GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Tdx),
            "partial",
            "source_stale",
            age_seconds > T0_QUOTE_MAX_AGE_SECS,
            format!(
                "BR-187 realtime source rejected code={code} age_seconds={age_seconds} \
                 max_seconds={T0_QUOTE_MAX_AGE_SECS}"
            ),
        ));
    }
    Ok(())
}

fn validate_quote_prices(
    code: &str,
    pre_close: f64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
) -> Result<(), GatewayError> {
    if [pre_close, open, high, low, close]
        .into_iter()
        .any(|value| !value.is_finite() || value <= 0.0)
    {
        return invalid_evidence(format!(
            "invalid quote price code={code} pre_close={pre_close} open={open} high={high} \
             low={low} close={close}"
        ));
    }
    if high < open.max(close) || low > open.min(close) || high < low {
        return invalid_evidence(format!(
            "quote OHLC conflict code={code} open={open} high={high} low={low} close={close}"
        ));
    }
    Ok(())
}

fn select_source_day_bars<'a>(
    code: &str,
    source_date: NaiveDate,
    bars: &'a [MagicTdxT0FiveMinuteBar],
) -> Result<Vec<&'a MagicTdxT0FiveMinuteBar>, GatewayError> {
    let selected = bars
        .iter()
        .filter(|bar| bar.at.date() == source_date)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return invalid_evidence(format!(
            "BR-187 source-day five-minute bars missing code={code} date={source_date}"
        ));
    }
    if selected.windows(2).any(|pair| pair[0].at >= pair[1].at) {
        return invalid_evidence(format!(
            "BR-187 source-day five-minute bars are not strictly ascending code={code} \
             date={source_date}"
        ));
    }
    Ok(selected)
}

fn tail_anchor<'a>(bars: &'a [&'a MagicTdxT0FiveMinuteBar]) -> Option<&'a MagicTdxT0FiveMinuteBar> {
    let threshold = NaiveTime::from_hms_opt(14, 30, 0).expect("valid fixed time");
    bars.iter().copied().find(|bar| bar.at.time() >= threshold)
}

fn percentage_change(value: f64, base: f64) -> f64 {
    (value / base - 1.0) * 100.0
}

fn validate_shape_percentage(code: &str, field: &str, value: f64) -> Result<(), GatewayError> {
    if !value.is_finite() {
        return invalid_evidence(format!(
            "non-finite intraday percentage code={code} field={field} value={value}"
        ));
    }
    if value.abs() > MANUAL_CONFIRMATION_LIMIT_PERCENT {
        return Err(GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Tdx),
            "partial",
            "manual_confirmation_required",
            false,
            format!(
                "BR-187 manual_confirmation_required code={code} field={field} \
                 value={value:.4}% limit=±{MANUAL_CONFIRMATION_LIMIT_PERCENT:.0}%"
            ),
        ));
    }
    Ok(())
}

fn classify_shape(
    open_pct: f64,
    high_pct: f64,
    low_pct: f64,
    close_pct: f64,
    amplitude: f64,
    tail_30m_pct: Option<f64>,
) -> &'static str {
    let retreat_from_high = high_pct - close_pct;
    let rise_from_low = close_pct - low_pct;
    if high_pct >= 2.0 && retreat_from_high >= 2.0 && close_pct < high_pct * 0.5 {
        "冲高回落"
    } else if tail_30m_pct.is_some_and(|tail| tail <= -1.5) {
        "尾盘跳水"
    } else if tail_30m_pct.is_some_and(|tail| tail >= 1.5) && close_pct > open_pct {
        "尾盘拉升"
    } else if open_pct >= 2.0 && close_pct <= open_pct - 1.5 {
        "高开低走"
    } else if open_pct <= -1.5 && close_pct >= open_pct + 2.0 {
        "低开高走"
    } else if close_pct >= high_pct - 0.5 && high_pct > 1.5 {
        "稳步推高"
    } else if close_pct <= low_pct + 0.5 && low_pct < -1.5 {
        "持续下行"
    } else if amplitude >= 4.0 && retreat_from_high >= 2.0 && rise_from_low >= 2.0 {
        "剧烈震荡"
    } else {
        "窄幅整理"
    }
}

fn invalid_evidence<T>(message: impl Into<String>) -> Result<T, GatewayError> {
    Err(GatewayError::invalid_evidence(
        CAPABILITY,
        Some(ProviderId::Tdx),
        message,
    ))
}

fn rfc3339(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_gateway::magic_tdx_t0::{MagicTdxT0Quote, MagicTdxT0Rejection, T0BookLevel};
    use chrono::TimeZone;

    fn book() -> [T0BookLevel; 5] {
        std::array::from_fn(|index| T0BookLevel {
            price: 9.9 + index as f64 * 0.01,
            volume: 1_000.0,
        })
    }

    fn fixture() -> MagicTdxT0Batch {
        let source_at = Utc.with_ymd_and_hms(2026, 7, 29, 7, 0, 0).unwrap();
        let observed_at = source_at + chrono::Duration::seconds(2);
        let date = source_at.with_timezone(&Local).date_naive();
        let bars = [(9, 35, 10.0), (9, 40, 10.1), (14, 30, 10.2), (14, 35, 10.3)]
            .into_iter()
            .map(|(hour, minute, close)| MagicTdxT0FiveMinuteBar {
                at: date
                    .and_hms_opt(hour, minute, 0)
                    .expect("valid fixture time"),
                open: close,
                high: close,
                low: close,
                close,
                volume: 1_000.0,
                amount: close * 1_000.0,
            })
            .collect();
        let record = MagicTdxT0Evidence {
            instrument: crate::magic_compat::InstrumentId::new(
                crate::magic_compat::Exchange::Shanghai,
                "TEST_CODE_600396",
                crate::magic_compat::AssetClass::Equity,
            )
            .unwrap(),
            code: "TEST_CODE_600396".to_owned(),
            requested_at: source_at - chrono::Duration::seconds(1),
            source_at,
            observed_at,
            batch_id: "TEST_CODE_INTRADAY_BATCH".to_owned(),
            quote: MagicTdxT0Quote {
                price: 10.4,
                last_close: 10.0,
                open: 10.1,
                high: 10.5,
                low: 9.9,
                volume: 100_000.0,
                amount: 1_020_000.0,
                bids: book(),
                asks: book(),
            },
            settled_daily: Vec::new(),
            completed_five_minute: bars,
            intraday_average_price: 10.2,
        };
        MagicTdxT0Batch {
            requested_at: source_at - chrono::Duration::seconds(1),
            source_at,
            observed_at,
            batch_id: "TEST_CODE_INTRADAY_BATCH".to_owned(),
            records: vec![record],
            rejections: Vec::new(),
        }
    }

    #[test]
    fn br187_projects_one_complete_same_batch_shape() {
        let projected = project_t0_batch("TEST_CODE_600396", fixture()).unwrap();
        let GatewayBatch::Available { records, evidence } = projected else {
            panic!("complete input cannot become verified empty");
        };
        assert_eq!(evidence.provider, ProviderId::Tdx);
        assert_eq!(evidence.batch_id, "TEST_CODE_INTRADAY_BATCH");
        assert_eq!(records.len(), 1);
        let shape = &records[0];
        assert_eq!(shape.date, "2026-07-29");
        assert_eq!(shape.pre_close, 10.0);
        assert!((shape.close_pct - 4.0).abs() < 1e-9);
        assert!((shape.tail_30m_pct.unwrap() - (10.4 / 10.2 - 1.0) * 100.0).abs() < 1e-9);
        assert_eq!(shape.shape_label, "尾盘拉升");
    }

    #[test]
    fn br187_rejects_partial_or_cross_batch_evidence() {
        let mut partial = fixture();
        partial.rejections.push(MagicTdxT0Rejection {
            code: "TEST_CODE_600396".to_owned(),
            reason_code: "five_minute_gap",
            detail: "TEST_CODE fixture gap".to_owned(),
            retryable: false,
        });
        assert_eq!(
            project_t0_batch("TEST_CODE_600396", partial)
                .unwrap_err()
                .reason_code(),
            "five_minute_gap"
        );

        let mut mismatched = fixture();
        mismatched.records[0].batch_id = "TEST_CODE_OTHER_BATCH".to_owned();
        assert_eq!(
            project_t0_batch("TEST_CODE_600396", mismatched)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );

        let mut request_time_mismatched = fixture();
        request_time_mismatched.records[0].requested_at =
            request_time_mismatched.requested_at + chrono::Duration::seconds(1);
        assert_eq!(
            project_t0_batch("TEST_CODE_600396", request_time_mismatched)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );

        let mut instrument_mismatched = fixture();
        instrument_mismatched.records[0].instrument = crate::magic_compat::InstrumentId::new(
            crate::magic_compat::Exchange::Shanghai,
            "TEST_CODE_600397",
            crate::magic_compat::AssetClass::Equity,
        )
        .unwrap();
        assert_eq!(
            project_t0_batch("TEST_CODE_600396", instrument_mismatched)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );
    }

    #[test]
    fn br187_preserves_missing_exact_confirmation_contract_as_a_blocker() {
        let mut blocked = fixture();
        blocked.records.clear();
        blocked.rejections.push(MagicTdxT0Rejection {
            code: "TEST_CODE_600396".to_owned(),
            reason_code: "manual_confirmation_contract_unavailable",
            detail: "TEST_CODE missing settled-daily provenance and lifecycle evidence".to_owned(),
            retryable: false,
        });

        let error = project_t0_batch("TEST_CODE_600396", blocked).unwrap_err();
        assert_eq!(
            error.reason_code(),
            "manual_confirmation_contract_unavailable"
        );
        assert!(!error.retryable());
        assert!(error.to_string().contains("settled-daily provenance"));
    }

    #[test]
    fn br187_rejects_stale_capture_without_widening_five_second_gate() {
        let mut stale = fixture();
        stale.observed_at = stale.source_at + chrono::Duration::seconds(6);
        stale.records[0].observed_at = stale.observed_at;
        assert_eq!(
            project_t0_batch("TEST_CODE_600396", stale)
                .unwrap_err()
                .reason_code(),
            "source_stale"
        );
    }

    #[test]
    fn br187_requires_manual_confirmation_for_over_twenty_percent_shape() {
        let mut extreme = fixture();
        extreme.records[0].quote.price = 13.0;
        extreme.records[0].quote.high = 13.0;
        assert_eq!(
            project_t0_batch("TEST_CODE_600396", extreme)
                .unwrap_err()
                .reason_code(),
            "manual_confirmation_required"
        );
    }

    #[test]
    fn br187_keeps_tail_absent_before_tail_window() {
        let mut morning = fixture();
        morning.records[0]
            .completed_five_minute
            .retain(|bar| bar.at.time() < NaiveTime::from_hms_opt(14, 30, 0).unwrap());
        let projected = project_t0_batch("TEST_CODE_600396", morning).unwrap();
        assert_eq!(projected.records()[0].tail_30m_pct, None);
    }

    #[test]
    fn br187_rejects_missing_or_unsorted_source_day_bars() {
        let mut missing = fixture();
        missing.records[0].completed_five_minute.clear();
        assert_eq!(
            project_t0_batch("TEST_CODE_600396", missing)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );

        let mut unsorted = fixture();
        unsorted.records[0].completed_five_minute.swap(0, 1);
        assert_eq!(
            project_t0_batch("TEST_CODE_600396", unsorted)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );
    }
}
