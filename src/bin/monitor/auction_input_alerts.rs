//! Auction input notices share T-02's ordinary counted authority, not trading
//! authority. Episodes are derived from frozen durable occurrences, not memory.
use chrono::{DateTime, NaiveDate, Timelike, Utc};
use sha2::{Digest, Sha256};
use stock_analysis::monitor::data_mode::DataMode;

use crate::durable_delivery_runtime::{
    CountedDeliveryBinding, CountedDeliveryOrigin, CountedDeliveryScope,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Input {
    PositionQuotes,
    VolumeCandidates,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Unavailable,
    Recovered,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    date: NaiveDate,
}
impl Slot {
    fn at(now: DateTime<Utc>) -> Option<Self> {
        let local = now.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap());
        let minute = local.hour() * 60 + local.minute();
        ((560..565).contains(&minute)
            && stock_analysis::calendar::verified_a_share_trading_day(local.date_naive()).ok()?)
        .then_some(Self {
            date: local.date_naive(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Reason {
    AcquisitionUnavailable,
    CandidatesEmpty,
    VolumeRatioUnavailable,
    InvalidCandidates,
    FreshBatch,
}
impl Reason {
    fn message(self, input: Input) -> &'static str {
        match (input, self) {
            (Input::PositionQuotes, Reason::AcquisitionUnavailable) => {
                "持仓实时报价采集未取得可用批次，本轮持仓报价观察不可用。"
            }
            (Input::VolumeCandidates, Reason::AcquisitionUnavailable) => {
                "P-02量能输入采集未取得可用批次，本轮量能候选不可用。"
            }
            (Input::VolumeCandidates, Reason::CandidatesEmpty) => {
                "量能候选为空：本轮已验证的涨停池为空。"
            }
            (Input::VolumeCandidates, Reason::VolumeRatioUnavailable) => {
                "P-02量能输入缺少有效量比，本轮无法形成量能候选。"
            }
            (Input::VolumeCandidates, Reason::InvalidCandidates) => {
                "P-02量能输入未形成有效候选，本轮不发送量能候选卡。"
            }
            (Input::PositionQuotes, Reason::FreshBatch) => {
                "持仓实时报价输入恢复：本轮新批次已通过报价准入检查。"
            }
            (Input::VolumeCandidates, Reason::FreshBatch) => {
                "P-02量能输入恢复：本轮新批次已取得有效量能候选。"
            }
            _ => "竞价输入状态不可用。",
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Fact {
    schema: String,
    window: String,
    input: Input,
    episode: u32,
    phase: Phase,
    reason: Reason,
    observed_at: DateTime<Utc>,
    batch_observed_at: Option<DateTime<Utc>>,
    batch_sha256: Option<String>,
    preceding_unavailable_sha256: Option<String>,
}

#[derive(Clone)]
struct Owner {
    fact: Fact,
    source_sha256: String,
}
enum Observation {
    Unavailable(Reason),
    Fresh {
        observed_at: DateTime<Utc>,
        batch_sha256: String,
    },
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn fingerprint(slot: Slot, input: Input, episode: u32, phase: Phase) -> String {
    hash(
        &serde_json::to_vec(&serde_json::json!({
            "domain": "auction-input-alert-v1", "date": slot.date.to_string(),
            "window": "09:20-09:25", "input": input, "episode": episode, "phase": phase
        }))
        .unwrap(),
    )
}
fn parse_owner(
    source: &[u8],
    slot: Slot,
    input: Input,
    episode: u32,
    phase: Phase,
) -> Result<Owner, String> {
    let value: serde_json::Value =
        serde_json::from_slice(source).map_err(|_| "auction_input_source_invalid")?;
    let fact: Fact = serde_json::from_value(value["auction_input_alert"].clone())
        .map_err(|_| "auction_input_fact_invalid")?;
    if value["schema"] != "data-mode-v2"
        || value["business_date"] != slot.date.to_string()
        || value["old"] != value["new"]
        || value["fact_fingerprint"] != fingerprint(slot, input, episode, phase)
        || !matches!(value["new"].as_str(), Some("Full" | "Degraded" | "Unsafe"))
        || fact.schema != "auction-input-alert-v1"
        || fact.window != "09:20-09:25"
        || fact.input != input
        || fact.episode != episode
        || fact.phase != phase
        || !(1..=100).contains(&episode)
        || Slot::at(fact.observed_at) != Some(slot)
        || (phase == Phase::Unavailable
            && (fact.reason == Reason::FreshBatch
                || fact.batch_observed_at.is_some()
                || fact.batch_sha256.is_some()
                || fact
                    .preceding_unavailable_sha256
                    .as_ref()
                    .is_some_and(|s| episode == 1 || !valid_hash(s))))
        || (phase == Phase::Recovered
            && (fact.reason != Reason::FreshBatch
                || fact.batch_observed_at.is_none_or(|at| {
                    at > fact.observed_at
                        || Slot::at(at) != Some(slot)
                        || fact.observed_at.signed_duration_since(at)
                            > chrono::Duration::seconds(30)
                })
                || fact.batch_sha256.as_ref().is_none_or(|s| !valid_hash(s))
                || fact
                    .preceding_unavailable_sha256
                    .as_ref()
                    .is_none_or(|s| !valid_hash(s))))
        || (input == Input::PositionQuotes
            && !matches!(
                fact.reason,
                Reason::AcquisitionUnavailable | Reason::FreshBatch
            ))
    {
        return Err("auction_input_fact_mismatch".into());
    }
    Ok(Owner {
        fact,
        source_sha256: hash(source),
    })
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A changed global DataMode must not create another owner for the same input
/// phase. The lookup checks the three real-mode occurrence variants.
fn next_fact(
    slot: Slot,
    input: Input,
    observation: Observation,
    now: DateTime<Utc>,
    mut lookup: impl FnMut(&str) -> Result<Option<Vec<u8>>, String>,
) -> Result<Option<Fact>, String> {
    if Slot::at(now) != Some(slot) {
        return Err("auction_input_window_expired".into());
    }
    let mut latest = None::<(Owner, bool)>;
    let mut next_episode = 1;
    for episode in 1..=100 {
        let unavailable = lookup(&fingerprint(slot, input, episode, Phase::Unavailable))?
            .map(|bytes| parse_owner(&bytes, slot, input, episode, Phase::Unavailable))
            .transpose()?;
        let recovered = lookup(&fingerprint(slot, input, episode, Phase::Recovered))?
            .map(|bytes| parse_owner(&bytes, slot, input, episode, Phase::Recovered))
            .transpose()?;
        match (unavailable, recovered) {
            (None, Some(_)) => return Err("auction_input_recovery_without_original".into()),
            (None, None) => {
                next_episode = episode;
                break;
            }
            (Some(old), new) => {
                if let Some((previous, was_recovered)) = latest.as_ref() {
                    if old.fact.observed_at <= previous.fact.observed_at
                        || (!was_recovered
                            && (old.fact.reason == previous.fact.reason
                                || old.fact.preceding_unavailable_sha256.as_ref()
                                    != Some(&previous.source_sha256)))
                        || (*was_recovered && old.fact.preceding_unavailable_sha256.is_some())
                    {
                        return Err("auction_input_episode_chain_invalid".into());
                    }
                }
                if let Some(new) = new {
                    if new.fact.preceding_unavailable_sha256.as_ref() != Some(&old.source_sha256)
                        || new
                            .fact
                            .batch_observed_at
                            .is_none_or(|at| at <= old.fact.observed_at)
                        || new.fact.observed_at < old.fact.observed_at
                    {
                        return Err("auction_input_recovery_chain_invalid".into());
                    }
                    latest = Some((new, true));
                } else {
                    latest = Some((old, false));
                }
                next_episode = episode + 1;
            }
        }
    }
    if next_episode > 100 {
        return Err("auction_input_episode_limit".into());
    }
    if latest
        .as_ref()
        .is_some_and(|(old, _)| now <= old.fact.observed_at)
    {
        return Ok(None);
    }
    let fact = match (&latest, observation) {
        (Some((old, false)), Observation::Unavailable(reason)) if old.fact.reason == reason => {
            return Ok(None)
        }
        (_, Observation::Unavailable(reason)) => Fact {
            schema: "auction-input-alert-v1".into(),
            window: "09:20-09:25".into(),
            input,
            episode: next_episode,
            phase: Phase::Unavailable,
            reason,
            observed_at: now,
            batch_observed_at: None,
            batch_sha256: None,
            preceding_unavailable_sha256: latest
                .as_ref()
                .filter(|(_, recovered)| !recovered)
                .map(|(old, _)| old.source_sha256.clone()),
        },
        (
            Some((old, false)),
            Observation::Fresh {
                observed_at,
                batch_sha256,
            },
        ) => {
            if observed_at <= old.fact.observed_at
                || observed_at > now
                || now.signed_duration_since(observed_at) > chrono::Duration::seconds(30)
                || Slot::at(observed_at) != Some(slot)
                || !valid_hash(&batch_sha256)
            {
                return Ok(None);
            }
            Fact {
                schema: "auction-input-alert-v1".into(),
                window: "09:20-09:25".into(),
                input,
                episode: old.fact.episode,
                phase: Phase::Recovered,
                reason: Reason::FreshBatch,
                observed_at: now,
                batch_observed_at: Some(observed_at),
                batch_sha256: Some(batch_sha256),
                preceding_unavailable_sha256: Some(old.source_sha256.clone()),
            }
        }
        (_, Observation::Fresh { .. }) => return Ok(None),
    };
    Ok(Some(fact))
}

/// Only this module can construct the source-only operational capability.
pub(super) struct PreparedAuctionInputAlert {
    slot: Slot,
    text: String,
    binding: CountedDeliveryBinding,
}
impl PreparedAuctionInputAlert {
    fn prepare(slot: Slot, fact: Fact, mode: DataMode) -> Result<Self, String> {
        let text = format!("竞价输入状态（{} {}，北京时间）\n{}\n仅报告该输入本轮状态；不表示整体数据健康或交易条件恢复。",
            slot.date, fact.observed_at.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap()).format("%H:%M:%S"), fact.reason.message(fact.input));
        // The mode is the real aggregate mode, never an input-state surrogate.
        let original = crate::push_templates::build_data_mode_counted_binding(
            slot.date,
            Some(mode),
            mode,
            &fingerprint(slot, fact.input, fact.episode, fact.phase),
            &text,
        )?;
        let mut source: serde_json::Value =
            serde_json::from_slice(original.source_binding_canonical())
                .map_err(|e| e.to_string())?;
        source["auction_input_alert"] = serde_json::to_value(fact).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec(&source).map_err(|e| e.to_string())?;
        let binding = CountedDeliveryBinding::new(
            slot.date,
            original.schedule_occurrence_identity(),
            bytes.clone(),
            CountedDeliveryScope::Global,
            hash(&bytes),
            CountedDeliveryOrigin::InternalDurable,
            None,
            false,
        )?;
        Ok(Self {
            slot,
            text,
            binding,
        })
    }
    pub(super) fn binding(&self) -> &CountedDeliveryBinding {
        &self.binding
    }
    pub(super) fn validate_for_use(&self) -> Result<(), String> {
        self.validate_at(Utc::now())
    }
    fn validate_at(&self, now: DateTime<Utc>) -> Result<(), String> {
        if Slot::at(now) != Some(self.slot) {
            return Err("auction_input_window_expired".into());
        }
        if self.binding.retry_authorized() || self.binding.task_binding().is_some() {
            return Err("auction_input_binding_invalid".into());
        }
        let source: serde_json::Value =
            serde_json::from_slice(self.binding.source_binding_canonical())
                .map_err(|_| "auction_input_source_invalid")?;
        if source["rendered_sha256"] != hash(self.text.as_bytes()) {
            return Err("auction_input_text_invalid".into());
        }
        Ok(())
    }
    pub(super) fn into_parts(self) -> (String, CountedDeliveryBinding) {
        (self.text, self.binding)
    }
}

async fn observe(input: Input, observation: Observation, now: DateTime<Utc>) {
    let Some(slot) = Slot::at(now) else {
        return;
    };
    let result = (|| {
        let fact = next_fact(slot, input, observation, now, |fingerprint| {
            crate::durable_delivery_runtime::auction_input_occurrence_source(slot.date, fingerprint)
        })?;
        let Some(fact) = fact else {
            return Ok(None);
        };
        let health = stock_analysis::monitor::data_mode::evaluate(
            &stock_analysis::monitor::data_mode::current_data_health_input(120, 600)?,
            None,
        );
        PreparedAuctionInputAlert::prepare(slot, fact, health.mode).map(Some)
    })();
    match result {
        Ok(Some(prepared)) => {
            let occurrence = prepared.binding.schedule_occurrence_identity().to_owned();
            let outcome = crate::notify::push_auction_input_alert(prepared).await;
            log::info!(
                "[auction-input] input={input:?} occurrence={occurrence} outcome={outcome:?}"
            );
        }
        Ok(None) => {}
        Err(error) => log::error!("[auction-input] input={input:?} notice unavailable: {error}"),
    }
}

pub(super) async fn position_failure(now: DateTime<Utc>) {
    observe(
        Input::PositionQuotes,
        Observation::Unavailable(Reason::AcquisitionUnavailable),
        now,
    )
    .await;
}
pub(super) async fn position_batch(
    quotes: &crate::market_data::ScannerPositionQuotes,
    now: DateTime<Utc>,
) {
    // NoPositions is an empty holding scope, not a failed RPC or recovery.
    if quotes.quotes().is_empty() {
        return;
    }
    let scanner = stock_analysis::monitor::scanner::TieredScanner::new(Vec::new());
    if quotes
        .quotes()
        .iter()
        .any(|quote| scanner.validate_admitted_quote(quote).is_err())
    {
        position_failure(now).await;
        return;
    }
    let observed_at = quotes
        .quotes()
        .iter()
        .map(|q| q.observed_at())
        .min()
        .unwrap();
    let batches = quotes.quotes().iter().map(|q| serde_json::json!({
        "code": q.code(), "price": q.price(), "previous_close": q.previous_close(),
        "change_percent": q.change_percent(), "source_at": q.source_at(), "observed_at": q.observed_at(),
        "provider": q.evidence().provider, "source": q.evidence().source, "batch_id": q.evidence().batch_id,
        "batch_source_at": q.evidence().source_at, "batch_observed_at": q.evidence().observed_at,
    })).collect::<Vec<_>>();
    observe(
        Input::PositionQuotes,
        Observation::Fresh {
            observed_at,
            batch_sha256: hash(&serde_json::to_vec(&batches).unwrap()),
        },
        now,
    )
    .await;
}
pub(super) async fn volume_failure(now: DateTime<Utc>) {
    observe(
        Input::VolumeCandidates,
        Observation::Unavailable(Reason::AcquisitionUnavailable),
        now,
    )
    .await;
}
fn volume_reason(
    snapshot: &Result<
        crate::push_templates::AuctionVolumeSnapshot,
        crate::push_templates::AuctionVolumeSelectionError,
    >,
) -> Option<Reason> {
    use crate::push_templates::AuctionVolumeSelectionError as E;
    match snapshot {
        Ok(_) => None,
        Err(E::SourceRowsEmpty) => Some(Reason::CandidatesEmpty),
        Err(E::NoEligibleUnnotifiedRows(counts)) if counts.all_source_rows_valid_and_notified() => {
            None
        }
        Err(E::NoEligibleUnnotifiedRows(counts))
            if counts.missing_volume_ratio_rows() > 0 || counts.invalid_volume_ratio_rows() > 0 =>
        {
            Some(Reason::VolumeRatioUnavailable)
        }
        Err(_) => Some(Reason::InvalidCandidates),
    }
}
pub(super) async fn volume_batch(
    tick: &crate::push_templates::AuctionVolumeTickData,
    now: DateTime<Utc>,
) {
    let Some(source) = tick.source_observation() else {
        return;
    };
    let evidence = source.limit_pool_batch().evidence();
    let Ok(observed_at) = stock_analysis::data_gateway::parse_evidence_instant(
        "LimitPools",
        evidence.provider,
        "observed_at",
        &evidence.observed_at,
    ) else {
        volume_failure(now).await;
        return;
    };
    if source.trading_date()
        != now
            .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
            .date_naive()
        || observed_at > now
        || now.signed_duration_since(observed_at) > chrono::Duration::seconds(30)
        || Slot::at(observed_at) != Slot::at(now)
    {
        volume_failure(now).await;
        return;
    }
    if let Some(reason) = volume_reason(&tick.snapshot) {
        observe(
            Input::VolumeCandidates,
            Observation::Unavailable(reason),
            now,
        )
        .await;
        return;
    }
    observe(Input::VolumeCandidates, Observation::Fresh { observed_at,
        batch_sha256: hash(&serde_json::to_vec(&serde_json::json!({
            "request_hash": source.limit_pool_request_hash(), "batch_id": evidence.batch_id,
            "source": evidence.source, "provider": evidence.provider, "source_at": evidence.source_at,
            "observed_at": evidence.observed_at, "audit_record_hash": source.limit_pool_receipt().record_hash,
            "composition_record_hash": source.composition_receipt().map(|r| &r.record_hash),
            "name_shards": source.name_shards().iter().map(|shard| serde_json::json!({
                "request_hash": shard.request_hash(), "record_hash": shard.receipt().record_hash,
                "batch_id": shard.batch().evidence().batch_id,
            })).collect::<Vec<_>>(),
        })).unwrap()) }, now).await;
}

/// Denial-only final guard for frozen auction inputs. Other T-02 status cards
/// retain their existing authority; this guard grants no caller a lower gate.
pub(super) fn validate_authoritative_request(
    namespace: &crate::durable_delivery_runtime::RuntimeNamespace,
    request: &stock_analysis::durable_delivery::AuthoritativeDeliveryRequest,
    now: DateTime<Utc>,
) -> Result<(), String> {
    use crate::durable_delivery_runtime::RuntimeNamespace;
    if request.push_kind != stock_analysis::durable_delivery::PushKind::DataMode {
        return Ok(());
    }
    let relative = match namespace {
        RuntimeNamespace::Production => std::path::PathBuf::from("data/durable_delivery.sqlite3"),
        RuntimeNamespace::Test { test_code } => std::path::PathBuf::from("data/test")
            .join(test_code)
            .join("durable_delivery.sqlite3"),
    };
    let path = stock_analysis::production_root::root_for_mode(matches!(
        namespace,
        RuntimeNamespace::Test { .. }
    ))
    .join(relative);
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "auction_input_store_unavailable")?;
    let (bytes, stored_hash): (Vec<u8>, String) = connection.query_row(
        "SELECT envelope_canonical,envelope_sha256 FROM delivery_decisions WHERE decision_identity=?1",
        [&request.decision_identity], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|_| "auction_input_original_missing")?;
    if hash(&bytes) != stored_hash {
        return Err("auction_input_original_hash_mismatch".into());
    }
    validate_stored_canonical(&bytes, request, now)?;
    let envelope: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "auction_input_original_invalid")?;
    let source_bytes: Vec<u8> =
        serde_json::from_value(envelope["source_binding_canonical"].clone())
            .map_err(|_| "auction_input_original_invalid")?;
    let Ok(source) = serde_json::from_slice::<serde_json::Value>(&source_bytes) else {
        return Ok(());
    };
    if source.get("auction_input_alert").is_none() {
        return Ok(());
    }
    let fact: Fact = serde_json::from_value(source["auction_input_alert"].clone())
        .map_err(|_| "auction_input_fact_invalid")?;
    if fact.preceding_unavailable_sha256.is_some() {
        // Schema9 keeps the source hash inside its frozen envelope, not in a
        // projection column. Read only this day's T-02 rows and verify exact
        // original bytes before selecting the predecessor by source SHA.
        let mut statement = connection.prepare("SELECT envelope_canonical,envelope_sha256 FROM delivery_decisions WHERE push_kind='DataMode' AND business_date=?1").map_err(|_| "auction_input_predecessor_read_failed")?;
        let rows = statement
            .query_map([envelope["business_date"].as_str()], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|_| "auction_input_predecessor_read_failed")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| "auction_input_predecessor_read_failed")?;
        let mut originals = Vec::new();
        for (canonical, stored_hash) in rows {
            if hash(&canonical) != stored_hash {
                return Err("auction_input_predecessor_invalid".into());
            }
            let candidate: serde_json::Value = serde_json::from_slice(&canonical)
                .map_err(|_| "auction_input_predecessor_invalid")?;
            let candidate_bytes: Vec<u8> =
                serde_json::from_value(candidate["source_binding_canonical"].clone())
                    .map_err(|_| "auction_input_predecessor_invalid")?;
            if Some(hash(&candidate_bytes)) == fact.preceding_unavailable_sha256 {
                originals.push(candidate_bytes);
            }
        }
        if originals.len() != 1 {
            return Err("auction_input_predecessor_invalid".into());
        }
        let previous_episode = if fact.phase == Phase::Recovered {
            fact.episode
        } else {
            fact.episode
                .checked_sub(1)
                .ok_or("auction_input_predecessor_invalid")?
        };
        let old = parse_owner(
            &originals[0],
            Slot::at(now).ok_or("auction_input_window_expired")?,
            fact.input,
            previous_episode,
            Phase::Unavailable,
        )?;
        if fact.preceding_unavailable_sha256.as_ref() != Some(&old.source_sha256)
            || fact.observed_at <= old.fact.observed_at
            || (fact.phase == Phase::Recovered
                && fact
                    .batch_observed_at
                    .is_none_or(|at| at <= old.fact.observed_at))
            || (fact.phase == Phase::Unavailable && fact.reason == old.fact.reason)
        {
            return Err("auction_input_predecessor_invalid".into());
        }
    }
    Ok(())
}

fn validate_stored_canonical(
    bytes: &[u8],
    request: &stock_analysis::durable_delivery::AuthoritativeDeliveryRequest,
    now: DateTime<Utc>,
) -> Result<(), String> {
    const INVALID: &str = "auction_input_original_invalid";
    let envelope: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| INVALID)?;
    let source_bytes: Vec<u8> =
        serde_json::from_value(envelope["source_binding_canonical"].clone())
            .map_err(|_| INVALID)?;
    let Ok(source) = serde_json::from_slice::<serde_json::Value>(&source_bytes) else {
        return Ok(());
    };
    if source.get("auction_input_alert").is_none() {
        return Ok(());
    }
    let fact: Fact =
        serde_json::from_value(source["auction_input_alert"].clone()).map_err(|_| INVALID)?;
    let slot = Slot::at(now).ok_or("auction_input_window_expired")?;
    let owner = parse_owner(&source_bytes, slot, fact.input, fact.episode, fact.phase)?;
    let occurrence = format!(
        "data-mode-v2:{}:{}:{}",
        slot.date,
        source["new"].as_str().ok_or(INVALID)?,
        fingerprint(slot, fact.input, fact.episode, fact.phase)
    );
    if envelope["decision_identity"] != request.decision_identity
        || envelope["push_kind"] != "DataMode"
        || envelope["sub_kind"] != "NONE"
        || envelope["scope_key"] != "GLOBAL"
        || envelope["business_date"] != slot.date.to_string()
        || envelope["schedule_occurrence_identity"] != occurrence
        || envelope["retry_authorized"] != false
        || envelope["task_binding"] != serde_json::Value::Null
        || envelope["source_binding_sha256"] != owner.source_sha256
        || envelope["source_evidence_fingerprint"] != owner.source_sha256
        || envelope["delivery_subject_hash"] != owner.source_sha256
        || envelope["rendered_content_sha256"] != request.rendered_content_sha256
        || source["rendered_sha256"] != request.rendered_content_sha256
        || hash(&request.rendered_content) != request.rendered_content_sha256
        || fact.observed_at > now
    {
        return Err(INVALID.into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "auction_input_alerts_tests.rs"]
mod tests;
