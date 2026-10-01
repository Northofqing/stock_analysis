//! TEST_CODE fixtures only: no production producer admission or provider access.
use super::*;
use crate::event::envelope::news_flash_evidence_sha256;
use crate::event::news_flash_identity::{
    news_flash_render_sha256, news_flash_reservation_sha256, NewsFlashReservationIdentityFields,
};
use crate::event::{NewsFlashAuditSource, NewsFlashWindow};

pub(crate) fn reservation(
    date: chrono::NaiveDate,
    window: NewsFlashWindow,
    rendered: &[u8],
    sources: Vec<NewsFlashAuditSource>,
) -> N02ReservationBindingV1 {
    let evidence = news_flash_evidence_sha256(&sources);
    let render = news_flash_render_sha256(rendered);
    let key = window.decision_key();
    let sha = news_flash_reservation_sha256(&NewsFlashReservationIdentityFields {
        push_kind: "news_flash_aggregated_v1",
        business_date: date,
        decision_key: &key,
        event_id: None,
        window: Some(window.label()),
        evidence_sha256: &evidence,
        render_sha256: &render,
    });
    N02ReservationBindingV1::try_from_reservation_material(
        N02ReservationMaterial {
            business_date: date,
            window: window.label().into(),
            push_kind: "news_flash_aggregated_v1".into(),
            decision_key: key,
            event_id: None,
            reservation_sha256: sha,
            sources,
            evidence_sha256: evidence,
            news_flash_render_sha256: render,
            rendered_len: rendered.len() as u64,
        },
        rendered,
    )
    .unwrap()
}

pub(crate) fn ready(
    identity: InitialIntentIdentity,
    rendered: Vec<u8>,
    binding: &N02ReservationBindingV1,
    template: Sha256Digest,
    contract: Sha256Digest,
    at: UtcMicros,
) -> InitialIntentDraft {
    let prepared = crate::monitor::push_job::n02_prepared_push_fixture(
        identity.namespace.clone(),
        identity.unit_id.clone(),
        identity.occurrence.clone(),
        identity.completion_owner.clone(),
        identity.source_contract_id.clone(),
        identity.subject.clone(),
        identity.audience.clone(),
        rendered,
    );
    InitialIntentDraft::ready_n02(identity, &prepared, binding, template, contract, at).unwrap()
}

pub(crate) fn sources() -> Vec<NewsFlashAuditSource> {
    let at = chrono::DateTime::parse_from_rfc3339("2026-08-18T01:20:00+08:00").unwrap();
    vec![NewsFlashAuditSource {
        event_id: "TEST_CODE_N02_EVENT".into(),
        provider: "TEST_CODE_N02_PROVIDER".into(),
        source: "TEST_CODE_N02_SOURCE".into(),
        published_at: at,
        observed_at: at,
        batch_id: "TEST_CODE_N02_BATCH".into(),
    }]
}

pub(crate) fn ready_default(
    identity: InitialIntentIdentity,
    rendered: Vec<u8>,
    template: Sha256Digest,
    contract: Sha256Digest,
    at: UtcMicros,
) -> InitialIntentDraft {
    let binding = reservation(
        chrono::NaiveDate::parse_from_str(identity.occurrence.business_date().as_str(), "%Y-%m-%d")
            .unwrap(),
        NewsFlashWindow::parse(identity.occurrence.occurrence_key().as_str()).unwrap(),
        &rendered,
        sources(),
    );
    ready(identity, rendered, &binding, template, contract, at)
}
