use super::super::{BusinessIntentStore, InitialIntentOutcome};
use super::*;

const RENDER: &[u8] = b"TEST_CODE rendered\n";
fn identity() -> InitialIntentIdentity {
    InitialIntentIdentity::new(
        Namespace::test(RunId::try_new("TEST_CODE".into()).unwrap()),
        UnitId::try_new("MU-news-flash-aggregate".into()).unwrap(),
        OccurrenceIdentityMaterial::new(
            BusinessDate::parse("2026-09-28").unwrap(),
            OccurrenceFamily::try_new("news-flash-window".into()).unwrap(),
            OccurrenceKey::try_new("09:30".into()).unwrap(),
        ),
        CompletionOwnerId::try_new("TEST_CODE-owner".into()).unwrap(),
        SourceContractId::try_new("TEST_CODE-source".into()).unwrap(),
        SubjectId::Global,
        AudienceId::try_new("TEST_CODE-audience".into()).unwrap(),
    )
}
fn prepared(i: &InitialIntentIdentity) -> PreparedPush {
    crate::monitor::push_job::n02_prepared_push_fixture(
        i.namespace.clone(),
        i.unit_id.clone(),
        i.occurrence.clone(),
        i.completion_owner.clone(),
        i.source_contract_id.clone(),
        i.subject.clone(),
        i.audience.clone(),
        RENDER.to_vec(),
    )
}
fn material() -> N02ReservationMaterial {
    let sources = ["A", "B"]
        .iter()
        .map(|name| NewsFlashAuditSource {
            event_id: format!("TEST_CODE-{name}"),
            provider: "TEST_CODE".into(),
            source: "TEST_CODE".into(),
            batch_id: format!("batch-{name}"),
            published_at: chrono::DateTime::parse_from_rfc3339("2026-09-28T09:00:00+08:00")
                .unwrap(),
            observed_at: chrono::DateTime::parse_from_rfc3339("2026-09-28T09:01:00+08:00").unwrap(),
        })
        .collect::<Vec<_>>();
    let evidence_sha256 = news_flash_evidence_sha256(&sources);
    let news_flash_render_sha256 = news_flash_render_sha256(RENDER);
    let business_date = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
    let reservation_sha256 = news_flash_reservation_sha256(&NewsFlashReservationIdentityFields {
        push_kind: "news_flash_aggregated_v1",
        business_date,
        decision_key: "window:09:30",
        event_id: None,
        window: Some("09:30"),
        evidence_sha256: &evidence_sha256,
        render_sha256: &news_flash_render_sha256,
    });
    N02ReservationMaterial {
        business_date,
        window: "09:30".into(),
        push_kind: "news_flash_aggregated_v1".into(),
        decision_key: "window:09:30".into(),
        event_id: None,
        reservation_sha256,
        sources,
        evidence_sha256,
        news_flash_render_sha256,
        rendered_len: RENDER.len() as u64,
    }
}
fn binding() -> N02ReservationBindingV1 {
    N02ReservationBindingV1::try_from_reservation_material(material(), RENDER).unwrap()
}
fn draft() -> InitialIntentDraft {
    InitialIntentDraft::ready_n02(
        identity(),
        &prepared(&identity()),
        &binding(),
        raw_digest(b"template"),
        raw_digest(b"contract"),
        UtcMicros::try_new(1).unwrap(),
    )
    .unwrap()
}
fn database() -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("business.sqlite3");
    FoundationSchemaMigration::bundled()
        .unwrap()
        .apply_to(&path)
        .unwrap();
    (root, path)
}
fn snapshot() -> IntentSnapshot {
    let (_root, path) = database();
    let mut store = BusinessIntentStore::open(&path).unwrap();
    match store.record_initial(&draft()).unwrap() {
        InitialIntentOutcome::Inserted(row) => row,
        other => panic!("{other:?}"),
    }
}
fn replace(row: &mut IntentSnapshot, bytes: Vec<u8>) {
    row.payload_sha256 = Some(raw_digest(&bytes));
    row.prepared_push_bytes = Some(bytes);
}
fn mutate_nested(row: &mut IntentSnapshot, f: impl FnOnce(&mut Value)) {
    let outer = parse_domain(row.prepared_push_bytes().unwrap(), DOMAIN).unwrap();
    let nested = outer["prepared_push_bytes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap() as u8)
        .collect::<Vec<_>>();
    let mut value = parse_domain(&nested, "PreparedPush/v1").unwrap();
    f(&mut value);
    let mut bytes = b"PreparedPush/v1\0".to_vec();
    bytes.extend(serde_json::to_vec(&value).unwrap());
    replace(row, wrapper(&bytes, &binding()));
}
#[test]
fn n02_wrapper_round_trips_and_attests_after_store_reopen() {
    let (_root, path) = database();
    let d = draft();
    {
        BusinessIntentStore::open(&path)
            .unwrap()
            .record_initial(&d)
            .unwrap();
    }
    let row = BusinessIntentStore::open(&path)
        .unwrap()
        .inspect(d.intent_id())
        .unwrap()
        .unwrap();
    let attested = row.attested_n02_binding().unwrap();
    assert_eq!(attested.reservation, binding());
    assert_eq!(attested.reservation.window(), NewsFlashWindow::H0930);
    assert_eq!(
        attested.ready.source_evidence_fingerprint,
        prepared(&identity())
            .source_binding()
            .evidence_fingerprint()
            .clone()
    );
    assert_eq!(row.prepared_push_bytes, d.prepared_push_bytes);
    assert_eq!(row.rendered_bytes(), Some(RENDER));
    assert_ne!(
        attested.ready.rendered_sha256.as_str(),
        binding().material().news_flash_render_sha256
    );
}
#[test]
fn n02_wrapper_rejects_identity_and_occurrence_mismatches() {
    for case in 0..6 {
        let mut i = identity();
        match case {
            0 => i.unit_id = UnitId::try_new("other".into()).unwrap(),
            1 => i.subject = SubjectId::entity("other".into()).unwrap(),
            2 | 3 | 4 => {
                i.occurrence = OccurrenceIdentityMaterial::new(
                    BusinessDate::parse(if case == 2 {
                        "2026-09-29"
                    } else {
                        "2026-09-28"
                    })
                    .unwrap(),
                    OccurrenceFamily::try_new(
                        if case == 3 {
                            "other"
                        } else {
                            "news-flash-window"
                        }
                        .into(),
                    )
                    .unwrap(),
                    OccurrenceKey::try_new(if case == 4 { "11:30" } else { "09:30" }.into())
                        .unwrap(),
                )
            }
            _ => i.source_contract_id = SourceContractId::try_new("other".into()).unwrap(),
        }
        let p = if case == 5 {
            prepared(&identity())
        } else {
            prepared(&i)
        };
        assert!(
            InitialIntentDraft::ready_n02(
                i,
                &p,
                &binding(),
                raw_digest(b"t"),
                raw_digest(b"c"),
                UtcMicros::try_new(1).unwrap()
            )
            .is_err(),
            "case {case}"
        );
    }
}
#[test]
fn n02_wrapper_rejects_inconsistent_legacy_reservation_material() {
    for case in 0..13 {
        let mut m = material();
        match case {
            0 => m.evidence_sha256 = "a".repeat(64),
            1 => m.sources.reverse(),
            2 => m.sources[0].provider = "".into(),
            3 => {
                m.sources[0].observed_at = m.sources[0].published_at - chrono::Duration::seconds(1)
            }
            4 => m.news_flash_render_sha256 = raw_digest(RENDER).as_str().into(),
            5 => m.reservation_sha256 = "a".repeat(64),
            6 => m.rendered_len += 1,
            7 => m.event_id = Some("N01".into()),
            8 => m.window = "10:00".into(),
            9 => m.decision_key = "window:11:30".into(),
            10 => m.push_kind = "news_flash_critical_v1".into(),
            11 => m.sources.clear(),
            _ => m.sources[0].event_id.push('\0'),
        }
        assert!(
            N02ReservationBindingV1::try_from_reservation_material(m, RENDER).is_err(),
            "case {case}"
        );
    }
    for bytes in [&b""[..], &b"\xff"[..], &b"different"[..]] {
        assert!(N02ReservationBindingV1::try_from_reservation_material(material(), bytes).is_err());
    }
}
#[test]
fn n02_wrapper_rejects_noncanonical_and_unknown_versions() {
    let original = snapshot();
    for case in 0..9 {
        let mut row = original.clone();
        let mut bytes = row.prepared_push_bytes.clone().unwrap();
        match case {
            0 => bytes[DOMAIN.len() - 1] = b'2',
            1 => bytes.insert(DOMAIN.len() + 1, b' '),
            2 => {
                bytes.pop();
                bytes.extend_from_slice(b",\"reservation\":null}");
            }
            _ => {
                let mut v = parse_domain(&bytes, DOMAIN).unwrap();
                match case {
                    3 => {
                        v["extra"] = Value::Null;
                    }
                    4 => {
                        v.as_object_mut().unwrap().remove("reservation");
                    }
                    5 => v["prepared_push_bytes"][0] = Value::from(256),
                    6 => v["reservation"]["sources"][0]["extra"] = Value::Null,
                    7 => {
                        v["reservation"]["sources"][0]["published_at"] =
                            Value::from("2026-09-28T01:00:00Z")
                    }
                    _ => v["prepared_push_bytes"] = Value::from("opaque"),
                }
                bytes = format!("{DOMAIN}\0").into_bytes();
                bytes.extend(serde_json::to_vec(&v).unwrap());
            }
        }
        replace(&mut row, bytes);
        assert!(row.attested_n02_binding().is_err(), "case {case}");
    }
    let mut row = original;
    mutate_nested(&mut row, |v| v["extra"] = Value::Null);
    assert!(row.attested_n02_binding().is_err());
}
#[test]
fn n02_wrapper_rejects_nested_prepared_row_mismatches() {
    for field in [
        "intent_id",
        "decision_id",
        "occurrence",
        "rendered_sha256",
        "run_context_sha256",
        "prepared_facts_sha256",
        "semantic_projection_sha256",
        "unit_id",
        "subject",
        "source_contract_id",
        "source_contract_version",
        "evidence_fingerprint",
        "source_ref_contract",
        "length",
        "sha256",
    ] {
        let mut row = snapshot();
        mutate_nested(&mut row, |v| match field {
            "source_contract_id" | "evidence_fingerprint" => {
                v["source_binding"][field] = Value::from("a".repeat(64))
            }
            "source_contract_version" => v["source_binding"][field] = Value::from(""),
            "source_ref_contract" => {
                v["source_binding"]["source_refs"][0]["source_contract_id"] = Value::from("other")
            }
            "length" => v["rendered_bytes"][field] = Value::from(999),
            "sha256" => v["rendered_bytes"][field] = Value::from("a".repeat(64)),
            "subject" => v[field]["kind"] = Value::from("Unknown"),
            "run_context_sha256" | "prepared_facts_sha256" | "semantic_projection_sha256" => {
                v[field] = Value::from("invalid")
            }
            _ => v[field] = Value::from("a".repeat(64)),
        });
        assert!(row.attested_n02_binding().is_err(), "field {field}");
    }
}
#[test]
fn n02_wrapper_rejects_legacy_opaque_intent_without_blocking_row_read() {
    let (_root, path) = database();
    let mut store = BusinessIntentStore::open(&path).unwrap();
    let old = InitialIntentDraft::ready_for_recovery_test(
        identity(),
        br#"{"reservation_sha256":"opaque","ordered_batches":["a"]}"#.to_vec(),
        RENDER.to_vec(),
        raw_digest(b"t"),
        raw_digest(b"c"),
        UtcMicros::try_new(1).unwrap(),
    )
    .unwrap();
    store.record_initial(&old).unwrap();
    let row = store.inspect(old.intent_id()).unwrap().unwrap();
    assert!(row.attested_ready_binding().is_ok());
    assert_eq!(
        row.attested_n02_binding().unwrap_err(),
        N02BindingError::UnsupportedPreparedFormat
    );
    assert!(matches!(
        store.record_initial(&draft()),
        Err(IntentStoreError::ImmutableConflict { .. })
    ));
    assert_eq!(
        store
            .inspect(old.intent_id())
            .unwrap()
            .unwrap()
            .prepared_push_bytes(),
        old.prepared_push_bytes.as_deref()
    );
}
#[test]
fn n02_wrapper_keeps_generic_ready_bytes() {
    let p = prepared(&identity());
    let d = InitialIntentDraft::ready(
        identity(),
        &p,
        raw_digest(b"t"),
        raw_digest(b"c"),
        UtcMicros::try_new(1).unwrap(),
    )
    .unwrap();
    assert_eq!(
        d.prepared_push_bytes.as_deref(),
        Some(p.canonical_snapshot_bytes().as_bytes())
    );
    let mut row = snapshot();
    replace(&mut row, d.prepared_push_bytes.unwrap());
    assert_eq!(
        row.attested_n02_binding().unwrap_err(),
        N02BindingError::UnsupportedPreparedFormat
    );
}

#[test]
fn n02_wrapper_rejects_noncanonical_nested_bytes_after_outer_rehash() {
    let original = prepared(&identity())
        .canonical_snapshot_bytes()
        .as_bytes()
        .to_vec();
    for case in 0..6 {
        let mut bytes = original.clone();
        match case {
            0 => bytes.insert("PreparedPush/v1".len() + 1, b' '),
            1 => bytes["PreparedPush/v".len()] = b'2',
            2 => {
                bytes.pop();
                bytes.extend_from_slice(
                    format!(",\"unit_id\":\"{}\"}}", identity().unit_id.as_str()).as_bytes(),
                );
            }
            _ => {
                let mut v = parse_domain(&bytes, "PreparedPush/v1").unwrap();
                match case {
                    3 => {
                        v.as_object_mut().unwrap().remove("run_context_sha256");
                    }
                    4 => v["source_binding"]["source_refs"][0]["unknown"] = Value::Null,
                    _ => {
                        let r = v["source_binding"]["source_refs"][0].clone();
                        v["source_binding"]["source_refs"]
                            .as_array_mut()
                            .unwrap()
                            .push(r);
                    }
                }
                bytes = b"PreparedPush/v1\0".to_vec();
                bytes.extend(serde_json::to_vec(&v).unwrap());
            }
        }
        let mut row = snapshot();
        replace(&mut row, wrapper(&bytes, &binding()));
        assert!(row.attested_ready_binding().is_ok());
        assert!(row.attested_n02_binding().is_err(), "case {case}");
    }
}

#[test]
fn n02_wrapper_rejects_other_valid_reservation_occurrence_after_outer_rehash() {
    for change_date in [false, true] {
        let mut m = material();
        if change_date {
            m.business_date = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        } else {
            m.window = "11:30".into();
            m.decision_key = "window:11:30".into();
        }
        m.reservation_sha256 = news_flash_reservation_sha256(&NewsFlashReservationIdentityFields {
            push_kind: &m.push_kind,
            business_date: m.business_date,
            decision_key: &m.decision_key,
            event_id: None,
            window: Some(&m.window),
            evidence_sha256: &m.evidence_sha256,
            render_sha256: &m.news_flash_render_sha256,
        });
        let other = N02ReservationBindingV1::try_from_reservation_material(m, RENDER).unwrap();
        let mut row = snapshot();
        replace(
            &mut row,
            wrapper(
                prepared(&identity()).canonical_snapshot_bytes().as_bytes(),
                &other,
            ),
        );
        assert!(row.attested_ready_binding().is_ok());
        assert!(matches!(
            row.attested_n02_binding(),
            Err(N02BindingError::PreparedIdentityMismatch { .. })
        ));
    }
}
