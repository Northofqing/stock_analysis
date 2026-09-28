use super::*;
use crate::data_gateway::{BatchEvidence, GlobalNewsProvider, GlobalNewsRecord};
use crate::event::{NewsFlashAuditSource, NewsFlashWindow};
use crate::market_domain::SourceEvidence;
use crate::monitor::push_job::{
    n02_prepared_push_from_facts_fixture, n02_source_capture_fixture, AudienceId, BusinessDate,
    CompletionOwnerId, Namespace, OccurrenceFamily, OccurrenceIdentityMaterial, OccurrenceKey,
    RunId, SubjectId, UnitId,
};
use crate::news::aggregator::raw_v2::{
    test_project_news_flash_record, NewsFlashProjectedEvent, NewsFlashProjectionTestCapability,
};
use crate::push_foundation::{BusinessIntentStore, FoundationSchemaMigration};
use chrono::{DateTime, NaiveDate, Timelike, Utc};
use serde_json::Value;

const RENDERED: &[u8] = b"TEST_CODE N02 rendered aggregate";

fn projected(
    item_id: &str,
    summary: &str,
    batch_id: &str,
    observed_nanos: u32,
) -> NewsFlashProjectedEvent {
    let provider = GlobalNewsProvider::Eastmoney;
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
    let projection = test_project_news_flash_record(
        &NewsFlashProjectionTestCapability::bind().unwrap(),
        provider,
        record,
        batch,
    );
    assert_eq!(projection.events().len(), 1);
    let (mut events, _) = projection.into_parts();
    events.pop().unwrap()
}

fn identity() -> InitialIntentIdentity {
    InitialIntentIdentity::new(
        Namespace::test(RunId::try_new("TEST_CODE_N02_SOURCE_RUN".into()).unwrap()),
        UnitId::try_new("MU-news-flash-aggregate".into()).unwrap(),
        OccurrenceIdentityMaterial::new(
            BusinessDate::parse("2026-09-28").unwrap(),
            OccurrenceFamily::try_new("news-flash-window".into()).unwrap(),
            OccurrenceKey::try_new("09:30".into()).unwrap(),
        ),
        CompletionOwnerId::try_new("TEST_CODE_N02_OWNER".into()).unwrap(),
        SourceContractId::try_new(N02_SOURCE_CONTRACT_ID.into()).unwrap(),
        SubjectId::Global,
        AudienceId::try_new("TEST_CODE_N02_AUDIENCE".into()).unwrap(),
    )
}

struct Case {
    v2: InitialIntentDraft,
    v1: InitialIntentDraft,
    reservation: N02ReservationBindingV1,
}

fn case(specs: &[(&str, &str, &str, u32)]) -> Case {
    let selected: Vec<_> = specs
        .iter()
        .map(|(item, summary, batch, nanos)| projected(item, summary, batch, *nanos))
        .collect();
    let sources = selected
        .iter()
        .map(|item| {
            let source = item.source();
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
    let binding = crate::push_foundation::intent_store::n02_test_support::reservation(
        NaiveDate::from_ymd_opt(2026, 9, 28).unwrap(),
        NewsFlashWindow::H0930,
        RENDERED,
        sources,
    );
    let witness = N02SourceChainV1::try_capture(binding.clone(), &selected, RENDERED).unwrap();
    let mut capture = n02_source_capture_fixture().unwrap();
    let mut selected_proof = None;
    let facts = capture
        .capture_once(|_| {
            let (captured, proof) = witness.into_captured_facts_with_proof().unwrap();
            selected_proof = Some(proof);
            Ok(captured)
        })
        .unwrap();
    let proof = selected_proof.unwrap();
    let identity = identity();
    let prepared = n02_prepared_push_from_facts_fixture(
        identity.namespace.clone(),
        identity.unit_id.clone(),
        identity.occurrence.clone(),
        identity.completion_owner.clone(),
        identity.source_contract_id.clone(),
        identity.subject.clone(),
        identity.audience.clone(),
        RENDERED.to_vec(),
        &facts,
    );
    let template = raw_digest(b"TEST_CODE_N02_TEMPLATE");
    let contract = raw_digest(b"TEST_CODE_N02_SOURCE_CONTRACT");
    let created = UtcMicros::try_new(1_801_000_000_000_000).unwrap();
    let v1 = InitialIntentDraft::ready_n02(
        identity.clone(),
        &prepared,
        &binding,
        template.clone(),
        contract.clone(),
        created,
    )
    .unwrap();
    let v2 = InitialIntentDraft::ready_n02_source_v2_test(
        identity, &prepared, &facts, &proof, template, contract, created,
    )
    .unwrap();
    Case {
        v2,
        v1,
        reservation: binding,
    }
}

fn database() -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("n02-source.sqlite3");
    FoundationSchemaMigration::bundled()
        .unwrap()
        .apply_to(&path)
        .unwrap();
    (root, path)
}

fn saved(case: &Case) -> (tempfile::TempDir, std::path::PathBuf, IntentSnapshot) {
    let (root, path) = database();
    BusinessIntentStore::open(&path)
        .unwrap()
        .record_initial(&case.v2)
        .unwrap();
    let row = BusinessIntentStore::open(&path)
        .unwrap()
        .inspect(case.v2.intent_id())
        .unwrap()
        .unwrap();
    (root, path, row)
}

fn replace(row: &mut IntentSnapshot, bytes: Vec<u8>) {
    row.payload_sha256 = Some(raw_digest(&bytes));
    row.prepared_push_bytes = Some(bytes);
}

fn mutate(row: &mut IntentSnapshot, action: impl FnOnce(&mut Value)) {
    let original = row.prepared_push_bytes().unwrap();
    let mut value = parse_domain(original, DOMAIN_V2).unwrap();
    let mut round_trip = format!("{DOMAIN_V2}\0").into_bytes();
    round_trip.extend(serde_json::to_vec(&value).unwrap());
    assert_eq!(round_trip, original);
    action(&mut value);
    let mut bytes = format!("{DOMAIN_V2}\0").into_bytes();
    bytes.extend(serde_json::to_vec(&value).unwrap());
    replace(row, bytes);
}

#[test]
fn n02_v2_persists_and_reopens_exact_source_proof_in_test_namespace() {
    let case = case(&[(
        "TEST_CODE_ITEM",
        "summary A",
        "TEST_CODE_BATCH",
        987_654_321,
    )]);
    let (_root, _path, row) = saved(&case);
    assert!(row
        .prepared_push_bytes()
        .unwrap()
        .starts_with(b"N02PreparedPush/v2\0"));
    assert_eq!(
        row.attested_n02_binding().unwrap().reservation,
        case.reservation
    );
    assert_eq!(row.namespace(), "Test:TEST_CODE_N02_SOURCE_RUN");
    assert_eq!(row.rendered_bytes(), Some(RENDERED));
}

#[test]
fn n02_v2_content_only_changes_proof_and_conflicts_with_same_intent() {
    let first = case(&[(
        "TEST_CODE_ITEM",
        "summary A",
        "TEST_CODE_BATCH",
        987_654_321,
    )]);
    let changed = case(&[(
        "TEST_CODE_ITEM",
        "summary B",
        "TEST_CODE_BATCH",
        987_654_321,
    )]);
    assert_eq!(first.reservation, changed.reservation);
    assert_eq!(first.v2.intent_id(), changed.v2.intent_id());
    assert_ne!(first.v2.prepared_push_bytes, changed.v2.prepared_push_bytes);
    let (_root, path, row) = saved(&first);
    assert!(matches!(
        BusinessIntentStore::open(&path)
            .unwrap()
            .record_initial(&changed.v2),
        Err(IntentStoreError::ImmutableConflict { .. })
    ));
    let mut coherent_payload_swap = row.clone();
    replace(
        &mut coherent_payload_swap,
        changed.v2.prepared_push_bytes.as_deref().unwrap().to_vec(),
    );
    assert!(coherent_payload_swap.attested_n02_binding().is_err());
    let mut tampered = row;
    let changed_root = parse_domain(
        changed.v2.prepared_push_bytes.as_deref().unwrap(),
        DOMAIN_V2,
    )
    .unwrap();
    mutate(&mut tampered, |root| {
        root["admitted_records_hex"] = changed_root["admitted_records_hex"].clone()
    });
    assert!(tampered.attested_n02_binding().is_err());
}

#[test]
fn n02_v2_rejects_v1_and_opaque_rows_for_new_contract() {
    let case = case(&[(
        "TEST_CODE_ITEM",
        "summary A",
        "TEST_CODE_BATCH",
        987_654_321,
    )]);
    let (root, path) = database();
    let mut store = BusinessIntentStore::open(&path).unwrap();
    store.record_initial(&case.v1).unwrap();
    drop(store);
    let row = BusinessIntentStore::open(&path)
        .unwrap()
        .inspect(case.v1.intent_id())
        .unwrap()
        .unwrap();
    assert!(row.attested_ready_binding().is_ok());
    assert_eq!(
        row.attested_n02_binding().unwrap_err(),
        N02BindingError::UnsupportedPreparedFormat
    );
    assert!(matches!(
        BusinessIntentStore::open(&path)
            .unwrap()
            .record_initial(&case.v2),
        Err(IntentStoreError::ImmutableConflict { .. })
    ));
    let opaque = InitialIntentDraft::ready_for_recovery_test(
        identity(),
        b"TEST_CODE_OLD_OPAQUE_N02".to_vec(),
        RENDERED.to_vec(),
        raw_digest(b"TEST_CODE_N02_TEMPLATE"),
        raw_digest(b"TEST_CODE_N02_SOURCE_CONTRACT"),
        UtcMicros::try_new(1_801_000_000_000_000).unwrap(),
    )
    .unwrap();
    let (_opaque_root, opaque_path) = database();
    let mut opaque_store = BusinessIntentStore::open(&opaque_path).unwrap();
    let opaque_row = opaque_store
        .record_initial(&opaque)
        .unwrap()
        .snapshot()
        .clone();
    assert!(opaque_row.attested_ready_binding().is_ok());
    assert_eq!(
        opaque_row.attested_n02_binding().unwrap_err(),
        N02BindingError::UnsupportedPreparedFormat
    );
    drop(root);
}

#[test]
fn n02_v2_rejects_mutated_source_order_facts_fingerprint_and_format() {
    let case = case(&[
        ("TEST_CODE_A", "summary A", "TEST_CODE_BATCH_A", 987_654_321),
        ("TEST_CODE_B", "summary B", "TEST_CODE_BATCH_B", 987_654_322),
    ]);
    let (_root, _path, original) = saved(&case);
    let reversed = self::case(&[
        ("TEST_CODE_B", "summary B", "TEST_CODE_BATCH_B", 987_654_322),
        ("TEST_CODE_A", "summary A", "TEST_CODE_BATCH_A", 987_654_321),
    ]);
    let mut coherent_order_swap = original.clone();
    replace(
        &mut coherent_order_swap,
        reversed.v2.prepared_push_bytes.as_deref().unwrap().to_vec(),
    );
    assert!(coherent_order_swap.attested_n02_binding().is_err());
    for field in [
        "order",
        "record_content",
        "record_schema",
        "record_batch",
        "record_item",
        "record_time",
        "record_provider",
        "source_ref",
        "source_time",
        "facts_sha",
        "facts_len",
        "fingerprint",
        "prepared_facts",
        "wrapper_extra",
        "render_raw",
        "nested_extra",
    ] {
        let mut row = original.clone();
        mutate(&mut row, |root| match field {
            "order" => root["admitted_records_hex"]
                .as_array_mut()
                .unwrap()
                .reverse(),
            "record_content" | "record_schema" => {
                let mut record: Value = serde_json::from_slice(
                    &unhex(
                        root["admitted_records_hex"][0].as_str().unwrap(),
                        MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES,
                        "test",
                    )
                    .unwrap(),
                )
                .unwrap();
                if field == "record_schema" {
                    record["schema"] = "NewsFlashAdmittedRecord/v99".into();
                } else {
                    record["record"]["summary"] = "TEST_CODE tampered summary".into();
                }
                root["admitted_records_hex"][0] =
                    Value::from(hex(&serde_json::to_vec(&record).unwrap()));
            }
            "record_batch" | "record_item" | "record_time" | "record_provider" => {
                let mut record: Value = serde_json::from_slice(
                    &unhex(
                        root["admitted_records_hex"][0].as_str().unwrap(),
                        MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES,
                        "test",
                    )
                    .unwrap(),
                )
                .unwrap();
                match field {
                    "record_batch" => {
                        record["source_identity"]["batch_id"] = "TEST_CODE_OTHER".into()
                    }
                    "record_item" => record["record"]["item_id"] = "TEST_CODE_OTHER".into(),
                    "record_time" => {
                        record["source_identity"]["observed_at"]["nanoseconds"] = 8u64.into()
                    }
                    _ => record["registration"]["provider"] = "Jin10".into(),
                }
                root["admitted_records_hex"][0] =
                    Value::from(hex(&serde_json::to_vec(&record).unwrap()));
            }
            "source_ref" => root["source_refs"][0]["content_sha256"] = Value::from("a".repeat(64)),
            "source_time" => root["source_times"][0]["value"] = 1u64.into(),
            "facts_sha" => root["canonical_facts"]["sha256"] = Value::from("a".repeat(64)),
            "facts_len" => root["canonical_facts"]["length"] = 1u64.into(),
            "fingerprint" | "prepared_facts" | "nested_extra" => {
                let mut prepared: Value = serde_json::from_slice(
                    &unhex(
                        root["prepared_push_hex"].as_str().unwrap(),
                        MAX_GENERIC_PREPARED_BYTES,
                        "test",
                    )
                    .unwrap()["PreparedPush/v1\0".len()..],
                )
                .unwrap();
                match field {
                    "fingerprint" => {
                        prepared["source_binding"]["evidence_fingerprint"] =
                            Value::from("a".repeat(64))
                    }
                    "prepared_facts" => {
                        prepared["prepared_facts_sha256"] = Value::from("a".repeat(64))
                    }
                    _ => prepared["extra"] = Value::Null,
                }
                let mut bytes = b"PreparedPush/v1\0".to_vec();
                bytes.extend(serde_json::to_vec(&prepared).unwrap());
                root["prepared_push_hex"] = Value::from(hex(&bytes));
            }
            "wrapper_extra" => root["extra"] = Value::Null,
            "render_raw" => root["rendered_raw_sha256"] = Value::from("a".repeat(64)),
            _ => unreachable!(),
        });
        assert!(row.attested_n02_binding().is_err(), "field {field}");
    }
    let mut unknown_version = original;
    let mut bytes = unknown_version.prepared_push_bytes().unwrap().to_vec();
    bytes[DOMAIN_V2.len() - 1] = b'3';
    replace(&mut unknown_version, bytes);
    assert_eq!(
        unknown_version.attested_n02_binding().unwrap_err(),
        N02BindingError::UnsupportedPreparedFormat
    );
}
