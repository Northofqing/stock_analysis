use super::*;
use crate::data_gateway::{BatchEvidence, GlobalNewsProvider, GlobalNewsRecord};
use crate::event::news_flash_identity::{
    news_flash_render_sha256, news_flash_reservation_sha256, NewsFlashReservationIdentityFields,
};
use crate::event::{envelope::news_flash_evidence_sha256, NewsFlashAuditSource};
use crate::market_domain::SourceEvidence;
use crate::monitor::push_job::context::n02_source_capture_fixture;
use crate::monitor::push_job::{CaptureStateView, Namespace};
use crate::news::aggregator::raw_v2::{
    ordered_news_flash_evidence_sha256, test_project_news_flash_record,
    NewsFlashProjectionTestCapability,
};
use chrono::{DateTime, NaiveDate, Timelike, Utc};

const RENDERED: &[u8] = b"TEST_CODE N02 rendered aggregate";

fn projected(
    item_id: &str,
    summary: &str,
    batch_id: &str,
    observed_nanos: u32,
) -> NewsFlashProjectedEvent {
    let published_at = DateTime::parse_from_rfc3339("2026-09-28T09:00:00+08:00")
        .unwrap()
        .with_timezone(&Utc);
    let observed_at = DateTime::parse_from_rfc3339("2026-09-28T09:01:02+08:00")
        .unwrap()
        .with_timezone(&Utc)
        .with_nanosecond(observed_nanos)
        .unwrap();
    let observed_wire = observed_at.to_rfc3339();
    let source_at = "2026-09-28 09:00".to_owned();
    let provider = GlobalNewsProvider::Eastmoney;
    let batch = BatchEvidence {
        provider: provider.provider_id(),
        source: provider.source().to_owned(),
        source_at: Some(source_at.clone()),
        observed_at: observed_wire.clone(),
        batch_id: batch_id.to_owned(),
    };
    let record = GlobalNewsRecord {
        item_id: item_id.to_owned(),
        title: "TEST_CODE title".to_owned(),
        summary: Some(summary.to_owned()),
        content: None,
        publisher: "TEST_CODE publisher".to_owned(),
        canonical_url: format!("https://example.com/{item_id}"),
        published_at,
        observed_at,
        instruments: Vec::new(),
        topics: Vec::new(),
        language: "zh-CN".to_owned(),
        evidence: SourceEvidence::new(provider.provider_id(), observed_wire, batch_id)
            .unwrap()
            .with_source_at(source_at)
            .unwrap(),
    };
    let capability = NewsFlashProjectionTestCapability::bind().unwrap();
    let projection = test_project_news_flash_record(&capability, provider, record, batch);
    assert_eq!(projection.available_feed_count(), 1);
    assert_eq!(projection.verified_empty_feed_count(), 0);
    let (mut events, _) = projection.into_parts();
    assert_eq!(events.len(), 1);
    events.pop().unwrap()
}

fn binding(selected: &[NewsFlashProjectedEvent]) -> N02ReservationBindingV1 {
    let sources: Vec<_> = selected
        .iter()
        .map(|projected| {
            let source = projected.source();
            NewsFlashAuditSource {
                event_id: source.event_id().to_owned(),
                provider: source.provider().to_owned(),
                source: source.source().to_owned(),
                published_at: source.published_at().fixed_offset(),
                observed_at: source.observed_at().fixed_offset(),
                batch_id: source.batch_id().to_owned(),
            }
        })
        .collect();
    let business_date = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
    let evidence_sha256 = news_flash_evidence_sha256(&sources);
    assert_eq!(
        evidence_sha256,
        ordered_news_flash_evidence_sha256(selected)
    );
    let render_sha256 = news_flash_render_sha256(RENDERED);
    let reservation_sha256 = news_flash_reservation_sha256(&NewsFlashReservationIdentityFields {
        push_kind: "news_flash_aggregated_v1",
        business_date,
        decision_key: "window:09:30",
        event_id: None,
        window: Some("09:30"),
        evidence_sha256: &evidence_sha256,
        render_sha256: &render_sha256,
    });
    N02ReservationBindingV1::try_from_reservation_material(
        crate::push_foundation::N02ReservationMaterial {
            business_date,
            window: "09:30".to_owned(),
            push_kind: "news_flash_aggregated_v1".to_owned(),
            decision_key: "window:09:30".to_owned(),
            event_id: None,
            reservation_sha256,
            sources,
            evidence_sha256,
            news_flash_render_sha256: render_sha256,
            rendered_len: RENDERED.len() as u64,
        },
        RENDERED,
    )
    .unwrap()
}

#[test]
fn n02_source_capture_uses_selected_record_and_normal_test_preparation() {
    let selected = vec![projected(
        "TEST_CODE_ITEM",
        "TEST_CODE summary",
        "TEST_CODE_BATCH",
        987_654_321,
    )];
    let witness = N02SourceChainV1::try_capture(binding(&selected), &selected, RENDERED).unwrap();
    let admitted = selected[0].record_evidence().unwrap();
    assert_eq!(
        witness.records()[0].canonical_bytes(),
        admitted.canonical_bytes()
    );
    let source_ref = &witness.source_refs()[0];
    // Independently calculated from the eight length-delimited fields using
    // Python hashlib; this fixes the versioned occurrence preimage contract.
    assert_eq!(
        source_ref.source_ref_id().as_str(),
        "df61cfe862b9271d9d4d85bb2f9e6f08312a38a6b2542b2a9b6bc1cb172b9114"
    );
    assert_eq!(source_ref.provider().as_str(), "Eastmoney");
    assert_eq!(source_ref.external_id().as_str(), "TEST_CODE_ITEM");
    assert_eq!(
        source_ref.source_contract_id().as_str(),
        N02_SOURCE_CONTRACT_ID
    );
    assert_eq!(
        source_ref.content_sha256().as_str(),
        admitted.content_sha256()
    );
    assert_eq!(
        witness.source_times()[0].kind(),
        super::super::SourceTimeKind::ObservedAt
    );
    assert_eq!(
        witness.source_times()[0].value().unwrap().get(),
        selected[0].source().observed_at().timestamp_micros()
    );
    assert!(witness
        .canonical_facts()
        .as_bytes()
        .starts_with(b"N02SelectedFacts/v1\0{"));
    let facts: serde_json::Value = serde_json::from_slice(
        &witness.canonical_facts().as_bytes()["N02SelectedFacts/v1\0".len()..],
    )
    .unwrap();
    assert_eq!(
        facts["entries"][0]["source_identity"]["observed_at"]["nanoseconds"],
        987_654_321
    );
    assert_eq!(
        facts["entries"][0]["selected_semantics"]["title"],
        "TEST_CODE title"
    );
    assert_eq!(facts["projection_policy"], SOURCE_ONLY_POLICY);
    assert_eq!(facts["rendered_len"], RENDERED.len());
    assert_eq!(
        facts["rendered_raw_sha256"],
        witness.rendered_raw_sha256().as_str()
    );

    let expected_refs = witness.source_refs().to_vec();
    let expected_times = witness.source_times().to_vec();
    let expected_facts_sha = witness.canonical_facts().sha256().clone();
    let mut capture = n02_source_capture_fixture().unwrap();
    assert!(matches!(
        capture.context().namespace(),
        Namespace::Test { .. }
    ));
    let snapshot = capture
        .capture_once(|_| Ok(witness.into_captured_facts().unwrap()))
        .unwrap();
    let prepared = snapshot.facts();
    assert_eq!(prepared.source_refs(), expected_refs);
    assert_eq!(prepared.provider_observed_at(), expected_times);
    assert_eq!(prepared.facts_sha256(), &expected_facts_sha);
    assert_eq!(
        prepared.source_contract_id().as_str(),
        N02_SOURCE_CONTRACT_ID
    );
    assert_eq!(
        prepared.source_contract_version().as_str(),
        N02_SOURCE_CONTRACT_VERSION
    );
    assert!(!prepared.verified_empty());
    assert!(prepared.model_output_refs().is_empty());
    assert_eq!(capture.state(), CaptureStateView::Sealed);
}

#[test]
fn n02_source_content_only_changes_ref_and_facts_but_not_legacy_identity() {
    let a = vec![projected(
        "TEST_CODE_ITEM",
        "TEST_CODE summary A",
        "TEST_CODE_BATCH",
        987_654_321,
    )];
    let b = vec![projected(
        "TEST_CODE_ITEM",
        "TEST_CODE summary B",
        "TEST_CODE_BATCH",
        987_654_321,
    )];
    let a_binding = binding(&a);
    let b_binding = binding(&b);
    assert_eq!(a_binding, b_binding);
    assert_eq!(a[0].event().full_title, b[0].event().full_title);
    let a_witness = N02SourceChainV1::try_capture(a_binding, &a, RENDERED).unwrap();
    let b_witness = N02SourceChainV1::try_capture(b_binding, &b, RENDERED).unwrap();
    assert_ne!(
        a_witness.source_refs()[0].content_sha256(),
        b_witness.source_refs()[0].content_sha256()
    );
    assert_ne!(
        a_witness.source_refs()[0].source_ref_id(),
        b_witness.source_refs()[0].source_ref_id()
    );
    assert_ne!(
        a_witness.canonical_facts().sha256(),
        b_witness.canonical_facts().sha256()
    );

    let changed_batch = vec![projected(
        "TEST_CODE_ITEM",
        "TEST_CODE summary A",
        "TEST_CODE_BATCH_2",
        987_654_321,
    )];
    let changed_time = vec![projected(
        "TEST_CODE_ITEM",
        "TEST_CODE summary A",
        "TEST_CODE_BATCH",
        987_654_322,
    )];
    for changed in [&changed_batch, &changed_time] {
        let other = N02SourceChainV1::try_capture(binding(changed), changed, RENDERED).unwrap();
        assert_ne!(
            a_witness.source_refs()[0].source_ref_id(),
            other.source_refs()[0].source_ref_id()
        );
    }
}

#[test]
fn n02_source_rejects_changed_selected_order_and_duplicate_event() {
    let a = projected(
        "TEST_CODE_A",
        "TEST_CODE summary A",
        "TEST_CODE_BATCH_A",
        987_654_321,
    );
    let b = projected(
        "TEST_CODE_B",
        "TEST_CODE summary B",
        "TEST_CODE_BATCH_B",
        987_654_322,
    );
    let selected = vec![a.clone(), b];
    let original = binding(&selected);
    let mut swapped = selected.clone();
    swapped.reverse();
    assert_eq!(
        N02SourceChainV1::try_capture(original, &swapped, RENDERED).unwrap_err(),
        N02SourceError::EvidenceDigestMismatch
    );
    let duplicate = vec![a.clone(), a];
    assert_eq!(
        N02SourceChainV1::try_capture(binding(&duplicate), &duplicate, RENDERED).unwrap_err(),
        N02SourceError::DuplicateEvent { index: 1 }
    );
}
