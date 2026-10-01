//! Immutable legacy reservation association. This is integrity evidence, not producer admission.
use super::*;
use crate::event::envelope::news_flash_evidence_sha256;
use crate::event::news_flash_identity::{
    news_flash_render_sha256, news_flash_reservation_sha256, NewsFlashReservationIdentityFields,
};
use crate::event::{NewsFlashAuditSource, NewsFlashWindow};
use crate::monitor::push_job::{
    subject_value, ExternalId, SourceContractVersion, SourceProvider, SourceRefId,
};
use chrono::NaiveDate;
use serde_json::Value;

const DOMAIN: &str = "N02PreparedPush/v1";
mod v2;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum N02BindingError {
    #[error("unsupported N02 prepared format")]
    UnsupportedPreparedFormat,
    #[error("invalid N02 canonical encoding")]
    InvalidCanonicalEncoding,
    #[error("invalid N02 field: {field}")]
    InvalidField { field: &'static str },
    #[error("N02 reservation mismatch: {field}")]
    ReservationMismatch { field: &'static str },
    #[error("N02 prepared identity mismatch: {field}")]
    PreparedIdentityMismatch { field: &'static str },
    #[error(transparent)]
    IntentStore(#[from] IntentStoreError),
}
type Result<T> = std::result::Result<T, N02BindingError>;
fn invalid(field: &'static str) -> N02BindingError {
    N02BindingError::InvalidField { field }
}
fn same(ok: bool, field: &'static str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(N02BindingError::ReservationMismatch { field })
    }
}
fn digest(field: &'static str, s: &str) -> Result<Sha256Digest> {
    Sha256Digest::parse(field, s).map_err(|_| invalid(field))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct N02ReservationMaterial {
    pub business_date: NaiveDate,
    pub window: String,
    pub push_kind: String,
    pub decision_key: String,
    pub event_id: Option<String>,
    pub reservation_sha256: String,
    pub sources: Vec<NewsFlashAuditSource>,
    pub evidence_sha256: String,
    pub news_flash_render_sha256: String,
    pub rendered_len: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct N02ReservationBindingV1 {
    material: N02ReservationMaterial,
    window: NewsFlashWindow,
    reservation_sha256: Sha256Digest,
    evidence_sha256: Sha256Digest,
    render_sha256: Sha256Digest,
}
impl N02ReservationBindingV1 {
    pub fn try_from_reservation_material(
        material: N02ReservationMaterial,
        rendered: &[u8],
    ) -> Result<Self> {
        let window = NewsFlashWindow::parse(&material.window).map_err(|_| invalid("window"))?;
        BusinessDate::parse(&material.business_date.to_string())
            .map_err(|_| invalid("business_date"))?;
        same(
            material.push_kind == "news_flash_aggregated_v1",
            "push_kind",
        )?;
        same(material.event_id.is_none(), "event_id")?;
        same(
            material.decision_key == window.decision_key(),
            "decision_key",
        )?;
        same(
            !rendered.is_empty() && std::str::from_utf8(rendered).is_ok(),
            "rendered_bytes",
        )?;
        same(
            material.rendered_len == rendered.len() as u64,
            "rendered_len",
        )?;
        same(!material.sources.is_empty(), "sources")?;
        for source in &material.sources {
            if [
                &source.event_id,
                &source.provider,
                &source.source,
                &source.batch_id,
            ]
            .iter()
            .any(|s| s.trim().is_empty() || s.contains('\0'))
                || source.observed_at < source.published_at
            {
                return Err(invalid("sources"));
            }
        }
        let evidence_sha256 = digest("evidence_sha256", &material.evidence_sha256)?;
        let render_sha256 = digest(
            "news_flash_render_sha256",
            &material.news_flash_render_sha256,
        )?;
        let reservation_sha256 = digest("reservation_sha256", &material.reservation_sha256)?;
        same(
            evidence_sha256
                == digest(
                    "evidence_sha256",
                    &news_flash_evidence_sha256(&material.sources),
                )?,
            "evidence_sha256",
        )?;
        same(
            render_sha256
                == digest(
                    "news_flash_render_sha256",
                    &news_flash_render_sha256(rendered),
                )?,
            "news_flash_render_sha256",
        )?;
        let expected = news_flash_reservation_sha256(&NewsFlashReservationIdentityFields {
            push_kind: &material.push_kind,
            business_date: material.business_date,
            decision_key: &material.decision_key,
            event_id: None,
            window: Some(window.label()),
            evidence_sha256: evidence_sha256.as_str(),
            render_sha256: render_sha256.as_str(),
        });
        same(
            reservation_sha256 == digest("reservation_sha256", &expected)?,
            "reservation_sha256",
        )?;
        Ok(Self {
            material,
            window,
            reservation_sha256,
            evidence_sha256,
            render_sha256,
        })
    }
    pub fn material(&self) -> &N02ReservationMaterial {
        &self.material
    }
    pub(crate) fn window(&self) -> NewsFlashWindow {
        self.window
    }
    fn occurrence(&self) -> OccurrenceId {
        derive_occurrence_id(&OccurrenceIdentityMaterial::new(
            BusinessDate::parse(&self.material.business_date.to_string()).expect("validated date"),
            OccurrenceFamily::try_new("news-flash-window".into()).unwrap(),
            OccurrenceKey::try_new(self.window.label().into()).unwrap(),
        ))
    }
    fn fields(&self) -> BTreeMap<&'static str, CanonicalValue> {
        let m = &self.material;
        BTreeMap::from([
            ("business_date", string(m.business_date.to_string())),
            ("window", string(self.window.label())),
            ("push_kind", string(&m.push_kind)),
            ("decision_key", string(&m.decision_key)),
            ("event_id", CanonicalValue::Null),
            (
                "reservation_sha256",
                string(self.reservation_sha256.as_str()),
            ),
            ("evidence_sha256", string(self.evidence_sha256.as_str())),
            (
                "news_flash_render_sha256",
                string(self.render_sha256.as_str()),
            ),
            ("rendered_len", CanonicalValue::Unsigned(m.rendered_len)),
            (
                "sources",
                CanonicalValue::Array(
                    m.sources
                        .iter()
                        .map(|s| {
                            CanonicalValue::Object(BTreeMap::from([
                                ("event_id", string(&s.event_id)),
                                ("provider", string(&s.provider)),
                                ("source", string(&s.source)),
                                ("batch_id", string(&s.batch_id)),
                                ("published_at", string(s.published_at.to_rfc3339())),
                                ("observed_at", string(s.observed_at.to_rfc3339())),
                            ]))
                        })
                        .collect(),
                ),
            ),
        ])
    }
}
fn string(s: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(s.into())
}
fn wrapper(prepared: &[u8], binding: &N02ReservationBindingV1) -> Vec<u8> {
    canonical_preimage(
        DOMAIN,
        &BTreeMap::from([
            (
                "prepared_push_bytes",
                CanonicalValue::Array(
                    prepared
                        .iter()
                        .map(|b| CanonicalValue::Unsigned(u64::from(*b)))
                        .collect(),
                ),
            ),
            ("reservation", CanonicalValue::Object(binding.fields())),
        ]),
    )
}
impl InitialIntentDraft {
    pub fn ready_n02(
        identity: InitialIntentIdentity,
        prepared: &PreparedPush,
        binding: &N02ReservationBindingV1,
        template_sha256: Sha256Digest,
        source_contract_sha256: Sha256Digest,
        created_at: UtcMicros,
    ) -> Result<Self> {
        let mut draft = Self::ready(
            identity,
            prepared,
            template_sha256,
            source_contract_sha256,
            created_at,
        )?;
        N02ReservationBindingV1::try_from_reservation_material(
            binding.material.clone(),
            prepared.rendered_bytes().as_bytes(),
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
        let bytes = wrapper(
            draft.prepared_push_bytes.as_deref().expect("ready payload"),
            binding,
        );
        draft.payload_sha256 = Some(raw_digest(&bytes));
        draft.prepared_push_bytes = Some(bytes);
        Ok(draft)
    }
}
fn validate_n02_identity(
    unit: &str,
    subject: &SubjectId,
    date: &str,
    family: &str,
    key: &str,
    occurrence: &OccurrenceId,
    binding: &N02ReservationBindingV1,
) -> Result<()> {
    for (ok, field) in [
        (unit == "MU-news-flash-aggregate", "unit_id"),
        (subject == &SubjectId::Global, "subject"),
        (
            date == binding.material.business_date.to_string(),
            "business_date",
        ),
        (family == "news-flash-window", "occurrence_family"),
        (key == binding.window.label(), "occurrence_key"),
        (occurrence == &binding.occurrence(), "occurrence"),
    ] {
        if !ok {
            return Err(N02BindingError::PreparedIdentityMismatch { field });
        }
    }
    Ok(())
}
#[derive(Debug)]
pub(crate) struct AttestedN02Intent {
    pub(crate) ready: AttestedReadyIntent,
    pub(crate) reservation: N02ReservationBindingV1,
}
impl IntentSnapshot {
    pub(crate) fn attested_n02_binding(&self) -> Result<AttestedN02Intent> {
        let ready = self.attested_ready_binding()?;
        if self.source_contract_id == crate::monitor::push_job::N02_SOURCE_CONTRACT_ID {
            return v2::attest_source_v2(self, ready);
        }
        let bytes = self
            .prepared_push_bytes
            .as_deref()
            .ok_or(N02BindingError::UnsupportedPreparedFormat)?;
        let root = parse_domain(bytes, DOMAIN)?;
        keys(&root, &["prepared_push_bytes", "reservation"])?;
        let prepared = root["prepared_push_bytes"]
            .as_array()
            .ok_or_else(|| invalid("prepared_push_bytes"))?
            .iter()
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| invalid("prepared_push_bytes"))
            })
            .collect::<Result<Vec<_>>>()?;
        let reservation = decode_reservation(
            &root["reservation"],
            self.rendered_bytes
                .as_deref()
                .ok_or_else(|| invalid("rendered_bytes"))?,
        )?;
        if wrapper(&prepared, &reservation) != bytes {
            return Err(N02BindingError::InvalidCanonicalEncoding);
        }
        validate_n02_identity(
            &self.unit_id,
            &ready.subject,
            &self.business_date,
            &self.occurrence_family,
            &self.occurrence_key,
            &ready.occurrence,
            &reservation,
        )?;
        validate_prepared(&prepared, self, &ready)?;
        Ok(AttestedN02Intent { ready, reservation })
    }
}
fn parse_domain(bytes: &[u8], domain: &str) -> Result<Value> {
    let rest = bytes
        .strip_prefix(domain.as_bytes())
        .and_then(|b| b.strip_prefix(&[0]))
        .ok_or(N02BindingError::UnsupportedPreparedFormat)?;
    serde_json::from_slice(rest).map_err(|_| N02BindingError::InvalidCanonicalEncoding)
}
fn keys(v: &Value, expected: &[&str]) -> Result<()> {
    let o = v
        .as_object()
        .ok_or(N02BindingError::InvalidCanonicalEncoding)?;
    if o.len() != expected.len() || expected.iter().any(|k| !o.contains_key(*k)) {
        return Err(N02BindingError::InvalidCanonicalEncoding);
    }
    Ok(())
}
fn text<'a>(v: &'a Value, field: &'static str) -> Result<&'a str> {
    v[field].as_str().ok_or_else(|| invalid(field))
}
fn decode_reservation(v: &Value, rendered: &[u8]) -> Result<N02ReservationBindingV1> {
    keys(
        v,
        &[
            "business_date",
            "window",
            "push_kind",
            "decision_key",
            "event_id",
            "reservation_sha256",
            "sources",
            "evidence_sha256",
            "news_flash_render_sha256",
            "rendered_len",
        ],
    )?;
    let date = text(v, "business_date")?;
    BusinessDate::parse(date).map_err(|_| invalid("business_date"))?;
    if !v["event_id"].is_null() {
        return Err(invalid("event_id"));
    }
    let sources = v["sources"]
        .as_array()
        .ok_or_else(|| invalid("sources"))?
        .iter()
        .map(|s| {
            keys(
                s,
                &[
                    "event_id",
                    "provider",
                    "source",
                    "published_at",
                    "observed_at",
                    "batch_id",
                ],
            )?;
            Ok(NewsFlashAuditSource {
                event_id: text(s, "event_id")?.into(),
                provider: text(s, "provider")?.into(),
                source: text(s, "source")?.into(),
                batch_id: text(s, "batch_id")?.into(),
                published_at: chrono::DateTime::parse_from_rfc3339(text(s, "published_at")?)
                    .map_err(|_| invalid("published_at"))?,
                observed_at: chrono::DateTime::parse_from_rfc3339(text(s, "observed_at")?)
                    .map_err(|_| invalid("observed_at"))?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    N02ReservationBindingV1::try_from_reservation_material(
        N02ReservationMaterial {
            business_date: NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .map_err(|_| invalid("business_date"))?,
            window: text(v, "window")?.into(),
            push_kind: text(v, "push_kind")?.into(),
            decision_key: text(v, "decision_key")?.into(),
            event_id: None,
            reservation_sha256: text(v, "reservation_sha256")?.into(),
            sources,
            evidence_sha256: text(v, "evidence_sha256")?.into(),
            news_flash_render_sha256: text(v, "news_flash_render_sha256")?.into(),
            rendered_len: v["rendered_len"]
                .as_u64()
                .ok_or_else(|| invalid("rendered_len"))?,
        },
        rendered,
    )
}

// Decode every field into its existing typed domain, then reconstruct the exact canonical bytes.
// Model refs are absent in PreparedPush: its evidence fingerprint cannot be recomputed here.
fn validate_prepared(
    bytes: &[u8],
    row: &IntentSnapshot,
    ready: &AttestedReadyIntent,
) -> Result<()> {
    let v = parse_domain(bytes, "PreparedPush/v1")?;
    keys(
        &v,
        &[
            "decision_id",
            "intent_id",
            "occurrence",
            "prepared_facts_sha256",
            "rendered_bytes",
            "rendered_sha256",
            "run_context_sha256",
            "semantic_projection_sha256",
            "source_binding",
            "subject",
            "unit_id",
        ],
    )?;
    let mut fields = BTreeMap::new();
    for key in [
        "decision_id",
        "intent_id",
        "occurrence",
        "prepared_facts_sha256",
        "rendered_sha256",
        "run_context_sha256",
        "semantic_projection_sha256",
    ] {
        fields.insert(key, string(digest(key, text(&v, key)?)?.as_str()));
    }
    let unit = UnitId::try_new(text(&v, "unit_id")?.into()).map_err(|_| invalid("unit_id"))?;
    fields.insert("unit_id", string(unit.as_str()));
    keys(&v["subject"], &["kind", "value"])?;
    let subject = match text(&v["subject"], "kind")? {
        "Global" if v["subject"]["value"].is_null() => SubjectId::Global,
        "Entity" => SubjectId::entity(text(&v["subject"], "value")?.into())
            .map_err(|_| invalid("subject"))?,
        _ => return Err(invalid("subject")),
    };
    fields.insert("subject", subject_value(&subject));
    keys(&v["rendered_bytes"], &["length", "sha256"])?;
    let length = v["rendered_bytes"]["length"]
        .as_u64()
        .ok_or_else(|| invalid("length"))?;
    let sha = digest("sha256", text(&v["rendered_bytes"], "sha256")?)?;
    fields.insert(
        "rendered_bytes",
        CanonicalValue::Object(BTreeMap::from([
            ("length", CanonicalValue::Unsigned(length)),
            ("sha256", string(sha.as_str())),
        ])),
    );
    let b = &v["source_binding"];
    keys(
        b,
        &[
            "evidence_fingerprint",
            "source_contract_id",
            "source_contract_version",
            "source_refs",
        ],
    )?;
    let contract = SourceContractId::try_new(text(b, "source_contract_id")?.into())
        .map_err(|_| invalid("source_contract_id"))?;
    let version = SourceContractVersion::try_new(text(b, "source_contract_version")?.into())
        .map_err(|_| invalid("source_contract_version"))?;
    let evidence = digest("evidence_fingerprint", text(b, "evidence_fingerprint")?)?;
    let mut seen_refs = std::collections::BTreeSet::new();
    let refs = b["source_refs"]
        .as_array()
        .ok_or_else(|| invalid("source_refs"))?
        .iter()
        .map(|r| {
            keys(
                r,
                &[
                    "content_sha256",
                    "external_id",
                    "provider",
                    "source_contract_id",
                    "source_ref_id",
                ],
            )?;
            let id = SourceRefId::try_new(text(r, "source_ref_id")?.into())
                .map_err(|_| invalid("source_ref_id"))?;
            if !seen_refs.insert(id.clone()) {
                return Err(invalid("source_ref_id"));
            }
            let provider = SourceProvider::try_new(text(r, "provider")?.into())
                .map_err(|_| invalid("provider"))?;
            let external = ExternalId::try_new(text(r, "external_id")?.into())
                .map_err(|_| invalid("external_id"))?;
            let c = SourceContractId::try_new(text(r, "source_contract_id")?.into())
                .map_err(|_| invalid("source_contract_id"))?;
            same(c == contract, "source_ref_contract")?;
            let content = digest("content_sha256", text(r, "content_sha256")?)?;
            Ok(CanonicalValue::Object(BTreeMap::from([
                ("source_ref_id", string(id.as_str())),
                ("provider", string(provider.as_str())),
                ("external_id", string(external.as_str())),
                ("source_contract_id", string(c.as_str())),
                ("content_sha256", string(content.as_str())),
            ])))
        })
        .collect::<Result<Vec<_>>>()?;
    fields.insert(
        "source_binding",
        CanonicalValue::Object(BTreeMap::from([
            ("source_contract_id", string(contract.as_str())),
            ("source_contract_version", string(version.as_str())),
            ("evidence_fingerprint", string(evidence.as_str())),
            ("source_refs", CanonicalValue::Array(refs)),
        ])),
    );
    if canonical_preimage("PreparedPush/v1", &fields) != bytes {
        return Err(N02BindingError::InvalidCanonicalEncoding);
    }
    for (ok, field) in [
        (
            text(&v, "intent_id")? == ready.intent_id.as_str(),
            "intent_id",
        ),
        (
            text(&v, "decision_id")? == ready.decision_id.as_str(),
            "decision_id",
        ),
        (
            text(&v, "occurrence")? == ready.occurrence.as_str(),
            "occurrence",
        ),
        (unit == ready.unit_id, "unit_id"),
        (subject == ready.subject, "subject"),
        (
            contract.as_str() == row.source_contract_id,
            "source_contract_id",
        ),
        (
            evidence == ready.source_evidence_fingerprint,
            "evidence_fingerprint",
        ),
        (
            sha == ready.rendered_sha256 && text(&v, "rendered_sha256")? == sha.as_str(),
            "rendered_sha256",
        ),
        (
            length == row.rendered_bytes.as_ref().map_or(0, |b| b.len()) as u64,
            "rendered_len",
        ),
    ] {
        if !ok {
            return Err(N02BindingError::PreparedIdentityMismatch { field });
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "n02_binding_tests.rs"]
mod tests;
