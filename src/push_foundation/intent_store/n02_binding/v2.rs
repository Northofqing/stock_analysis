//! Strict N02 source proof stored in the existing immutable Ready payload.
//! Only test fixtures can write this development contract; production can only
//! read and reject invalid rows until a producer/catalog review is complete.

use super::*;
use crate::monitor::push_job::{
    n02_replay_evidence_fingerprint, n02_replay_prepared_facts_sha256, source_ref_value,
    source_time_value, N02SelectedProofV1, N02SourceChainV1, PreparedFactsSnapshot, SourceRef,
    SourceTime, N02_SOURCE_CONTRACT_ID, N02_SOURCE_CONTRACT_VERSION,
};
use crate::news::aggregator::raw_v2::{
    replay_n02_admitted_record, NewsFlashRecordEvidenceV1, MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES,
};

const DOMAIN_V2: &str = "N02PreparedPush/v2";
const MAX_N02_V2_BYTES: usize = 8 * MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES;
const MAX_GENERIC_PREPARED_BYTES: usize = 64 * 1024;

fn proof(ok: bool, field: &'static str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(N02BindingError::PreparedIdentityMismatch { field })
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[usize::from(byte >> 4)] as char);
        output.push(DIGITS[usize::from(byte & 15)] as char);
    }
    output
}

fn unhex(value: &str, max_len: usize, field: &'static str) -> Result<Vec<u8>> {
    if value.len() % 2 != 0 || value.len() > max_len.saturating_mul(2) {
        return Err(invalid(field));
    }
    let digit = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = digit(pair[0]).ok_or_else(|| invalid(field))?;
            let low = digit(pair[1]).ok_or_else(|| invalid(field))?;
            Ok((high << 4) | low)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn wrapper_v2(
    prepared: &[u8],
    binding: &N02ReservationBindingV1,
    records: &[&NewsFlashRecordEvidenceV1],
    source_refs: &[SourceRef],
    source_times: &[SourceTime],
    facts_len: usize,
    facts_sha256: &Sha256Digest,
    rendered_raw_sha256: &Sha256Digest,
) -> Vec<u8> {
    canonical_preimage(
        DOMAIN_V2,
        &BTreeMap::from([
            (
                "admitted_records_hex",
                CanonicalValue::Array(
                    records
                        .iter()
                        .map(|record| string(hex(record.canonical_bytes())))
                        .collect(),
                ),
            ),
            (
                "canonical_facts",
                CanonicalValue::Object(BTreeMap::from([
                    ("length", CanonicalValue::Unsigned(facts_len as u64)),
                    ("sha256", string(facts_sha256.as_str())),
                ])),
            ),
            ("prepared_push_hex", string(hex(prepared))),
            ("rendered_raw_sha256", string(rendered_raw_sha256.as_str())),
            ("reservation", CanonicalValue::Object(binding.fields())),
            ("source_contract_id", string(N02_SOURCE_CONTRACT_ID)),
            (
                "source_contract_version",
                string(N02_SOURCE_CONTRACT_VERSION),
            ),
            (
                "source_refs",
                CanonicalValue::Array(source_refs.iter().map(source_ref_value).collect()),
            ),
            (
                "source_times",
                CanonicalValue::Array(source_times.iter().map(source_time_value).collect()),
            ),
        ]),
    )
}

#[cfg(test)]
impl InitialIntentDraft {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ready_n02_source_v2_test(
        identity: InitialIntentIdentity,
        prepared: &PreparedPush,
        facts: &PreparedFactsSnapshot,
        selected: &N02SelectedProofV1<'_>,
        template_sha256: Sha256Digest,
        source_contract_sha256: Sha256Digest,
        created_at: UtcMicros,
    ) -> Result<Self> {
        proof(
            matches!(identity.namespace, Namespace::Test { .. }),
            "namespace",
        )?;
        proof(
            identity.source_contract_id.as_str() == N02_SOURCE_CONTRACT_ID,
            "source_contract_id",
        )?;
        let mut draft = Self::ready(
            identity,
            prepared,
            template_sha256,
            source_contract_sha256,
            created_at,
        )?;
        let rendered = prepared.rendered_bytes().as_bytes();
        let binding = selected.binding();
        N02ReservationBindingV1::try_from_reservation_material(
            binding.material().clone(),
            rendered,
        )?;
        validate_n02_identity(
            &draft.unit_id,
            prepared.subject(),
            &draft.business_date,
            &draft.occurrence_family,
            &draft.occurrence_key,
            prepared.occurrence(),
            binding,
        )?;
        let facts = facts.facts();
        proof(
            facts.run_context_sha256() == prepared.run_context_sha256(),
            "run_context_sha256",
        )?;
        proof(
            facts.canonical_sha256() == *prepared.prepared_facts_sha256(),
            "prepared_facts_sha256",
        )?;
        proof(
            facts.source_contract_id().as_str() == N02_SOURCE_CONTRACT_ID
                && facts.source_contract_version().as_str() == N02_SOURCE_CONTRACT_VERSION,
            "source_contract",
        )?;
        proof(
            facts.source_refs() == selected.source_refs()
                && facts.provider_observed_at() == selected.source_times()
                && facts.canonical_facts().len() == selected.facts_len()
                && facts.facts_sha256() == selected.facts_sha256()
                && !facts.verified_empty()
                && facts.model_output_refs().is_empty(),
            "selected_facts",
        )?;
        proof(
            prepared.source_binding().source_contract_id() == facts.source_contract_id()
                && prepared.source_binding().source_contract_version()
                    == facts.source_contract_version()
                && prepared.source_binding().source_refs() == selected.source_refs(),
            "source_binding",
        )?;
        let expected_fingerprint = n02_replay_evidence_fingerprint(selected.source_refs());
        proof(
            prepared.source_binding().evidence_fingerprint() == &expected_fingerprint,
            "evidence_fingerprint",
        )?;
        proof(
            selected.rendered_raw_sha256() == &raw_digest(rendered),
            "rendered_raw_sha256",
        )?;
        let generic = draft.prepared_push_bytes.as_deref().expect("Ready payload");
        proof(
            generic.len() <= MAX_GENERIC_PREPARED_BYTES,
            "prepared_push_len",
        )?;
        let bytes = wrapper_v2(
            generic,
            binding,
            selected.records(),
            selected.source_refs(),
            selected.source_times(),
            selected.facts_len(),
            selected.facts_sha256(),
            selected.rendered_raw_sha256(),
        );
        proof(bytes.len() <= MAX_N02_V2_BYTES, "v2_bytes_len")?;
        draft.payload_sha256 = Some(raw_digest(&bytes));
        draft.prepared_push_bytes = Some(bytes);
        Ok(draft)
    }
}

/// The strict reader runs before N02's authority query when the row declares
/// the development admitted-record source contract. Its comparison starts
/// from persisted record bytes, not in-memory producer facts.
pub(super) fn attest_source_v2(
    row: &IntentSnapshot,
    ready: AttestedReadyIntent,
) -> Result<AttestedN02Intent> {
    proof(
        matches!(ready.namespace, Namespace::Test { .. }),
        "namespace",
    )?;
    let bytes = row
        .prepared_push_bytes
        .as_deref()
        .ok_or(N02BindingError::UnsupportedPreparedFormat)?;
    if bytes.len() > MAX_N02_V2_BYTES {
        return Err(invalid("v2_bytes_len"));
    }
    let root = parse_domain(bytes, DOMAIN_V2)?;
    keys(
        &root,
        &[
            "admitted_records_hex",
            "canonical_facts",
            "prepared_push_hex",
            "rendered_raw_sha256",
            "reservation",
            "source_contract_id",
            "source_contract_version",
            "source_refs",
            "source_times",
        ],
    )?;
    proof(
        text(&root, "source_contract_id")? == N02_SOURCE_CONTRACT_ID
            && text(&root, "source_contract_version")? == N02_SOURCE_CONTRACT_VERSION,
        "source_contract",
    )?;
    let prepared = unhex(
        text(&root, "prepared_push_hex")?,
        MAX_GENERIC_PREPARED_BYTES,
        "prepared_push_hex",
    )?;
    let encoded = root["admitted_records_hex"]
        .as_array()
        .ok_or_else(|| invalid("admitted_records_hex"))?;
    if !(1..=3).contains(&encoded.len()) {
        return Err(invalid("admitted_records_hex"));
    }
    let selected = encoded
        .iter()
        .map(|entry| {
            let value = entry
                .as_str()
                .ok_or_else(|| invalid("admitted_records_hex"))?;
            let bytes = unhex(
                value,
                MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES,
                "admitted_records_hex",
            )?;
            replay_n02_admitted_record(&bytes).map_err(|_| invalid("admitted_record"))
        })
        .collect::<Result<Vec<_>>>()?;
    let rendered = row
        .rendered_bytes
        .as_deref()
        .ok_or_else(|| invalid("rendered_bytes"))?;
    let source_count = root["reservation"]["sources"]
        .as_array()
        .ok_or_else(|| invalid("sources"))?
        .len();
    proof(source_count == selected.len(), "source_count")?;
    let reservation = decode_reservation(&root["reservation"], rendered)?;
    let witness = N02SourceChainV1::try_capture(reservation.clone(), &selected, rendered)
        .map_err(|_| invalid("selected_source_chain"))?;
    let records = witness.records();
    let expected = wrapper_v2(
        &prepared,
        &reservation,
        records,
        witness.source_refs(),
        witness.source_times(),
        witness.canonical_facts().len(),
        witness.canonical_facts().sha256(),
        witness.rendered_raw_sha256(),
    );
    if expected != bytes {
        return Err(N02BindingError::InvalidCanonicalEncoding);
    }
    validate_n02_identity(
        &row.unit_id,
        &ready.subject,
        &row.business_date,
        &row.occurrence_family,
        &row.occurrence_key,
        &ready.occurrence,
        &reservation,
    )?;
    validate_prepared(&prepared, row, &ready)?;
    let nested = parse_domain(&prepared, "PreparedPush/v1")?;
    let source_binding = &nested["source_binding"];
    proof(
        text(source_binding, "source_contract_id")? == N02_SOURCE_CONTRACT_ID
            && text(source_binding, "source_contract_version")? == N02_SOURCE_CONTRACT_VERSION
            && source_binding["source_refs"] == root["source_refs"],
        "source_binding",
    )?;
    let run_context = digest("run_context_sha256", text(&nested, "run_context_sha256")?)?;
    let reconstructed_facts = n02_replay_prepared_facts_sha256(
        &run_context,
        &SourceContractId::try_new(N02_SOURCE_CONTRACT_ID.to_owned())
            .map_err(|_| invalid("source_contract_id"))?,
        &SourceContractVersion::try_new(N02_SOURCE_CONTRACT_VERSION.to_owned())
            .map_err(|_| invalid("source_contract_version"))?,
        witness.source_refs(),
        witness.canonical_facts().len(),
        witness.canonical_facts().sha256(),
        witness.source_times(),
    );
    proof(
        text(&nested, "prepared_facts_sha256")? == reconstructed_facts.as_str(),
        "prepared_facts_sha256",
    )?;
    let fingerprint = n02_replay_evidence_fingerprint(witness.source_refs());
    proof(
        text(source_binding, "evidence_fingerprint")? == fingerprint.as_str()
            && ready.source_evidence_fingerprint == fingerprint,
        "evidence_fingerprint",
    )?;
    Ok(AttestedN02Intent { ready, reservation })
}

#[cfg(test)]
#[path = "v2_tests.rs"]
mod tests;
