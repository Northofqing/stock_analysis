//! The retained monitor's quote-only auction/intraday report. This capability
//! cannot be constructed from a TopStock, an account estimate or a time slot.
//! Only the original Gateway quote may authorize its deterministic fact card.
use crate::durable_delivery_runtime::{
    CountedDeliveryBinding, CountedDeliveryOrigin, CountedDeliveryScope,
};
use crate::market_data::{fetch_scanner_position_quotes, ScannerPositionQuotes};
use chrono::{DateTime, FixedOffset, NaiveDate, Timelike, Utc};
use sha2::{Digest, Sha256};
use stock_analysis::data_gateway::market_data::AdmittedRealtimeQuote;
use stock_analysis::monitor::scanner::TieredScanner;

fn shanghai() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).unwrap()
}
fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObservationSlot {
    business_date: NaiveDate,
    label: &'static str,
    slot: String,
}
impl ObservationSlot {
    fn at(now: DateTime<Utc>) -> Option<Self> {
        let local = now.with_timezone(&shanghai());
        if !stock_analysis::calendar::verified_a_share_trading_day(local.date_naive()).ok()? {
            return None;
        }
        let minute = local.hour() * 60 + local.minute();
        let (label, slot) = match minute {
            560..565 => ("集合竞价报价观察", "auction".to_owned()),
            570..690 | 780..900 => (
                "盘内报价观察",
                format!("intraday:{:02}:{:02}", minute / 60, minute % 60 / 15 * 15),
            ),
            _ => return None,
        };
        Some(Self {
            business_date: local.date_naive(),
            label,
            slot,
        })
    }
    pub(super) fn occurrence(&self) -> String {
        format!(
            "retained-quote-observation:{}:{}",
            self.business_date, self.slot
        )
    }
}

/// Opaque, owns the original admitted records. No public raw-row constructor,
/// relaxed profile selector or deserialize path exists.
#[derive(Debug)]
pub(super) struct PreparedObservation {
    slot: ObservationSlot,
    quotes: ScannerPositionQuotes,
    text: String,
    binding: CountedDeliveryBinding,
}
impl PreparedObservation {
    fn prepare(
        slot: ObservationSlot,
        quotes: ScannerPositionQuotes,
        banner: &str,
    ) -> Result<Self, String> {
        if quotes.quotes().is_empty() {
            return Err("retained_observation_no_quotes".into());
        }
        let scanner = TieredScanner::new(Vec::new());
        for quote in quotes.quotes() {
            scanner
                .validate_admitted_quote(quote)
                .map_err(|reason| reason.reason_code().to_owned())?;
        }
        let mut rows = quotes.quotes().iter().collect::<Vec<_>>();
        rows.sort_by(|a, b| a.code().cmp(b.code()));
        let text = render(&slot, banner, quotes.scope_note(), &rows);
        let canonical = serde_json::json!({
            "schema": "retained-quote-observation-v1", "business_date": slot.business_date.to_string(),
            "occurrence": slot.occurrence(), "scope_note": quotes.scope_note(),
            "requested_codes": quotes.requested(), "ordered_original_quotes": rows.iter().map(|q| quote_canonical(q)).collect::<Vec<_>>(),
            "rendered_sha256": hash(text.as_bytes()),
        });
        let bytes = canonical.to_string().into_bytes();
        let observed_at = rows.iter().map(|quote| quote.observed_at()).max().unwrap();
        let batch_ids = rows
            .iter()
            .map(|quote| quote.evidence().batch_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let binding = CountedDeliveryBinding::new(
            slot.business_date,
            slot.occurrence(),
            bytes.clone(),
            CountedDeliveryScope::Global,
            hash(&bytes),
            CountedDeliveryOrigin::Provider {
                observed_at: Some(observed_at),
                as_of: Some(slot.business_date),
                ordered_batch_ids: batch_ids,
            },
            None,
            false,
        )?;
        let prepared = Self {
            slot,
            quotes,
            text,
            binding,
        };
        prepared.validate_for_use()?;
        Ok(prepared)
    }
    pub(super) fn validate_for_use(&self) -> Result<(), String> {
        if ObservationSlot::at(Utc::now()).as_ref() != Some(&self.slot) {
            return Err("retained_observation_window_expired".into());
        }
        if self.quotes.quotes().is_empty() {
            return Err("retained_observation_no_quotes".into());
        }
        // Nothing can wait on MoneyFlows, LLM attribution or private platform
        // jobs before these point-of-use checks. The original 5-second rule stays.
        let scanner = TieredScanner::new(Vec::new());
        for quote in self.quotes.quotes() {
            scanner
                .validate_admitted_quote(quote)
                .map_err(|reason| reason.reason_code().to_owned())?;
            if quote.source_at().with_timezone(&shanghai()).date_naive() != self.slot.business_date
            {
                return Err("retained_observation_quote_date_mismatch".into());
            }
        }
        if hash(self.binding.source_binding_canonical())
            != self.binding.source_evidence_fingerprint()
            || self.binding.schedule_occurrence_identity() != self.slot.occurrence()
            || self.binding.business_date() != self.slot.business_date
            || self.binding.retry_authorized()
            || self.binding.task_binding().is_some()
        {
            return Err("retained_observation_binding_invalid".into());
        }
        let canonical: serde_json::Value =
            serde_json::from_slice(self.binding.source_binding_canonical())
                .map_err(|_| "retained_observation_binding_invalid")?;
        if canonical["rendered_sha256"].as_str() != Some(hash(self.text.as_bytes()).as_str()) {
            return Err("retained_observation_text_mismatch".into());
        }
        Ok(())
    }
    pub(super) fn binding(&self) -> &CountedDeliveryBinding {
        &self.binding
    }
    pub(super) fn into_parts(self) -> (String, CountedDeliveryBinding) {
        (self.text, self.binding)
    }
}
fn quote_canonical(q: &AdmittedRealtimeQuote) -> serde_json::Value {
    serde_json::json!({ "code":q.code(), "name":q.name(), "price":q.price(), "previous_close":q.previous_close(),
        "change_percent":q.change_percent(), "source_at":q.source_at().to_rfc3339(), "observed_at":q.observed_at().to_rfc3339(),
        "provider":format!("{:?}",q.evidence().provider), "source":q.evidence().source,
        "batch_source_at":q.evidence().source_at, "batch_observed_at":q.evidence().observed_at, "batch_id":q.evidence().batch_id })
}
fn render(
    slot: &ObservationSlot,
    banner: &str,
    scope_note: &str,
    rows: &[&AdmittedRealtimeQuote],
) -> String {
    let mut text = report_header(slot, banner, scope_note);
    for quote in rows {
        text.push_str(&format!(
            "{}({}) {:.2} {:+.2}% | 源 {}\n",
            quote.name(),
            quote.code(),
            quote.price(),
            quote.change_percent(),
            quote
                .source_at()
                .with_timezone(&shanghai())
                .format("%H:%M:%S")
        ));
    }
    text.push_str(
        "仅报告已接纳的实时报价；量比、资金流、涨跌停资格未取得，不判断竞价强弱或提供交易指令。",
    );
    text
}
fn report_header(slot: &ObservationSlot, banner: &str, scope_note: &str) -> String {
    format!(
        "{}\n📊 {}（{}，北京时间）\n{}\n",
        banner, slot.label, slot.business_date, scope_note
    )
}

/// Denial-only readback before Magiclaw. The coordinator has already frozen
/// this request; stored JSON cannot construct an admitted quote or bypass L5.
pub(super) fn validate_authoritative_request(
    namespace: &crate::durable_delivery_runtime::RuntimeNamespace,
    request: &stock_analysis::durable_delivery::AuthoritativeDeliveryRequest,
    now: DateTime<Utc>,
) -> Result<(), String> {
    use stock_analysis::durable_delivery::PushKind;
    if request.push_kind != PushKind::IntradayMarket {
        return Ok(());
    }
    let relative = match namespace {
        crate::durable_delivery_runtime::RuntimeNamespace::Production => {
            std::path::PathBuf::from("data/durable_delivery.sqlite3")
        }
        crate::durable_delivery_runtime::RuntimeNamespace::Test { test_code } => {
            std::path::PathBuf::from("data/test")
                .join(test_code)
                .join("durable_delivery.sqlite3")
        }
    };
    let path = stock_analysis::production_root::root_for_mode(matches!(
        namespace,
        crate::durable_delivery_runtime::RuntimeNamespace::Test { .. }
    ))
    .join(relative);
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| format!("retained_observation_store_read_failed:{error}"))?;
    let (canonical, stored_hash): (Vec<u8>, String) = connection.query_row(
        "SELECT envelope_canonical,envelope_sha256 FROM delivery_decisions WHERE decision_identity=?1",
        [&request.decision_identity], |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|error| format!("retained_observation_original_decision_missing:{error}"))?;
    if hash(&canonical) != stored_hash {
        return Err("retained_observation_envelope_hash_mismatch".into());
    }
    validate_stored_canonical(&canonical, request, now)
}

fn validate_stored_canonical(
    canonical: &[u8],
    request: &stock_analysis::durable_delivery::AuthoritativeDeliveryRequest,
    now: DateTime<Utc>,
) -> Result<(), String> {
    const INVALID: &str = "retained_observation_original_binding_invalid";
    let envelope: serde_json::Value = serde_json::from_slice(canonical).map_err(|_| INVALID)?;
    let occurrence = envelope["schedule_occurrence_identity"]
        .as_str()
        .ok_or(INVALID)?;
    if !occurrence.starts_with("retained-quote-observation:") {
        return Ok(());
    }
    let slot = ObservationSlot::at(now).ok_or("retained_observation_window_expired")?;
    let date = slot.business_date.to_string();
    if occurrence != slot.occurrence() || envelope["business_date"] != date {
        return Err("retained_observation_window_expired".into());
    }
    if envelope["decision_identity"] != request.decision_identity
        || envelope["push_kind"] != "IntradayMarket"
        || envelope["sub_kind"] != "NONE"
        || envelope["scope_key"] != "GLOBAL"
        || envelope["task_binding"] != serde_json::Value::Null
        || envelope["retry_authorized"] != false
        || envelope["rendered_content_sha256"] != request.rendered_content_sha256
        || hash(&request.rendered_content) != request.rendered_content_sha256
    {
        return Err(INVALID.into());
    }
    let bytes: Vec<u8> = serde_json::from_value(envelope["source_binding_canonical"].clone())
        .map_err(|_| INVALID)?;
    let fingerprint = hash(&bytes);
    if envelope["source_binding_sha256"] != fingerprint
        || envelope["source_evidence_fingerprint"] != fingerprint
        || envelope["delivery_subject_hash"] != fingerprint
    {
        return Err(INVALID.into());
    }
    let source: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| INVALID)?;
    if source["schema"] != "retained-quote-observation-v1"
        || source["business_date"] != date
        || source["occurrence"] != occurrence
        || source["rendered_sha256"] != request.rendered_content_sha256
        || envelope["provider_as_of"] != date
    {
        return Err(INVALID.into());
    }
    let quotes = source["ordered_original_quotes"]
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or(INVALID)?;
    let mut observed = Vec::new();
    let mut batch_ids = std::collections::BTreeSet::new();
    for quote in quotes {
        let source_at = parse_stored_time(&quote["source_at"])?;
        let observed_at = parse_stored_time(&quote["observed_at"])?;
        if source_at > now
            || observed_at > now
            || source_at > observed_at
            || now.signed_duration_since(source_at) > chrono::Duration::seconds(5)
            || source_at.with_timezone(&shanghai()).date_naive() != slot.business_date
        {
            return Err("retained_observation_original_quote_expired".into());
        }
        // Raw provider evidence may use decimal Unix seconds. Its exact bytes
        // remain hash-bound; the sealed quote's normalized original instants
        // above own freshness, never a reinterpretation of raw batch strings.
        if quote["batch_source_at"]
            .as_str()
            .is_none_or(|at| at.is_empty())
            || quote["batch_observed_at"]
                .as_str()
                .is_none_or(|at| at.is_empty())
        {
            return Err(INVALID.into());
        }
        batch_ids.insert(
            quote["batch_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or(INVALID)?
                .to_owned(),
        );
        observed.push(observed_at);
    }
    let original_ids: Vec<String> =
        serde_json::from_value(envelope["original_batch_ids"].clone()).map_err(|_| INVALID)?;
    if original_ids != batch_ids.into_iter().collect::<Vec<_>>()
        || parse_stored_time(&envelope["provider_observed_at"])? != *observed.iter().max().unwrap()
    {
        return Err(INVALID.into());
    }
    Ok(())
}
fn parse_stored_time(value: &serde_json::Value) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(
        value
            .as_str()
            .ok_or("retained_observation_original_time_missing")?,
    )
    .map(|at| at.with_timezone(&Utc))
    .map_err(|_| "retained_observation_original_time_invalid".into())
}

/// Sole scheduler; no catch-up/replay of a past auction or intraday window.
/// Exact occurrence ownership is read from the original durable store before
/// fetching, including Delivered/Uncertain. A changed quote after restart does
/// not create another card for the same slot.
pub(super) async fn run() {
    let mut completed = None::<String>;
    loop {
        let Some(slot) = ObservationSlot::at(Utc::now()) else {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            continue;
        };
        let occurrence = slot.occurrence();
        if completed.as_ref() != Some(&occurrence) {
            match crate::durable_delivery_runtime::retained_observation_occurrence_owned(
                slot.business_date,
                &occurrence,
            ) {
                Ok(true) => {
                    completed = Some(occurrence);
                }
                Err(error) => {
                    log::error!("[retained-observation] owner inspection failed: {error}")
                }
                Ok(false) => {
                    let banner = crate::current_banner_for("retained quote observation")
                        .map(|banner| banner.render())
                        .unwrap_or_else(|| "[账户状态不可用 | 不提供交易指令]".to_owned());
                    match tokio::task::spawn_blocking(move || {
                        fetch_scanner_position_quotes()
                            .and_then(|quotes| PreparedObservation::prepare(slot, quotes, &banner))
                    })
                    .await
                    {
                        Ok(Ok(prepared)) => {
                            let outcome =
                                crate::notify::push_retained_market_observation(prepared).await;
                            log::info!("[retained-observation] occurrence={occurrence} outcome={outcome:?}");
                            if matches!(
                                outcome,
                                crate::notify::PushOutcome::Pushed
                                    | crate::notify::PushOutcome::Deduped
                            ) {
                                completed = Some(occurrence);
                            }
                        }
                        Ok(Err(error)) => log::warn!(
                            "[retained-observation] unavailable={error} occurrence={occurrence}"
                        ),
                        Err(error) => {
                            log::error!("[retained-observation] acquisition task failed: {error}")
                        }
                    }
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }
    #[test]
    fn retained_observation_auction_only_current_live_window_no_catch_up() {
        assert!(ObservationSlot::at(at("2026-10-08T01:19:59Z")).is_none());
        let slot = ObservationSlot::at(at("2026-10-08T01:20:00Z")).unwrap();
        assert_eq!(
            slot.occurrence(),
            "retained-quote-observation:2026-10-08:auction"
        );
        assert_eq!(ObservationSlot::at(at("2026-10-08T01:24:59Z")), Some(slot));
        assert!(ObservationSlot::at(at("2026-10-08T01:25:00Z")).is_none());
    }
    #[test]
    fn retained_observation_restart_slot_identity_and_half_open_sessions() {
        let a = ObservationSlot::at(at("2026-10-08T05:30:01Z")).unwrap();
        assert_eq!(
            ObservationSlot::at(at("2026-10-08T05:44:59Z")),
            Some(a.clone())
        );
        assert_eq!(
            a.occurrence(),
            "retained-quote-observation:2026-10-08:intraday:13:30"
        );
        assert_ne!(ObservationSlot::at(at("2026-10-08T05:45:00Z")).unwrap(), a);
        for value in [
            "2026-10-08T03:30:00Z",
            "2026-10-08T04:59:59Z",
            "2026-10-08T07:00:00Z",
            "2026-10-08T16:00:00Z",
            "2026-10-11T01:20:00Z",
        ] {
            assert!(ObservationSlot::at(at(value)).is_none(), "{value}");
        }
        assert_eq!(
            ObservationSlot::at(at("2026-10-09T01:20:00Z"))
                .unwrap()
                .business_date
                .to_string(),
            "2026-10-09"
        );
    }
    #[test]
    fn retained_observation_frozen_and_old_scope_are_displayed_without_metrics_or_advice() {
        let slot = ObservationSlot::at(at("2026-10-08T05:30:00Z")).unwrap();
        let text = report_header(
            &slot,
            "[🔴 Frozen | 仓位缺失 | 日盈亏缺失 | 数据Degraded]",
            "观察名单来源: 本地记录 2026-09-28；不代表今日持仓",
        );
        assert!(text.contains("Frozen | 仓位缺失 | 日盈亏缺失 | 数据Degraded"));
        assert!(text.contains("2026-09-28；不代表今日持仓"));
        assert!(!text.contains("买入") && !text.contains("涨停"));
    }
    #[test]
    fn retained_observation_missing_quote_is_unavailable_before_any_binding() {
        let slot = ObservationSlot::at(at("2026-10-08T05:30:00Z")).unwrap();
        assert_eq!(
            PreparedObservation::prepare(
                slot,
                ScannerPositionQuotes::NoPositions,
                "TEST_CODE Frozen"
            )
            .unwrap_err(),
            "retained_observation_no_quotes"
        );
    }
    fn stored_fixture(
        source_at: &str,
        observed_at: &str,
        occurrence: &str,
    ) -> (
        Vec<u8>,
        stock_analysis::durable_delivery::AuthoritativeDeliveryRequest,
    ) {
        use stock_analysis::durable_delivery::{
            AuthoritativeDeliveryRequest, DeliveryEnvelope, DeliverySubKind, PushKind,
        };

        let rendered = b"TEST_CODE Frozen; original source time".to_vec();
        let batch_observed = DateTime::parse_from_rfc3339(observed_at)
            .map(|at| format!("{}.{:09}", at.timestamp(), at.timestamp_subsec_nanos()))
            .unwrap_or_else(|_| "TEST_CODE_INVALID_RAW_TIME".into());
        let source = serde_json::json!({"schema":"retained-quote-observation-v1", "business_date":"2026-10-08", "occurrence":occurrence, "rendered_sha256":hash(&rendered), "ordered_original_quotes":[{"source_at":source_at,"observed_at":observed_at,"batch_source_at":source_at,"batch_observed_at":batch_observed,"batch_id":"TEST_CODE_ORIGINAL_BATCH"}]}).to_string().into_bytes();
        let fingerprint = hash(&source);
        let envelope = DeliveryEnvelope::new(
            "2026-10-08",
            PushKind::IntradayMarket,
            DeliverySubKind::None,
            "GLOBAL",
            occurrence,
            &fingerprint,
            source,
            &fingerprint,
            rendered.clone(),
            false,
            None,
        )
        .unwrap()
        .with_provider_evidence(
            Some(observed_at.into()),
            Some("2026-10-08".into()),
            vec!["TEST_CODE_ORIGINAL_BATCH".into()],
        )
        .unwrap();
        let request = AuthoritativeDeliveryRequest {
            decision_identity: envelope.decision_identity.clone(),
            attempt_identity: "TEST_CODE_ATTEMPT".into(),
            fence_token: 1,
            push_kind: PushKind::IntradayMarket,
            stable_template_id: PushKind::IntradayMarket.stable_template_id().into(),
            rendered_content: rendered,
            rendered_content_sha256: envelope.rendered_content_sha256.clone(),
        };
        (envelope.canonical_bytes().unwrap(), request)
    }
    #[test]
    fn retained_observation_reserved_resume_cannot_send_after_original_window() {
        let (auction, request) = stored_fixture(
            "2026-10-08T01:24:57Z",
            "2026-10-08T01:24:58Z",
            "retained-quote-observation:2026-10-08:auction",
        );
        assert!(validate_stored_canonical(&auction, &request, at("2026-10-08T01:24:59Z")).is_ok());
        for now in [
            "2026-10-08T01:25:00Z",
            "2026-10-08T05:30:00Z",
            "2026-10-09T01:20:00Z",
        ] {
            assert_eq!(
                validate_stored_canonical(&auction, &request, at(now)).unwrap_err(),
                "retained_observation_window_expired"
            );
        }
        let (intraday, request) = stored_fixture(
            "2026-10-08T05:44:57Z",
            "2026-10-08T05:44:58Z",
            "retained-quote-observation:2026-10-08:intraday:13:30",
        );
        assert!(validate_stored_canonical(&intraday, &request, at("2026-10-08T05:44:59Z")).is_ok());
        for now in ["2026-10-08T05:45:00Z", "2026-10-09T05:30:00Z"] {
            assert_eq!(
                validate_stored_canonical(&intraday, &request, at(now)).unwrap_err(),
                "retained_observation_window_expired"
            );
        }
        let (ordinary, request) = stored_fixture(
            "2026-10-08T05:44:57Z",
            "2026-10-08T05:44:58Z",
            "TEST_CODE_OTHER_PRODUCER",
        );
        assert!(validate_stored_canonical(&ordinary, &request, at("2026-10-09T07:00:00Z")).is_ok());
    }
    #[test]
    fn retained_observation_physical_guard_rejects_stored_age_missing_time_and_hash() {
        let now = at("2026-10-08T05:42:00Z");
        let make = |source_at: &str, observed_at: &str| {
            stored_fixture(
                source_at,
                observed_at,
                "retained-quote-observation:2026-10-08:intraday:13:30",
            )
        };
        let (fresh, request) = make("2026-10-08T05:41:55Z", "2026-10-08T05:41:59Z");
        assert!(validate_stored_canonical(&fresh, &request, now).is_ok());
        // Same slot does not make an old original quote fresh after restart.
        let (old, old_request) = make("2026-10-08T05:30:00Z", "2026-10-08T05:30:03Z");
        assert_eq!(
            validate_stored_canonical(&old, &old_request, now).unwrap_err(),
            "retained_observation_original_quote_expired"
        );
        assert_eq!(
            validate_stored_canonical(&fresh, &request, now + chrono::Duration::nanoseconds(1))
                .unwrap_err(),
            "retained_observation_original_quote_expired"
        );
        let (future, future_request) = make("2026-10-08T05:42:01Z", "2026-10-08T05:42:01Z");
        assert_eq!(
            validate_stored_canonical(&future, &future_request, now).unwrap_err(),
            "retained_observation_original_quote_expired"
        );
        let (yesterday, yesterday_request) = make("2026-10-07T05:41:59Z", "2026-10-07T05:41:59Z");
        assert_eq!(
            validate_stored_canonical(&yesterday, &yesterday_request, now).unwrap_err(),
            "retained_observation_original_quote_expired"
        );
        let (bad_time, bad_request) = make("TEST_CODE_INVALID_TIME", "2026-10-08T05:41:59Z");
        assert_eq!(
            validate_stored_canonical(&bad_time, &bad_request, now).unwrap_err(),
            "retained_observation_original_time_invalid"
        );
        let mut changed: serde_json::Value = serde_json::from_slice(&fresh).unwrap();
        changed["source_binding_sha256"] = "TEST_CODE_CHANGED".into();
        assert_eq!(
            validate_stored_canonical(&serde_json::to_vec(&changed).unwrap(), &request, now)
                .unwrap_err(),
            "retained_observation_original_binding_invalid"
        );
        changed = serde_json::from_slice(&fresh).unwrap();
        changed["provider_observed_at"] = serde_json::Value::Null;
        assert_eq!(
            validate_stored_canonical(&serde_json::to_vec(&changed).unwrap(), &request, now)
                .unwrap_err(),
            "retained_observation_original_time_missing"
        );
    }
}

/// Explicit opt-in integration readback. Inputs are copies of the actual
/// retained Schema9 store; Gateway quotes remain real and the sink is purely
/// in-memory. Never part of CI or a production dispatch/replay command.
#[cfg(test)]
mod live_readback {
    use super::*;
    use std::sync::{Arc, Mutex};
    use stock_analysis::durable_delivery::*;
    #[derive(Default)]
    struct Append(Mutex<Vec<String>>);
    impl ImmutableAppendPort for Append {
        fn append_exact(
            &self,
            _kind: &str,
            identity: &str,
            bytes: &[u8],
            sha256: &str,
        ) -> stock_analysis::durable_delivery::Result<String> {
            assert_eq!(hash(bytes), sha256);
            self.0.lock().unwrap().push(identity.to_owned());
            Ok(format!("TEST_CODE_MEMORY_APPEND:{identity}"))
        }
    }
    struct Sink(AuthoritativeSinkResult, std::sync::atomic::AtomicUsize);
    impl AuthoritativeSinkPort for Sink {
        fn sink_identity(&self) -> &str {
            "TEST_CODE_MEMORY_ONLY_NO_FEISHU"
        }
        fn deliver(&self, _request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
            self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.0.clone()
        }
    }
    #[test]
    #[ignore = "explicit real original Schema9 copy and Gateway readback; rejected before Magiclaw"]
    fn retained_observation_actual_copy_physical_guard_fresh_then_same_slot_expired() {
        assert_eq!(
            std::env::var("TEST_CODE_RETAINED_OBSERVATION_REAL_READBACK").unwrap(),
            "1"
        );
        let input: serde_json::Value = serde_json::from_slice(
            &std::fs::read(std::env::var("TEST_CODE_RETAINED_OBSERVATION_INPUT").unwrap()).unwrap(),
        )
        .unwrap();
        dotenvy::from_path("/Users/zhangzhen/.local/share/stock-analysis-runtime/.env").unwrap();
        let namespace = "TEST_CODE_RETAINED_OBSERVATION_REAL_READBACK_physical_guard";
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data/test")
            .join(namespace);
        stock_analysis::database::DatabaseManager::init_retained_monitor(Some(
            base.join("quote-audit-main.sqlite3"),
        ))
        .unwrap();
        let case_path = base.join("durable_delivery.sqlite3");
        let coordinator =
            DurableDeliveryCoordinator::open_existing_monitor_schema9(CoordinatorConfig::test(
                &case_path,
                namespace,
                "TEST_CODE_PHYSICAL_GUARD_OWNER_0123456789abcdef",
            ))
            .unwrap();
        let requested = input["requested_codes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let admitted = stock_analysis::data_gateway::MarketDataGateway::new()
            .required_realtime_quotes(&requested)
            .unwrap();
        let original = admitted
            .quotes()
            .iter()
            .map(quote_canonical)
            .collect::<Vec<_>>();
        let report = PreparedObservation::prepare(
            ObservationSlot::at(Utc::now()).unwrap(),
            ScannerPositionQuotes::from_real_admitted_for_readback(
                requested,
                admitted,
                input["scope_note"].as_str().unwrap().into(),
            )
            .unwrap(),
            input["banner"].as_str().unwrap(),
        )
        .unwrap();
        assert!(matches!(
            crate::v14_adapter::v14_gate_retained_observation(&report),
            crate::v14_adapter::V14Gate::Approved(_)
        ));
        let (text, binding) = report.into_parts();
        let CountedDeliveryOrigin::Provider {
            observed_at: Some(observed),
            as_of: Some(date),
            ordered_batch_ids,
        } = binding.origin().clone()
        else {
            panic!("real provider only")
        };
        let envelope = DeliveryEnvelope::new(
            binding.business_date().to_string(),
            PushKind::IntradayMarket,
            DeliverySubKind::None,
            "GLOBAL",
            binding.schedule_occurrence_identity(),
            binding.source_evidence_fingerprint(),
            binding.source_binding_canonical().to_vec(),
            binding.delivery_subject_hash(),
            text.into_bytes(),
            false,
            None,
        )
        .unwrap()
        .with_provider_evidence(
            Some(observed.to_rfc3339()),
            Some(date.to_string()),
            ordered_batch_ids,
        )
        .unwrap();
        let request = AuthoritativeDeliveryRequest {
            decision_identity: envelope.decision_identity.clone(),
            attempt_identity: "TEST_CODE_GUARD_READBACK_ONLY".into(),
            fence_token: 1,
            push_kind: PushKind::IntradayMarket,
            stable_template_id: PushKind::IntradayMarket.stable_template_id().into(),
            rendered_content: envelope.rendered_content.clone(),
            rendered_content_sha256: envelope.rendered_content_sha256.clone(),
        };
        let fresh_guard_at = Utc::now();
        validate_stored_canonical(
            &envelope.canonical_bytes().unwrap(),
            &request,
            fresh_guard_at,
        )
        .unwrap();
        let append = Append::default();
        coordinator.prepare(&envelope, 1, Utc::now()).unwrap();
        coordinator
            .reconcile_all_pending(&append, Utc::now())
            .unwrap();
        assert_eq!(
            coordinator
                .decision_state(&envelope.decision_identity)
                .unwrap(),
            DecisionState::Reserved
        );
        std::thread::sleep(std::time::Duration::from_secs(6));
        let stale_guard_at = Utc::now();
        assert_eq!(
            ObservationSlot::at(fresh_guard_at),
            ObservationSlot::at(stale_guard_at),
            "run with enough time left in the current slot"
        );
        std::env::set_var("STOCK_ENV_MODE", "test");
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        std::env::set_var("DURABLE_DELIVERY_TEST_CODE", namespace);
        let sink = Arc::new(
            crate::durable_delivery_runtime::MagiclawAuthoritativeSink::bind(
                crate::durable_delivery_runtime::RuntimeNamespace::Test {
                    test_code: namespace.into(),
                },
            )
            .unwrap(),
        );
        coordinator
            .resume_deliverable(&envelope.decision_identity, &[sink], Utc::now())
            .unwrap();
        coordinator
            .reconcile_all_pending(&append, Utc::now())
            .unwrap();
        assert_eq!(
            coordinator
                .decision_state(&envelope.decision_identity)
                .unwrap(),
            DecisionState::RejectedDurable
        );
        let rejected = coordinator
            .rejected_sink_evidence(&envelope.decision_identity)
            .unwrap()
            .unwrap();
        assert_eq!(
            rejected.reason_code,
            "retained_observation_physical_guard_rejected"
        );
        assert!(!rejected.retry_authorized);
        assert_eq!(
            String::from_utf8(rejected.evidence).unwrap(),
            "retained_observation_original_quote_expired"
        );
        std::fs::write(base.join("physical-guard-results.json"),serde_json::to_vec_pretty(&serde_json::json!({"decision_identity":envelope.decision_identity,"state":"RejectedDurable","fresh_guard_at":fresh_guard_at,"stale_guard_at":stale_guard_at,"original_quotes":original,"occurrence":envelope.schedule_occurrence_identity,"same_slot":true,"source_origin":"Provider","feishu_calls":0,"reason":"retained_observation_original_quote_expired","retry_authorized":false})).unwrap()).unwrap();
    }
    #[test]
    #[ignore = "requires an active current trading window, real provider and original Schema9 copies; memory sink only"]
    fn retained_observation_actual_schema9_copy_opaque_provider_and_terminal_owners() {
        assert_eq!(
            std::env::var("TEST_CODE_RETAINED_OBSERVATION_REAL_READBACK").unwrap(),
            "1"
        );
        let input: serde_json::Value = serde_json::from_slice(
            &std::fs::read(std::env::var("TEST_CODE_RETAINED_OBSERVATION_INPUT").unwrap()).unwrap(),
        )
        .unwrap();
        dotenvy::from_path("/Users/zhangzhen/.local/share/stock-analysis-runtime/.env").unwrap();
        let namespace = "TEST_CODE_RETAINED_OBSERVATION_REAL_READBACK";
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data/test")
            .join(namespace);
        stock_analysis::database::DatabaseManager::init_retained_monitor(Some(
            base.join("quote-audit-main.sqlite3"),
        ))
        .unwrap();
        let requested = input["requested_codes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let mut results = Vec::new();
        for (name, expected) in [
            ("accepted", DecisionState::Delivered),
            ("uncertain", DecisionState::UncertainManualReview),
            ("rejected", DecisionState::RejectedDurable),
        ] {
            let case_code = format!("{namespace}_{name}");
            let case_path = base
                .parent()
                .unwrap()
                .join(&case_code)
                .join("durable_delivery.sqlite3");
            let coordinator =
                DurableDeliveryCoordinator::open_existing_monitor_schema9(CoordinatorConfig::test(
                    &case_path,
                    &case_code,
                    "TEST_CODE_READBACK_OWNER_0123456789abcdef",
                ))
                .unwrap();
            let admitted = stock_analysis::data_gateway::MarketDataGateway::new()
                .required_realtime_quotes(&requested)
                .unwrap();
            let original = admitted
                .quotes()
                .iter()
                .map(quote_canonical)
                .collect::<Vec<_>>();
            let stale = admitted.quotes().to_vec();
            let quotes = ScannerPositionQuotes::from_real_admitted_for_readback(
                requested.clone(),
                admitted,
                input["scope_note"].as_str().unwrap().to_owned(),
            )
            .unwrap();
            let report = PreparedObservation::prepare(
                ObservationSlot::at(Utc::now()).unwrap(),
                quotes,
                input["banner"].as_str().unwrap(),
            )
            .unwrap();
            assert!(report.text.contains("Frozen"));
            assert!(matches!(
                crate::v14_adapter::v14_gate_retained_observation(&report),
                crate::v14_adapter::V14Gate::Approved(_)
            ));
            let canonical: serde_json::Value =
                serde_json::from_slice(report.binding.source_binding_canonical()).unwrap();
            for quote in &original {
                assert!(canonical["ordered_original_quotes"]
                    .as_array()
                    .unwrap()
                    .contains(quote));
            }
            let occurrence = report.slot.occurrence();
            let fingerprint = report.binding.source_evidence_fingerprint().to_owned();
            let (text, binding) = report.into_parts();
            let origin = binding.origin().clone();
            let CountedDeliveryOrigin::Provider {
                observed_at: Some(observed_at),
                as_of: Some(as_of),
                ordered_batch_ids,
            } = origin
            else {
                panic!("real provider only")
            };
            let envelope = DeliveryEnvelope::new(
                binding.business_date().to_string(),
                PushKind::IntradayMarket,
                DeliverySubKind::None,
                "GLOBAL",
                binding.schedule_occurrence_identity(),
                binding.source_evidence_fingerprint(),
                binding.source_binding_canonical().to_vec(),
                binding.delivery_subject_hash(),
                text.as_bytes().to_vec(),
                false,
                None,
            )
            .unwrap()
            .with_provider_evidence(
                Some(observed_at.to_rfc3339()),
                Some(as_of.to_string()),
                ordered_batch_ids,
            )
            .unwrap();
            let append = Append::default();
            coordinator.prepare(&envelope, 1, Utc::now()).unwrap();
            coordinator
                .reconcile_all_pending(&append, Utc::now())
                .unwrap();
            let sink_result = match name {
                "accepted" => AuthoritativeSinkResult::Accepted(TypedReceipt {
                    channel: "TEST_CODE_MEMORY".into(),
                    provider: "TEST_CODE_MEMORY".into(),
                    message_id: "TEST_CODE_MEMORY_RECEIPT".into(),
                    platform_message_id: None,
                    accepted_at: Utc::now(),
                    latency_ms: Some(0),
                }),
                "uncertain" => AuthoritativeSinkResult::Uncertain(TypedUncertainty {
                    reason_code: "TEST_CODE_MEMORY_UNCERTAIN".into(),
                    evidence: b"TEST_CODE_NO_FEISHU".to_vec(),
                    observed_at: Utc::now(),
                }),
                _ => AuthoritativeSinkResult::Rejected(TypedRejection {
                    reason_code: "TEST_CODE_MEMORY_REJECTED".into(),
                    evidence: b"TEST_CODE_NO_FEISHU".to_vec(),
                    retry_authorized: false,
                    observed_at: Utc::now(),
                }),
            };
            let sink = Arc::new(Sink(sink_result, std::sync::atomic::AtomicUsize::new(0)));
            coordinator
                .resume_deliverable(&envelope.decision_identity, &[sink.clone()], Utc::now())
                .unwrap();
            coordinator
                .reconcile_all_pending(&append, Utc::now())
                .unwrap();
            assert_eq!(
                coordinator
                    .decision_state(&envelope.decision_identity)
                    .unwrap(),
                expected
            );
            drop(coordinator);
            let coordinator =
                DurableDeliveryCoordinator::open_existing_monitor_schema9(CoordinatorConfig::test(
                    &case_path,
                    &case_code,
                    "TEST_CODE_READBACK_REOPEN_OWNER_0123456789abcdef",
                ))
                .unwrap();
            let owner = coordinator
                .inspect_exact_occurrence_owner(
                    &binding.business_date().to_string(),
                    PushKind::IntradayMarket,
                    DeliverySubKind::None,
                    "GLOBAL",
                    &occurrence,
                )
                .unwrap()
                .unwrap();
            assert_eq!(owner.envelope, envelope);
            assert_eq!(owner.state, expected);
            assert_eq!(sink.1.load(std::sync::atomic::Ordering::SeqCst), 1);
            // Original quotes are not made fresh by sink acceptance or a slot.
            std::thread::sleep(std::time::Duration::from_secs(6));
            let scanner = TieredScanner::new(Vec::new());
            assert!(stale
                .iter()
                .all(|quote| scanner.validate_admitted_quote(quote).is_err()));
            results.push(serde_json::json!({"case":name,"state":format!("{:?}",expected),"occurrence":occurrence,"source_fingerprint":fingerprint,"original_quotes":original,"decision_identity":envelope.decision_identity,"memory_sink_calls":1,"feishu_calls":0}));
        }
        std::fs::write(
            base.join("opaque-readback-results.json"),
            serde_json::to_vec_pretty(&results).unwrap(),
        )
        .unwrap();
    }
}
