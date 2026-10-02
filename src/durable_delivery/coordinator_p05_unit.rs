//! P05 local preparation store. Stored bytes never mint source, sink or completion authority.
use super::*;
use crate::database::p05_prediction_freeze::P05UnitFreezeWithScores;
use crate::database::DatabaseManager;
use crate::opportunity::candidate_panel::{CandidateEntry, CandidateSource, EvidenceTier};
use chrono::{FixedOffset, NaiveTime};
use serde::{de::DeserializeOwned, Deserialize};

const FAMILY: &str = "MU-auction-candidates:auction-0920-0925:v1";
const POLICY: &str = "P05_AUCTION_UNIT_FIRST_OBSERVED_V1";
const MAX_BYTES: usize = 4_194_304;
const MAX_ROWS: usize = 512;

#[path = "coordinator_p05_unit_runtime.rs"]
pub(super) mod runtime;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum ObservedTier {
    Strong,
    Reference,
    Theme,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum ObservedSource {
    StockPick,
    OptimalClose,
    VolumeWatchlist,
    VolumeRealTrade,
    IndustryChain,
    NewsCatalyst,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservedEntry {
    code: String,
    name: String,
    sources: Vec<ObservedSource>,
    tier: ObservedTier,
    evidence: Vec<String>,
    price_bits: Option<u64>,
    change_bits: Option<u64>,
    heat_bits: Option<u64>,
}

fn invalid(reason: &str) -> DurableDeliveryError {
    DurableDeliveryError::PolicyMismatch(format!("P05 preparation: {reason}"))
}
fn shanghai_offset() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).unwrap()
}
fn utc_text(now: DateTime<Utc>) -> String {
    now.to_rfc3339_opts(SecondsFormat::Nanos, true)
}
fn date_day(date: &str) -> Result<NaiveDate> {
    let day = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| invalid("date invalid"))?;
    if day.to_string() != date {
        return Err(invalid("date not canonical"));
    }
    verify_day(day)?;
    Ok(day)
}
fn verify_day(day: NaiveDate) -> Result<()> {
    if !crate::calendar::verified_a_share_trading_day(day)
        .map_err(|_| invalid("calendar unavailable"))?
    {
        return Err(invalid("not a verified trading day"));
    }
    Ok(())
}
fn calendar_hash(day: NaiveDate) -> Result<String> {
    crate::calendar::verified_a_share_calendar_authority_hash(day)
        .map(str::to_owned)
        .map_err(|_| invalid("calendar unavailable"))
}
fn parse_shanghai(text: &str) -> Result<DateTime<FixedOffset>> {
    let at =
        DateTime::parse_from_rfc3339(text).map_err(|_| invalid("capture timestamp invalid"))?;
    if at.offset().local_minus_utc() != 8 * 3600
        || at.to_rfc3339_opts(SecondsFormat::Nanos, false) != text
    {
        return Err(invalid("capture must be canonical Shanghai timestamp"));
    }
    date_day(&at.date_naive().to_string())?;
    Ok(at)
}
fn parse_utc(text: &str) -> Result<DateTime<Utc>> {
    let at = DateTime::parse_from_rfc3339(text)
        .map_err(|_| invalid("UTC timestamp invalid"))?
        .with_timezone(&Utc);
    if utc_text(at) != text {
        return Err(invalid("UTC timestamp not canonical"));
    }
    Ok(at)
}
fn in_window(time: NaiveTime) -> bool {
    time >= NaiveTime::from_hms_opt(9, 20, 0).unwrap()
        && time < NaiveTime::from_hms_opt(9, 25, 0).unwrap()
}
fn fresh_window(captured: DateTime<FixedOffset>, now: DateTime<Utc>) -> Result<()> {
    let current = now.with_timezone(&shanghai_offset());
    if !in_window(captured.time())
        || !in_window(current.time())
        || current.date_naive() != captured.date_naive()
        || now < captured.with_timezone(&Utc)
    {
        return Err(invalid(
            "fresh capture and real clock must share the original auction window",
        ));
    }
    Ok(())
}
fn unit_occurrence(date: &str) -> String {
    format!("p05-auction-unit:{date}:0920-0925:v1")
}
fn board_occurrence(draft: &DraftCanonical) -> Result<String> {
    Ok(format!(
        "candidate-board:{}:{}",
        draft.business_date,
        parse_shanghai(&draft.input.captured_shanghai)?.format("%H:%M")
    ))
}
fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(invalid("canonical bytes exceed bound"));
    }
    Ok(bytes)
}
fn decode<T: Serialize + DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(invalid("stored bytes exceed bound"));
    }
    let value: T = serde_json::from_slice(bytes)?;
    if encode(&value)? != bytes {
        return Err(invalid(
            "stored bytes are not the closed canonical encoding",
        ));
    }
    Ok(value)
}
fn preimage(domain: &str, canonical: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(domain.len() + canonical.len() + 16);
    bytes.extend_from_slice(&(domain.len() as u64).to_be_bytes());
    bytes.extend_from_slice(domain.as_bytes());
    bytes.extend_from_slice(&(canonical.len() as u64).to_be_bytes());
    bytes.extend_from_slice(canonical);
    bytes
}
fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn verify_blob(id: &str, bytes: &[u8], hash: &str, image: &[u8], domain: &str) -> Result<()> {
    if !is_lower_sha256(id)
        || !is_lower_sha256(hash)
        || sha256_hex(bytes) != hash
        || preimage(domain, bytes) != image
        || sha256_hex(image) != id
    {
        return Err(invalid("stored canonical hash or identity differs"));
    }
    Ok(())
}
fn current_codes(input: &P05ObservedDraftInput) -> Vec<String> {
    input
        .entries
        .iter()
        .map(|row| row.code.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
fn strong_recipe(input: &P05ObservedDraftInput) -> Result<Vec<StrongRecipe>> {
    input
        .entries
        .iter()
        .filter(|entry| entry.tier == ObservedTier::Strong && entry.price_bits.is_some())
        .map(|entry| {
            let bits = entry.heat_bits.unwrap_or(50.0_f64.to_bits());
            if !f64::from_bits(bits).is_finite() {
                return Err(invalid("Strong sample score invalid"));
            }
            Ok(StrongRecipe {
                code: entry.code.clone(),
                score_bits: bits,
            })
        })
        .collect()
}
fn validate_input(input: &P05ObservedDraftInput) -> Result<()> {
    let captured = parse_shanghai(&input.captured_shanghai)?;
    if !in_window(captured.time()) || input.entries.is_empty() || input.entries.len() > MAX_ROWS {
        return Err(invalid("observed draft has no bounded auction batch"));
    }
    let mut codes = BTreeSet::new();
    for entry in &input.entries {
        if entry.code.is_empty()
            || entry.code.len() > 128
            || entry.name.len() > 1024
            || !codes.insert(&entry.code)
            || entry.evidence.len() > 128
            || entry.evidence.iter().any(|item| item.len() > 16_384)
            || entry.sources.len() > 6
        {
            return Err(invalid("observed entry bounds or duplicate code"));
        }
        let unique = entry
            .sources
            .iter()
            .map(|item| format!("{item:?}"))
            .collect::<BTreeSet<_>>();
        if unique.len() != entry.sources.len() {
            return Err(invalid("duplicate observed source"));
        }
    }
    for bytes in [&input.auction_rendered, &input.board_rendered] {
        if bytes.is_empty() || bytes.len() > 262_144 || std::str::from_utf8(bytes).is_err() {
            return Err(invalid("original render bytes invalid"));
        }
    }
    if !input.entries.iter().any(|entry| {
        entry
            .price_bits
            .is_some_and(|bits| f64::from_bits(bits).is_finite() && f64::from_bits(bits) > 0.0)
            && entry
                .heat_bits
                .is_some_and(|bits| f64::from_bits(bits).is_finite())
    }) {
        return Err(invalid("AuctionRepush has no eligible priced candidate"));
    }
    strong_recipe(input)?;
    Ok(())
}
fn ensure_no_legacy_family(connection: &Connection, date: &str) -> Result<()> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM delivery_decisions WHERE business_date=?1 AND push_kind IN (?2,?3,?4)",
        params![date, PushKind::AuctionRepush.as_str(), PushKind::CandidateBoard.as_str(), PushKind::CandidateInvalidated.as_str()],
        |r| r.get(0),
    )?;
    if count != 0 {
        return Err(invalid("same-day affected legacy decision or owner exists"));
    }
    Ok(())
}
fn load_prospective_origin(connection: &Connection, id: &str) -> Result<ProspectiveOrigin> {
    let reserved: i64 = connection.query_row("SELECT COUNT(*) FROM p05_baseline_origins WHERE origin_identity=?1 AND (completed_unit_identity IS NOT NULL OR completed_receipt_identity IS NOT NULL OR accepted_physical_refs IS NOT NULL)",[id],|r|r.get(0))?;
    if reserved != 0 {
        return Err(invalid("P1 cannot adopt reserved Completed origin fields"));
    }
    let (family,date,kind,bytes,hash,image):(String,String,String,Vec<u8>,String,Vec<u8>)=connection.query_row("SELECT family,business_date,origin_kind,origin_canonical,origin_sha256,origin_preimage FROM p05_baseline_origins WHERE origin_identity=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
    verify_blob(id, &bytes, &hash, &image, "p05-prospective-origin-v1")?;
    let data: ProspectiveOrigin = decode(&bytes)?;
    let day = date_day(&date)?;
    let shanghai = parse_shanghai(&data.captured_shanghai)?;
    if data.schema != "p05-prospective-origin-v1"
        || family != FAMILY
        || data.family != family
        || data.business_date != date
        || kind != "ProspectiveNoPreviousV2Baseline"
        || data.kind != kind
        || shanghai.date_naive() != day
        || shanghai.time() >= NaiveTime::from_hms_opt(9, 20, 0).unwrap()
        || parse_utc(&data.captured_utc)? != shanghai.with_timezone(&Utc)
        || data.calendar_authority_hash != calendar_hash(day)?
        || !matches!(
            data.legacy_snapshot_observation.as_str(),
            "AbsentAtAnchoredLegacyDirectory" | "AbsentAtAnchoredLegacyDateLeaf"
        )
        || data.durable_family_observation != "AbsentAffectedFamilyAtOwnedTransaction"
        || data.prediction_preparation_observation != "AbsentAtOwnedOperationalReadSnapshot"
        || data.completed_unit_identity.is_some()
        || data.completed_receipt_identity.is_some()
        || data.accepted_physical_refs.is_some()
    {
        return Err(invalid(
            "prospective origin contract differs; no Completed origin in P1",
        ));
    }
    Ok(data)
}
struct OriginObservation {
    business_date: String,
    kind: String,
    codes: Vec<String>,
    completed_unit: Option<String>,
    completed_receipt: Option<String>,
}
fn load_origin(c: &Connection, id: &str) -> Result<OriginObservation> {
    let kind: String = c.query_row(
        "SELECT origin_kind FROM p05_baseline_origins WHERE origin_identity=?1",
        [id],
        |r| r.get(0),
    )?;
    match kind.as_str() {
        "ProspectiveNoPreviousV2Baseline" => {
            let origin = load_prospective_origin(c, id)?;
            Ok(OriginObservation {
                business_date: origin.business_date,
                kind,
                codes: Vec::new(),
                completed_unit: None,
                completed_receipt: None,
            })
        }
        "CompletedUnitV2Baseline" => runtime::load_completed_origin(c, id),
        _ => Err(invalid("unknown baseline origin kind")),
    }
}
fn load_draft(
    connection: &Connection,
    date: &str,
    namespace: FileObjectIdentity,
) -> Result<Option<StoredP05Draft>> {
    load_draft_inner(connection, date, namespace, true)
}
fn load_draft_inner(
    connection: &Connection,
    date: &str,
    namespace: FileObjectIdentity,
    validate_origin: bool,
) -> Result<Option<StoredP05Draft>> {
    type Row = (
        String,
        String,
        String,
        String,
        i64,
        String,
        Vec<u8>,
        String,
        Vec<u8>,
    );
    let row:Option<Row>=connection.query_row("SELECT draft_identity,unit_occurrence,family,policy,baseline_revision,baseline_origin_identity,draft_canonical,draft_sha256,draft_preimage FROM p05_unit_drafts WHERE business_date=?1",[date],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional()?;
    let Some((id, occurrence, family, policy, revision, origin_id, bytes, hash, image)) = row
    else {
        return Ok(None);
    };
    let data: DraftCanonical = decode(&bytes)?;
    if !matches!(
        data.schema.as_str(),
        "p05-unit-draft-v1" | "p05-unit-draft-v2"
    ) {
        return Err(invalid("unknown draft codec"));
    }
    verify_blob(&id, &bytes, &hash, &image, &data.schema)?;
    validate_input(&data.input)?;
    let origin = if validate_origin {
        Some(load_origin(connection, &origin_id)?)
    } else {
        None
    };
    if family != FAMILY
        || data.family != family
        || policy != POLICY
        || data.policy != policy
        || data.business_date != date
        || parse_shanghai(&data.input.captured_shanghai)?
            .date_naive()
            .to_string()
            != date
        || data.unit_occurrence != occurrence
        || occurrence != unit_occurrence(date)
        || revision < 0
        || data.baseline_revision != revision
        || data.baseline_origin_identity != origin_id
        || data.current_codes != current_codes(&data.input)
        || data.strong_recipe != strong_recipe(&data.input)?
    {
        return Err(invalid("stored draft contract differs"));
    }
    if let Some(origin) = origin {
        if data.baseline_kind != origin.kind {
            return Err(invalid("draft baseline kind differs"));
        }
        validate_baseline_draft(&data, &origin)?;
    }
    Ok(Some(StoredP05Draft {
        namespace,
        identity: id,
        canonical: bytes,
        data,
    }))
}
fn require_draft(
    connection: &Connection,
    draft: &StoredP05Draft,
    namespace: FileObjectIdentity,
) -> Result<()> {
    let actual = load_draft(connection, &draft.data.business_date, namespace)?
        .ok_or_else(|| invalid("owned draft missing"))?;
    if actual != *draft {
        return Err(invalid("owned draft differs from capability"));
    }
    Ok(())
}
fn load_started(
    connection: &Connection,
    draft_id: &str,
    namespace: FileObjectIdentity,
) -> Result<Option<P05PredictionStart>> {
    let row:Option<(String,String,Vec<u8>,String,Vec<u8>)>=connection.query_row("SELECT event_identity,phase,event_canonical,event_sha256,event_preimage FROM p05_prediction_prepare_events WHERE draft_identity=?1",[draft_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
    let Some((id, phase, bytes, hash, image)) = row else {
        return Ok(None);
    };
    verify_blob(&id, &bytes, &hash, &image, "p05-prediction-start-v1")?;
    let data: StartedCanonical = decode(&bytes)?;
    parse_utc(&data.started_utc)?;
    if phase != "Started"
        || data.schema != "p05-prediction-start-v1"
        || data.phase != phase
        || data.draft_identity != draft_id
    {
        return Err(invalid("Started contract differs"));
    }
    Ok(Some(P05PredictionStart {
        namespace,
        draft_identity: draft_id.into(),
        identity: id,
        canonical: bytes,
        claimed_here: false,
    }))
}
fn original_v1_envelope(draft: &DraftCanonical, kind: PushKind) -> Result<DeliveryEnvelope> {
    let captured = parse_shanghai(&draft.input.captured_shanghai)?;
    let (schema, occurrence, rendered) = match kind {
        PushKind::AuctionRepush => (
            "auction-repush-v1",
            format!(
                "auction-repush:{}:{}",
                draft.business_date,
                captured.format("%H:%M:%S")
            ),
            draft.input.auction_rendered.clone(),
        ),
        PushKind::CandidateBoard => (
            "candidate-board-v1",
            board_occurrence(draft)?,
            draft.input.board_rendered.clone(),
        ),
        _ => return Err(invalid("unsupported P1 child kind")),
    };
    // serde_json::Value retains the existing original producer's key ordering.
    let source = serde_json::to_vec(
        &json!({"schema":schema,"business_date":draft.business_date,"rendered_sha256":sha256_hex(&rendered)}),
    )?;
    let hash = sha256_hex(&source);
    DeliveryEnvelope::new(
        &draft.business_date,
        kind,
        super::super::model::DeliverySubKind::None,
        "GLOBAL",
        occurrence,
        &hash,
        source,
        &hash,
        rendered,
        false,
        None,
    )
}
fn validate_actual_freeze(
    draft: &DraftCanonical,
    observed: &P05UnitFreezeWithScores,
) -> Result<()> {
    let frozen = observed.freeze();
    if draft.strong_recipe.is_empty()
        || frozen.business_date() != draft.business_date
        || frozen.occurrence_identity() != board_occurrence(draft)?
        || frozen.rendered_bytes() != draft.input.board_rendered
        || frozen.ordered_rows().len() != draft.strong_recipe.len()
        || observed.ordered_score_bits().len() != draft.strong_recipe.len()
        || observed
            .ordered_score_bits()
            .iter()
            .zip(frozen.ordered_rows())
            .zip(&draft.strong_recipe)
            .any(|(((row_id, bits), row), recipe)| {
                *row_id != row.prediction_row_id() || *bits != recipe.score_bits
            })
        || frozen
            .ordered_rows()
            .iter()
            .zip(&draft.strong_recipe)
            .any(|(actual, expected)| {
                actual.code() != expected.code || actual.prediction_row_id() <= 0
            })
    {
        return Err(invalid(
            "actual freeze differs from original draft; no v1 fallback",
        ));
    }
    Ok(())
}
fn freeze_observation(observed: &P05UnitFreezeWithScores) -> PredictionObservation {
    let frozen = observed.freeze();
    PredictionObservation::Frozen {
        source_canonical: frozen.source_canonical().to_vec(),
        source_sha256: frozen.source_sha256().into(),
        target_date: frozen.target_date().into(),
        calendar_authority_hash: frozen.calendar_authority_hash().into(),
        trading_dates: frozen.trading_dates().to_vec(),
        ordered_prediction_score_bits: observed
            .ordered_score_bits()
            .iter()
            .map(|(_, bits)| *bits)
            .collect(),
        ordered_rows: frozen
            .ordered_rows()
            .iter()
            .map(|row| FrozenRowObservation {
                prediction_row_id: row.prediction_row_id(),
                code: row.code().into(),
            })
            .collect(),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredBoardSourceV2 {
    schema: String,
    business_date: String,
    occurrence_identity: String,
    target_date: String,
    calendar_authority_hash: String,
    trading_dates: Vec<String>,
    rendered_sha256: String,
    ordered_rows: Vec<FrozenRowObservation>,
}
fn board_from_stored_observation(
    draft: &DraftCanonical,
    prediction: &PredictionObservation,
) -> Result<DeliveryEnvelope> {
    match prediction {
        PredictionObservation::UnlinkedNoStrong {
            occurrence_identity,
            actual_read,
        } => {
            if !draft.strong_recipe.is_empty()
                || occurrence_identity != &board_occurrence(draft)?
                || actual_read != "AbsentAtOwnedOperationalReadSnapshot"
            {
                return Err(invalid("unlinked preparation contract differs"));
            }
            original_v1_envelope(draft, PushKind::CandidateBoard)
        }
        PredictionObservation::Frozen {
            source_canonical,
            source_sha256,
            target_date,
            calendar_authority_hash,
            trading_dates,
            ordered_rows,
            ordered_prediction_score_bits,
        } => {
            let source: StoredBoardSourceV2 = decode(source_canonical)?;
            let day = date_day(&draft.business_date)?;
            let mut days = vec![day.to_string()];
            let mut target = day;
            for _ in 0..5 {
                target = crate::calendar::verified_next_a_share_trading_day(target)
                    .map_err(|_| invalid("freeze calendar range unavailable"))?;
                days.push(target.to_string());
            }
            let mut ids = BTreeSet::new();
            if draft.strong_recipe.is_empty()
                || source.schema != "candidate-board-v2"
                || source.business_date != draft.business_date
                || source.occurrence_identity != board_occurrence(draft)?
                || source.target_date != *target_date
                || *target_date != target.to_string()
                || source.calendar_authority_hash != *calendar_authority_hash
                || *calendar_authority_hash != calendar_hash(day)?
                || source.trading_dates != *trading_dates
                || *trading_dates != days
                || source.rendered_sha256 != sha256_hex(&draft.input.board_rendered)
                || source.ordered_rows != *ordered_rows
                || ordered_rows.len() != draft.strong_recipe.len()
                || ordered_prediction_score_bits.len() != draft.strong_recipe.len()
                || ordered_prediction_score_bits
                    .iter()
                    .zip(&draft.strong_recipe)
                    .any(|(bits, recipe)| *bits != recipe.score_bits)
                || ordered_rows
                    .iter()
                    .zip(&draft.strong_recipe)
                    .any(|(row, recipe)| {
                        row.prediction_row_id <= 0
                            || !ids.insert(row.prediction_row_id)
                            || row.code != recipe.code
                    })
                || sha256_hex(source_canonical) != *source_sha256
            {
                return Err(invalid("stored freeze observation contract differs"));
            }
            // Validation of old bytes only: never reconstruct FrozenCandidateBoardV2.
            DeliveryEnvelope::new(
                &draft.business_date,
                PushKind::CandidateBoard,
                super::super::model::DeliverySubKind::None,
                "GLOBAL",
                board_occurrence(draft)?,
                source_sha256,
                source_canonical.clone(),
                source_sha256,
                draft.input.board_rendered.clone(),
                false,
                None,
            )
        }
    }
}
fn load_intent(
    connection: &Connection,
    date: &str,
    namespace: FileObjectIdentity,
) -> Result<Option<StoredP05Intent>> {
    load_intent_inner(connection, date, namespace, true)
}
fn load_intent_inner(
    connection: &Connection,
    date: &str,
    namespace: FileObjectIdentity,
    validate_origin: bool,
) -> Result<Option<StoredP05Intent>> {
    let Some(draft) = load_draft_inner(connection, date, namespace, validate_origin)? else {
        return Ok(None);
    };
    type Row = (String, String, i64, Vec<u8>, String, Vec<u8>);
    let row:Option<Row>=connection.query_row("SELECT intent_identity,started_event_identity,child_count,intent_canonical,intent_sha256,intent_preimage FROM p05_unit_intents WHERE draft_identity=?1",[&draft.identity],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    let Some((id, start_id, count, bytes, hash, image)) = row else {
        return Ok(None);
    };
    let data: IntentCanonical = decode(&bytes)?;
    let schema = if draft.data.schema == "p05-unit-draft-v1" {
        "p05-unit-intent-v1"
    } else {
        "p05-unit-intent-v2"
    };
    verify_blob(&id, &bytes, &hash, &image, schema)?;
    let start = load_started(connection, &draft.identity, namespace)?
        .ok_or_else(|| invalid("intent Started missing"))?;
    if data.schema != schema
        || data.draft_identity != draft.identity
        || data.started_event_identity != start_id
        || start_id != start.identity
        || count != data.ordered_child_identities.len() as i64
        || data.invalidated != draft.data.invalidated
    {
        return Err(invalid("complete intent set differs"));
    }
    let expected = expected_children(&draft.data, &data.prediction)?;
    let mut statement=connection.prepare("SELECT child_identity,draft_identity,ordinal,child_kind,decision_identity,child_canonical,child_sha256,child_preimage FROM p05_unit_children WHERE intent_identity=?1 ORDER BY ordinal")?;
    type ChildRow = (
        String,
        String,
        i64,
        String,
        String,
        Vec<u8>,
        String,
        Vec<u8>,
    );
    let rows = statement
        .query_map([&id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<ChildRow>>>()?;
    if rows.len() != expected.len() || rows.len() != count as usize {
        return Err(invalid("intent does not contain every required child"));
    }
    let mut children = Vec::new();
    for (
        index,
        (
            (child_id, draft_id, ordinal, kind, decision_id, canonical, child_hash, child_image),
            envelope,
        ),
    ) in rows.into_iter().zip(expected).enumerate()
    {
        verify_blob(
            &child_id,
            &canonical,
            &child_hash,
            &child_image,
            "p05-unit-child-v1",
        )?;
        let child: ChildCanonical = decode(&canonical)?;
        let expected_kind = envelope.push_kind.as_str();
        if child.schema != "p05-unit-child-v1"
            || draft_id != draft.identity
            || child.draft_identity != draft.identity
            || ordinal != index as i64
            || child.ordinal != ordinal
            || kind != expected_kind
            || child.kind != kind
            || decision_id != envelope.decision_identity
            || child.envelope_canonical != envelope.canonical_bytes()?
            || data.ordered_child_identities[index] != child_id
        {
            return Err(invalid(
                "child ordinal, identity or original envelope differs",
            ));
        }
        children.push((child_id, child.envelope_canonical));
    }
    Ok(Some(StoredP05Intent {
        identity: id,
        draft_identity: draft.identity,
        canonical: bytes,
        children,
    }))
}

/// Invoked by bootstrap and every attested runtime operation, including reads.
pub(super) fn validate_rows(connection: &Connection) -> Result<()> {
    super::super::schema_p05_unit::verify_catalog(connection)?;
    let invalid_fk: i64 = connection.query_row(
        "SELECT COUNT(*) FROM pragma_foreign_key_check WHERE \"table\" GLOB 'p05_*'",
        [],
        |r| r.get(0),
    )?;
    if invalid_fk != 0 {
        return Err(invalid("P05 foreign key violation"));
    }
    let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version >= 14 {
        runtime::validate_baseline_rows(connection)?;
    } else {
        let ids = connection
            .prepare("SELECT origin_identity FROM p05_baseline_origins ORDER BY origin_identity")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for id in &ids {
            load_prospective_origin(connection, id)?;
        }
        let heads:Vec<(String,i64,String)>=connection.prepare("SELECT family,baseline_revision,origin_identity FROM p05_baseline_heads ORDER BY family")?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        if heads.len() != ids.len()
            || heads
                .iter()
                .any(|(family, rev, id)| family != FAMILY || *rev != 0 || !ids.contains(id))
        {
            return Err(invalid(
                "pre14 baseline must remain original prospective P1",
            ));
        }
    }
    // Identity is irrelevant to validation; no capability leaves this function.
    let namespace = FileObjectIdentity {
        device: 0,
        inode: 0,
        mode: 0,
        uid: 0,
    };
    let dates = connection
        .prepare("SELECT business_date FROM p05_unit_drafts ORDER BY business_date")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let head_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM p05_unit_heads", [], |r| r.get(0))?;
    if head_count != dates.len() as i64 {
        return Err(invalid("Unit head set differs"));
    }
    for date in dates {
        let draft = load_draft(connection, &date, namespace)?
            .ok_or_else(|| invalid("draft missing during validation"))?;
        let started = load_started(connection, &draft.identity, namespace)?;
        let intent = load_intent(connection, &date, namespace)?;
        let (revision,phase,current):(i64,String,Option<String>)=connection.query_row("SELECT mutation_revision,phase,intent_identity FROM p05_unit_heads WHERE draft_identity=?1",[&draft.identity],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let valid = match (revision, phase.as_str(), started, intent, current) {
            (1, "Draft", None, None, None) => true,
            (2, "Started", Some(_), None, None) => true,
            (rev, "IntentComplete", Some(_), Some(intent), Some(current))
                if rev == 3 || (version >= 14 && rev > 3) =>
            {
                intent.identity == current
            }
            _ => false,
        };
        if !valid {
            return Err(invalid(
                "Unit mutation head differs from complete stored preparation",
            ));
        }
    }
    if version >= 14 {
        runtime::validate_runtime_rows(connection)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "p05_unit_store_tests.rs"]
mod tests;

/// Raw summaries from the existing loader. No field is interpreted as QualifiedFacts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct P05ObservedSourceBytes {
    pub(crate) quote_evidence: Option<Vec<u8>>,
    pub(crate) statistics_evidence: Option<Vec<u8>>,
    pub(crate) p5_file_witnesses: Vec<u8>,
    pub(crate) p5_candidate_refs: Vec<u8>,
    pub(crate) chain_query: Vec<u8>,
    pub(crate) chain_candidate_refs: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct P05ObservedDraftInput {
    captured_shanghai: String,
    entries: Vec<ObservedEntry>,
    source_bytes: P05ObservedSourceBytes,
    auction_rendered: Vec<u8>,
    board_rendered: Vec<u8>,
}

impl P05ObservedDraftInput {
    pub(crate) fn from_observed(
        captured_at: DateTime<FixedOffset>,
        entries: &[CandidateEntry],
        source_bytes: P05ObservedSourceBytes,
        auction_rendered: Vec<u8>,
        board_rendered: Vec<u8>,
    ) -> Result<Self> {
        if captured_at.offset().local_minus_utc() != 8 * 3600 {
            return Err(invalid("capture offset must be Shanghai"));
        }
        let entries = entries
            .iter()
            .map(|entry| ObservedEntry {
                code: entry.code.clone(),
                name: entry.name.clone(),
                sources: entry
                    .sources
                    .iter()
                    .map(|source| match source {
                        CandidateSource::StockPick => ObservedSource::StockPick,
                        CandidateSource::OptimalClose => ObservedSource::OptimalClose,
                        CandidateSource::VolumeWatchlist => ObservedSource::VolumeWatchlist,
                        CandidateSource::VolumeRealTrade => ObservedSource::VolumeRealTrade,
                        CandidateSource::IndustryChain => ObservedSource::IndustryChain,
                        CandidateSource::NewsCatalyst => ObservedSource::NewsCatalyst,
                    })
                    .collect(),
                tier: match entry.tier {
                    EvidenceTier::Strong => ObservedTier::Strong,
                    EvidenceTier::Reference => ObservedTier::Reference,
                    EvidenceTier::Theme => ObservedTier::Theme,
                },
                evidence: entry.evidence.clone(),
                price_bits: entry.current_price.map(f64::to_bits),
                change_bits: entry.change_pct.map(f64::to_bits),
                heat_bits: entry.heat_score.map(f64::to_bits),
            })
            .collect();
        let input = Self {
            captured_shanghai: captured_at.to_rfc3339_opts(SecondsFormat::Nanos, false),
            entries,
            source_bytes,
            auction_rendered,
            board_rendered,
        };
        validate_input(&input)?;
        encode(&input)?;
        Ok(input)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProspectiveOrigin {
    schema: String,
    family: String,
    business_date: String,
    kind: String,
    captured_utc: String,
    captured_shanghai: String,
    calendar_authority_hash: String,
    legacy_snapshot_observation: String,
    durable_family_observation: String,
    prediction_preparation_observation: String,
    // Reserved for a later, separately validated real receipt finalizer.
    completed_unit_identity: Option<String>,
    completed_receipt_identity: Option<String>,
    accepted_physical_refs: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftCanonical {
    schema: String,
    family: String,
    policy: String,
    business_date: String,
    unit_occurrence: String,
    baseline_origin_identity: String,
    baseline_revision: i64,
    baseline_kind: String,
    input: P05ObservedDraftInput,
    current_codes: Vec<String>,
    strong_recipe: Vec<StrongRecipe>,
    invalidated: InvalidatedPreparation,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StrongRecipe {
    code: String,
    score_bits: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum InvalidatedPreparation {
    // Prospective policy initialization is not an empty market difference or Accepted.
    ProspectiveNoPreviousV2Baseline,
    CompletedBaseline {
        origin_date: String,
        unit_identity: String,
        receipt_identity: String,
        removals: Vec<InvalidatedObservation>,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InvalidatedObservation {
    code: String,
    name: String,
    prev: String,
    reason: String,
    scope_key: String,
    rendered: Vec<u8>,
}
/// Ordinary renderer input computed exclusively from the actual immutable baseline.
/// These fields carry no source, owner, completion or sink authority.
#[derive(Debug)]
pub struct P05InvalidationRenderFacts {
    business_date: String,
    hhmmss: String,
    code: String,
    name: String,
    prev: String,
    reason: String,
}
impl P05InvalidationRenderFacts {
    pub fn business_date(&self) -> &str {
        &self.business_date
    }
    pub fn hhmmss(&self) -> &str {
        &self.hhmmss
    }
    pub fn code(&self) -> &str {
        &self.code
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn previous_state(&self) -> &str {
        &self.prev
    }
    pub fn reason(&self) -> &str {
        &self.reason
    }
}
fn invalidated_envelopes(draft: &DraftCanonical) -> Result<Vec<DeliveryEnvelope>> {
    let removals = match &draft.invalidated {
        InvalidatedPreparation::ProspectiveNoPreviousV2Baseline => return Ok(Vec::new()),
        InvalidatedPreparation::CompletedBaseline { removals, .. } => removals,
    };
    removals.iter().map(|row| {
        let source=serde_json::to_vec(&json!({"schema":"candidate-invalidated-v1","business_date":draft.business_date,"code":row.code,"prev":row.prev,"reason":row.reason,"rendered_sha256":sha256_hex(&row.rendered)}))?;
        let hash=sha256_hex(&source);
        DeliveryEnvelope::new(&draft.business_date,PushKind::CandidateInvalidated,super::super::model::DeliverySubKind::None,&row.scope_key,format!("candidate-invalidated:{}:{}",draft.business_date,row.code),&hash,source,&hash,row.rendered.clone(),true,None)
    }).collect()
}
fn expected_children(
    draft: &DraftCanonical,
    prediction: &PredictionObservation,
) -> Result<Vec<DeliveryEnvelope>> {
    let mut children = vec![original_v1_envelope(draft, PushKind::AuctionRepush)?];
    children.extend(invalidated_envelopes(draft)?);
    children.push(board_from_stored_observation(draft, prediction)?);
    Ok(children)
}
fn validate_baseline_draft(draft: &DraftCanonical, origin: &OriginObservation) -> Result<()> {
    match (
        &draft.invalidated,
        &origin.completed_unit,
        &origin.completed_receipt,
    ) {
        (InvalidatedPreparation::ProspectiveNoPreviousV2Baseline, None, None)
            if draft.schema == "p05-unit-draft-v1"
                && draft.baseline_revision == 0
                && origin.business_date == draft.business_date
                && origin.kind == "ProspectiveNoPreviousV2Baseline" =>
        {
            Ok(())
        }
        (
            InvalidatedPreparation::CompletedBaseline {
                origin_date,
                unit_identity,
                receipt_identity,
                removals,
            },
            Some(unit),
            Some(receipt),
        ) if draft.schema == "p05-unit-draft-v2"
            && draft.baseline_revision > 0
            && origin.business_date < draft.business_date
            && origin_date == &origin.business_date
            && unit_identity == unit
            && receipt_identity == receipt =>
        {
            let expected = origin
                .codes
                .iter()
                .filter(|code| !draft.current_codes.contains(code))
                .collect::<Vec<_>>();
            if expected.len() != removals.len() {
                return Err(invalid(
                    "required T08 set differs from actual Completed baseline",
                ));
            }
            for (code, row) in expected.into_iter().zip(removals) {
                // The original dispatcher looks names up in the current batch;
                // removed members are absent there, so its fallback is the code.
                if code != &row.code
                    || row.name != row.code
                    || row.prev != "候选"
                    || row.reason != "从候选台消失"
                    || row.rendered.is_empty()
                    || row.rendered.len() > 262144
                    || std::str::from_utf8(&row.rendered).is_err()
                    || row.scope_key != runtime::scope_key_for_stored_code(&row.code)?
                {
                    return Err(invalid("fixed removal observation differs"));
                }
            }
            Ok(())
        }
        _ => Err(invalid("baseline provenance/date/version differs")),
    }
}

/// Verified local draft in one attested DB. It cannot be deserialized or sent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredP05Draft {
    namespace: FileObjectIdentity,
    identity: String,
    canonical: Vec<u8>,
    data: DraftCanonical,
}
impl StoredP05Draft {
    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }
    pub(crate) fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    pub(crate) fn business_date(&self) -> &str {
        &self.data.business_date
    }
    pub(crate) fn strong_samples(&self) -> Vec<(String, f64)> {
        self.data
            .strong_recipe
            .iter()
            .map(|row| (row.code.clone(), f64::from_bits(row.score_bits)))
            .collect()
    }
    pub(crate) fn board_occurrence(&self) -> Result<String> {
        board_occurrence(&self.data)
    }
    pub(crate) fn board_rendered_bytes(&self) -> &[u8] {
        &self.data.input.board_rendered
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StartedCanonical {
    schema: String,
    draft_identity: String,
    phase: String,
    started_utc: String,
}

/// Only the transaction winner may start sampling. Reopening never returns a new claim.
#[derive(Debug)]
pub(crate) struct P05PredictionStart {
    namespace: FileObjectIdentity,
    draft_identity: String,
    identity: String,
    canonical: Vec<u8>,
    claimed_here: bool,
}
impl P05PredictionStart {
    pub(crate) fn may_start_sampling(&self) -> bool {
        self.claimed_here
    }
    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenRowObservation {
    prediction_row_id: i64,
    code: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum PredictionObservation {
    Frozen {
        source_canonical: Vec<u8>,
        source_sha256: String,
        target_date: String,
        calendar_authority_hash: String,
        trading_dates: Vec<String>,
        ordered_rows: Vec<FrozenRowObservation>,
        ordered_prediction_score_bits: Vec<u64>,
    },
    UnlinkedNoStrong {
        occurrence_identity: String,
        actual_read: String,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChildCanonical {
    schema: String,
    draft_identity: String,
    ordinal: i64,
    kind: String,
    // Strictly the existing envelope bytes, with no Unit fields in its source.
    envelope_canonical: Vec<u8>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentCanonical {
    schema: String,
    draft_identity: String,
    started_event_identity: String,
    prediction: PredictionObservation,
    ordered_child_identities: Vec<String>,
    invalidated: InvalidatedPreparation,
}

/// Stored preparation observation only. A reader does not reconstruct a live freeze,
/// counted owner, Accepted receipt or permission to call a sink.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredP05Intent {
    identity: String,
    draft_identity: String,
    canonical: Vec<u8>,
    children: Vec<(String, Vec<u8>)>,
}
impl StoredP05Intent {
    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }
    pub(crate) fn draft_identity(&self) -> &str {
        &self.draft_identity
    }
    pub(crate) fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    pub(crate) fn children(&self) -> &[(String, Vec<u8>)] {
        &self.children
    }
}

impl DurableDeliveryCoordinator {
    fn p05_namespace(&self) -> Result<FileObjectIdentity> {
        self.database_binding
            .as_ref()
            .map(|binding| binding.objects[0].identity)
            .ok_or_else(|| invalid("P05 requires attested database"))
    }

    pub(crate) fn initialize_prospective_p05_family(
        &self,
        prediction_db: &DatabaseManager,
    ) -> Result<String> {
        self.initialize_prospective_p05_family_inner(prediction_db, Utc::now(), None)
    }

    fn require_p05_test_clock(&self) -> Result<()> {
        if !matches!(
            &self.config.environment,
            super::super::model::StoreEnvironment::Test { .. }
        ) {
            return Err(invalid("test clock requires actual Test environment"));
        }
        Ok(())
    }

    #[cfg(test)]
    fn initialize_prospective_p05_family_at(
        &self,
        prediction_db: &DatabaseManager,
        now: DateTime<Utc>,
        test_legacy_base: Option<&Path>,
    ) -> Result<String> {
        self.require_p05_test_clock()?;
        self.initialize_prospective_p05_family_inner(prediction_db, now, test_legacy_base)
    }

    fn initialize_prospective_p05_family_inner(
        &self,
        prediction_db: &DatabaseManager,
        now: DateTime<Utc>,
        test_legacy_base: Option<&Path>,
    ) -> Result<String> {
        let shanghai = now.with_timezone(&shanghai_offset());
        let date = shanghai.date_naive();
        verify_day(date)?;
        if shanghai.time() >= NaiveTime::from_hms_opt(9, 20, 0).unwrap() {
            return Err(invalid(
                "prospective initialization must precede 09:20 Shanghai",
            ));
        }
        let date = date.to_string();
        // Independent operational read ends here: no two DB mutexes/leases held.
        if prediction_db
            .p05_preparation_residue_for_date(&date)
            .map_err(|_| invalid("prospective prediction read unknown"))?
        {
            return Err(invalid("prospective prediction residue exists"));
        }
        let witness = self.p05_legacy_snapshot_absent(&date, test_legacy_base)?;
        let origin = ProspectiveOrigin {
            schema: "p05-prospective-origin-v1".into(),
            family: FAMILY.into(),
            business_date: date.clone(),
            kind: "ProspectiveNoPreviousV2Baseline".into(),
            captured_utc: utc_text(now),
            captured_shanghai: shanghai.to_rfc3339_opts(SecondsFormat::Nanos, false),
            calendar_authority_hash: calendar_hash(date_day(&date)?)?,
            legacy_snapshot_observation: witness,
            durable_family_observation: "AbsentAffectedFamilyAtOwnedTransaction".into(),
            prediction_preparation_observation: "AbsentAtOwnedOperationalReadSnapshot".into(),
            completed_unit_identity: None,
            completed_receipt_identity: None,
            accepted_physical_refs: None,
        };
        let canonical = encode(&origin)?;
        let preimage = preimage("p05-prospective-origin-v1", &canonical);
        let id = sha256_hex(&preimage);
        self.with_p05_immediate_transaction(&date,|tx| {
            let existing: Option<String>=tx.query_row("SELECT origin_identity FROM p05_baseline_heads WHERE family=?1",[FAMILY],|r|r.get(0)).optional()?;
            if let Some(existing)=existing { return Ok(existing); }
            if matches!(&self.config.environment,super::super::model::StoreEnvironment::Production) {
                let real=Utc::now().with_timezone(&shanghai_offset());
                if real.date_naive().to_string()!=date || real.time()>=NaiveTime::from_hms_opt(9,20,0).unwrap() || real.with_timezone(&Utc)<now { return Err(invalid("prospective real clock left pre-09:20 window")); }
            }
            ensure_no_legacy_family(tx,&date)?;
            let other_units:i64=tx.query_row("SELECT COUNT(*) FROM p05_unit_drafts",[],|r|r.get(0))?;
            if other_units!=0 { return Err(invalid("prospective Unit residue exists")); }
            // Recheck the anchored legacy namespace while inside the short local tx.
            if self.p05_legacy_snapshot_absent(&date,test_legacy_base)?!=origin.legacy_snapshot_observation { return Err(invalid("prospective legacy witness changed")); }
            tx.execute("INSERT INTO p05_baseline_origins(origin_identity,family,business_date,origin_kind,origin_canonical,origin_sha256,origin_preimage) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id,FAMILY,date,origin.kind,canonical,sha256_hex(&canonical),preimage])?;
            tx.execute("INSERT INTO p05_baseline_heads(family,baseline_revision,origin_identity) VALUES(?1,0,?2)",params![FAMILY,id])?;
            validate_rows(tx)?;
            Ok(id.clone())
        })
    }

    fn p05_legacy_snapshot_absent(
        &self,
        date: &str,
        test_legacy_base: Option<&Path>,
    ) -> Result<String> {
        let is_test = matches!(
            &self.config.environment,
            super::super::model::StoreEnvironment::Test { .. }
        );
        let root = crate::production_root::root_for_mode(is_test);
        let base = if let Some(base) = test_legacy_base {
            // This private clock/path seam is called only by cfg(test) tests.
            #[cfg(not(test))]
            {
                let _ = base;
                return Err(invalid("test namespace override unavailable"));
            }
            #[cfg(test)]
            {
                if !is_test
                    || base
                        != self
                            .config
                            .database_path
                            .parent()
                            .ok_or_else(|| invalid("test DB parent absent"))?
                {
                    return Err(invalid(
                        "prospective test path differs from owned DB namespace",
                    ));
                }
                base
            }
        } else if is_test {
            Path::new("data/test")
        } else {
            Path::new("data")
        };
        let chain = PinnedDirectoryChain::open(root, base)?;
        chain.validate()?;
        let parent = chain.parent_anchor()?;
        let directory_name = component_cstring(
            OsStr::new("candidate_board_snapshot"),
            "legacy snapshot directory",
        )?;
        // SAFETY: retained directory fd and one fixed component; newly owned on success.
        let descriptor = unsafe {
            openat(
                parent.as_raw_fd(),
                directory_name.as_ptr(),
                PIN_O_RDONLY | PIN_O_NOFOLLOW | PIN_O_NONBLOCK | PIN_O_CLOEXEC,
                0,
            )
        };
        let witness = if descriptor < 0 {
            if std::io::Error::last_os_error().raw_os_error() != Some(NO_SUCH_FILE_OS_ERROR) {
                return Err(invalid("legacy snapshot namespace unknown or alias"));
            }
            "AbsentAtAnchoredLegacyDirectory"
        } else {
            let directory = unsafe { File::from_raw_fd(descriptor) };
            let identity =
                require_trusted_directory_identity(&directory, "legacy snapshot directory", true)?;
            let name =
                component_cstring(OsStr::new(&format!("{date}.jsonl")), "legacy snapshot date")?;
            let child = unsafe {
                openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    PIN_O_RDONLY | PIN_O_NOFOLLOW | PIN_O_NONBLOCK | PIN_O_CLOEXEC,
                    0,
                )
            };
            if child >= 0 {
                drop(unsafe { File::from_raw_fd(child) });
                return Err(invalid("legacy same-day snapshot exists"));
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(NO_SUCH_FILE_OS_ERROR) {
                return Err(invalid("legacy same-day snapshot unknown or alias"));
            }
            let reopened = openat_component(
                parent,
                OsStr::new("candidate_board_snapshot"),
                PIN_O_RDONLY,
                "legacy snapshot recheck",
            )?;
            if require_trusted_directory_identity(&reopened, "legacy snapshot recheck", true)?
                != identity
            {
                return Err(invalid("legacy snapshot directory changed"));
            }
            "AbsentAtAnchoredLegacyDateLeaf"
        };
        chain.validate()?;
        Ok(witness.into())
    }

    pub(crate) fn read_p05_unit_draft(&self, date: &str) -> Result<Option<StoredP05Draft>> {
        date_day(date)?;
        let namespace = self.p05_namespace()?;
        self.with_connection(|connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let value = load_draft(&tx, date, namespace)?;
            validate_rows(&tx)?;
            tx.commit()?;
            Ok(value)
        })
    }

    pub(crate) fn store_p05_observed_draft(
        &self,
        input: &P05ObservedDraftInput,
    ) -> Result<StoredP05Draft> {
        self.store_p05_observed_draft_with_clock(input, None)
    }

    fn store_p05_observed_draft_with_clock(
        &self,
        input: &P05ObservedDraftInput,
        test_now: Option<DateTime<Utc>>,
    ) -> Result<StoredP05Draft> {
        self.store_p05_observed_draft_rendered(input, test_now, &mut |_| {
            Err(invalid(
                "T08 renderer unavailable for actual Completed baseline",
            ))
        })
    }
    pub(crate) fn store_p05_observed_draft_with_renderer(
        &self,
        input: &P05ObservedDraftInput,
        renderer: &mut impl FnMut(&P05InvalidationRenderFacts) -> Result<Vec<u8>>,
    ) -> Result<StoredP05Draft> {
        self.store_p05_observed_draft_rendered(input, None, renderer)
    }
    fn store_p05_observed_draft_rendered(
        &self,
        input: &P05ObservedDraftInput,
        test_now: Option<DateTime<Utc>>,
        renderer: &mut impl FnMut(&P05InvalidationRenderFacts) -> Result<Vec<u8>>,
    ) -> Result<StoredP05Draft> {
        if test_now.is_some() {
            self.require_p05_test_clock()?;
        }
        validate_input(input)?;
        let captured = parse_shanghai(&input.captured_shanghai)?;
        let date = captured.date_naive().to_string();
        let namespace = self.p05_namespace()?;
        self.with_p05_immediate_transaction_declared(&date,|tx,dependencies| {
            // Existing winner is restored outside the fresh window, never overwritten.
            if let Some(existing)=load_draft(tx,&date,namespace)? { return Ok(existing); }
            #[cfg(not(test))] if test_now.is_some() { return Err(invalid("test clock unavailable")); }
            fresh_window(captured,test_now.unwrap_or_else(Utc::now))?;
            ensure_no_legacy_family(tx,&date)?;
            let (origin_id,revision):(String,i64)=tx.query_row("SELECT origin_identity,baseline_revision FROM p05_baseline_heads WHERE family=?1",[FAMILY],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or_else(||invalid("NoPreviousBaseline: no real prospective origin"))?;
            let origin=load_origin(tx,&origin_id)?;
            let (schema,invalidated)=if origin.completed_unit.is_none() {
                if origin.business_date!=date || revision!=0 {return Err(invalid("NoPreviousBaseline: prospective date differs"));}
                let units:i64=tx.query_row("SELECT COUNT(*) FROM p05_unit_drafts",[],|r|r.get(0))?;
                if units!=0 {return Err(invalid("prospective Unit residue"));}
                ("p05-unit-draft-v1",InvalidatedPreparation::ProspectiveNoPreviousV2Baseline)
            } else {
                if origin.business_date>=date || revision<=0 {return Err(invalid("same day cannot create another episode or use future baseline"));}
                dependencies.require_current_baseline(tx,&origin_id)?;
                let pending:i64=tx.query_row("SELECT COUNT(*) FROM p05_unit_drafts d LEFT JOIN p05_s2_completion_heads h ON h.draft_identity=d.draft_identity WHERE d.business_date>?1 AND d.business_date<?2 AND h.first_completion_identity IS NULL",params![origin.business_date,date],|r|r.get(0))?;
                if pending!=0 {return Err(invalid("intervening incomplete Unit blocks baseline skip"));}
                let codes=current_codes(input);let mut removals=Vec::new();
                for code in origin.codes.iter().filter(|code|!codes.contains(code)) {
                    let scope=runtime::scope_key_for_new_code(code,matches!(self.config.environment,super::super::model::StoreEnvironment::Test {..}))?;
                    let facts=P05InvalidationRenderFacts {business_date:date.clone(),hhmmss:captured.format("%H:%M:%S").to_string(),code:code.clone(),name:code.clone(),prev:"候选".into(),reason:"从候选台消失".into()};
                    let rendered=renderer(&facts)?;
                    if rendered.is_empty() || rendered.len()>262144 || std::str::from_utf8(&rendered).is_err() {return Err(invalid("original T08 renderer returned invalid UTF8 bytes"));}
                    removals.push(InvalidatedObservation {code:code.clone(),name:facts.name,prev:facts.prev,reason:facts.reason,scope_key:scope,rendered});
                }
                ("p05-unit-draft-v2",InvalidatedPreparation::CompletedBaseline {origin_date:origin.business_date.clone(),unit_identity:origin.completed_unit.clone().unwrap(),receipt_identity:origin.completed_receipt.clone().unwrap(),removals})
            };
            let data=DraftCanonical {schema:schema.into(),family:FAMILY.into(),policy:POLICY.into(),business_date:date.clone(),unit_occurrence:unit_occurrence(&date),baseline_origin_identity:origin_id,baseline_revision:revision,baseline_kind:origin.kind,input:input.clone(),current_codes:current_codes(input),strong_recipe:strong_recipe(input)?,invalidated};
            fresh_window(captured,test_now.unwrap_or_else(Utc::now))?;
            let canonical=encode(&data)?;
            let preimage=preimage(&data.schema,&canonical);
            let identity=sha256_hex(&preimage);
            tx.execute("INSERT INTO p05_unit_drafts(draft_identity,unit_occurrence,business_date,family,policy,baseline_revision,baseline_origin_identity,draft_canonical,draft_sha256,draft_preimage) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![identity,data.unit_occurrence,date,FAMILY,POLICY,revision,data.baseline_origin_identity,canonical,sha256_hex(&canonical),preimage])?;
            tx.execute("INSERT INTO p05_unit_heads(draft_identity,mutation_revision,phase) VALUES(?1,1,'Draft')",[&identity])?;
            load_draft(tx,&date,namespace)?.ok_or_else(||invalid("draft insert unreadable"))
        })
    }

    pub(crate) fn claim_p05_prediction_prepare(
        &self,
        draft: &StoredP05Draft,
    ) -> Result<P05PredictionStart> {
        let namespace = self.p05_namespace()?;
        if namespace != draft.namespace {
            return Err(invalid("draft belongs to another namespace"));
        }
        self.with_p05_immediate_transaction(&draft.data.business_date,|tx| {
            require_draft(tx,draft,namespace)?;
            if let Some(mut existing)=load_started(tx,&draft.identity,namespace)? { existing.claimed_here=false; return Ok(existing); }
            if matches!(self.config.environment,super::super::model::StoreEnvironment::Production) {
                fresh_window(parse_shanghai(&draft.data.input.captured_shanghai)?,Utc::now())?;
            }
            let data=StartedCanonical {schema:"p05-prediction-start-v1".into(),draft_identity:draft.identity.clone(),phase:"Started".into(),started_utc:utc_text(Utc::now())};
            let canonical=encode(&data)?;
            let preimage=preimage("p05-prediction-start-v1",&canonical);
            let identity=sha256_hex(&preimage);
            tx.execute("INSERT INTO p05_prediction_prepare_events(event_identity,draft_identity,phase,event_canonical,event_sha256,event_preimage) VALUES(?1,?2,'Started',?3,?4,?5)",params![identity,draft.identity,canonical,sha256_hex(&canonical),preimage])?;
            require_single_cas_update(tx.execute("UPDATE p05_unit_heads SET mutation_revision=2,phase='Started' WHERE draft_identity=?1 AND mutation_revision=1 AND phase='Draft'",[&draft.identity])?,"P05 Started CAS")?;
            let mut start=load_started(tx,&draft.identity,namespace)?.ok_or_else(||invalid("Started insert unreadable"))?;
            start.claimed_here=true;
            Ok(start)
        })
    }

    /// Reads the actual operational freeze; never invokes save or holds both DBs.
    pub(crate) fn complete_p05_unit_intent_on(
        &self,
        draft: &StoredP05Draft,
        start: &P05PredictionStart,
        prediction_db: &DatabaseManager,
    ) -> Result<StoredP05Intent> {
        let namespace = self.p05_namespace()?;
        if namespace != draft.namespace
            || namespace != start.namespace
            || start.draft_identity != draft.identity
        {
            return Err(invalid(
                "P05 preparation capability namespace or draft mismatch",
            ));
        }
        // One durable read snapshot finishes before the independent operational read.
        self.with_connection(|connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
            require_draft(&tx, draft, namespace)?;
            let actual = load_started(&tx, &draft.identity, namespace)?
                .ok_or_else(|| invalid("Started claim missing"))?;
            if actual.identity != start.identity || actual.canonical != start.canonical {
                return Err(invalid("Started identity drift"));
            }
            tx.commit()?;
            Ok(())
        })?;
        let occurrence = board_occurrence(&draft.data)?;
        let freeze = prediction_db
            .read_p05_unit_freeze_with_scores(&occurrence)
            .map_err(|_| invalid("actual freeze read failed"))?;
        let (prediction, board) = match freeze {
            Some(observed) => {
                validate_actual_freeze(&draft.data, &observed)?;
                (
                    freeze_observation(&observed),
                    crate::p05_candidate_board_link::frozen_candidate_board_envelope(
                        observed.freeze(),
                    )?,
                )
            }
            None => {
                if !draft.data.strong_recipe.is_empty() {
                    return Err(invalid(
                        "Started freeze absent: ResolutionRequired, sampling cannot restart",
                    ));
                }
                if !start.claimed_here {
                    // Existing complete intent can be restored, but an unknown Started
                    // cannot mint a new v1 fallback from an absent freeze.
                    return self
                        .read_p05_unit_intent(&draft.data.business_date)?
                        .ok_or_else(|| invalid("Started absent freeze: ResolutionRequired"));
                }
                (
                    PredictionObservation::UnlinkedNoStrong {
                        occurrence_identity: occurrence.clone(),
                        actual_read: "AbsentAtOwnedOperationalReadSnapshot".into(),
                    },
                    original_v1_envelope(&draft.data, PushKind::CandidateBoard)?,
                )
            }
        };
        let auction = original_v1_envelope(&draft.data, PushKind::AuctionRepush)?;
        let mut envelopes = vec![auction];
        envelopes.extend(invalidated_envelopes(&draft.data)?);
        envelopes.push(board);
        let children = envelopes
            .into_iter()
            .enumerate()
            .map(|(ordinal, envelope)| {
                let data = ChildCanonical {
                    schema: "p05-unit-child-v1".into(),
                    draft_identity: draft.identity.clone(),
                    ordinal: ordinal as i64,
                    kind: match envelope.push_kind {
                        PushKind::AuctionRepush => "AuctionRepush",
                        PushKind::CandidateBoard => "CandidateBoard",
                        PushKind::CandidateInvalidated => "CandidateInvalidated",
                        _ => unreachable!(),
                    }
                    .into(),
                    envelope_canonical: envelope.canonical_bytes()?,
                };
                let canonical = encode(&data)?;
                let preimage = preimage("p05-unit-child-v1", &canonical);
                Ok((
                    sha256_hex(&preimage),
                    canonical,
                    preimage,
                    data,
                    envelope.decision_identity,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let data = IntentCanonical {
            schema: if draft.data.schema == "p05-unit-draft-v1" {
                "p05-unit-intent-v1"
            } else {
                "p05-unit-intent-v2"
            }
            .into(),
            draft_identity: draft.identity.clone(),
            started_event_identity: start.identity.clone(),
            prediction,
            ordered_child_identities: children.iter().map(|child| child.0.clone()).collect(),
            invalidated: draft.data.invalidated.clone(),
        };
        let canonical = encode(&data)?;
        let preimage = preimage(&data.schema, &canonical);
        let identity = sha256_hex(&preimage);
        self.with_p05_immediate_transaction(&draft.data.business_date,|tx| {
            require_draft(tx,draft,namespace)?;
            let actual_start=load_started(tx,&draft.identity,namespace)?.ok_or_else(||invalid("Started claim missing"))?;
            if actual_start.identity!=start.identity || actual_start.canonical!=start.canonical {return Err(invalid("Started identity drift"));}
            if let Some(existing)=load_intent(tx,&draft.data.business_date,namespace)? {
                if existing.canonical!=canonical {return Err(invalid("complete intent differs from original freeze"));}
                return Ok(existing);
            }
            ensure_no_legacy_family(tx,&draft.data.business_date)?;
            tx.execute("INSERT INTO p05_unit_intents(intent_identity,draft_identity,started_event_identity,child_count,intent_canonical,intent_sha256,intent_preimage) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![identity,draft.identity,start.identity,children.len() as i64,canonical,sha256_hex(&canonical),preimage])?;
            for (child_id,bytes,image,child,decision_id) in &children {
                tx.execute("INSERT INTO p05_unit_children(child_identity,draft_identity,intent_identity,ordinal,child_kind,decision_identity,child_canonical,child_sha256,child_preimage) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![child_id,draft.identity,identity,child.ordinal,child.kind,decision_id,bytes,sha256_hex(bytes),image])?;
            }
            require_single_cas_update(tx.execute("UPDATE p05_unit_heads SET mutation_revision=3,phase='IntentComplete',intent_identity=?1 WHERE draft_identity=?2 AND mutation_revision=2 AND phase='Started'",params![identity,draft.identity])?,"P05 complete intent CAS")?;
            load_intent(tx,&draft.data.business_date,namespace)?.ok_or_else(||invalid("intent insert unreadable"))
        })
    }

    pub(crate) fn read_p05_unit_intent(&self, date: &str) -> Result<Option<StoredP05Intent>> {
        date_day(date)?;
        let namespace = self.p05_namespace()?;
        self.with_connection(|connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let value = load_intent(&tx, date, namespace)?;
            validate_rows(&tx)?;
            tx.commit()?;
            Ok(value)
        })
    }
}
