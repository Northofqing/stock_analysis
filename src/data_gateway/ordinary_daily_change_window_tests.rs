use super::*;
use crate::grpc_client::{
    client::external_query_wire_fixture::ExternalQueryWireFixture,
    external_pb::magic::market::v1::{
        AdmissionState, CanonicalPayload, Operation, QueryRequest, QueryResponse,
    },
};
use crate::market_domain::{AssetClass, Exchange};
use c::{EvidenceRef, EvidenceV1, Terminal};

pub(crate) fn now() -> DateTime<Utc> {
    "2026-09-17T08:00:00Z".parse().unwrap()
}
pub(crate) fn input() -> OrdinaryDailyChangeWindowRequest {
    OrdinaryDailyChangeWindowRequest {
        instrument: InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap(),
        from: "2026-09-11".parse().unwrap(),
        to: "2026-09-15".parse().unwrap(),
        as_of: "2026-09-16T08:00:00Z".parse().unwrap(),
        source_profile: c::TEST_PROFILE.into(),
    }
}
fn refs(id: &str) -> Vec<EvidenceRef> {
    vec![EvidenceRef {
        native_response_id: "native1".into(),
        fact_id: id.into(),
    }]
}
fn decimal(value: &str, unit: &str) -> SourceDecimal {
    SourceDecimal {
        value: value.into(),
        unit: unit.into(),
        scale: 0,
    }
}
pub(crate) fn evidence(request: &QueryRequest) -> EvidenceV1 {
    let r: c::RequestV1 = serde_json::from_slice(&request.payload.as_ref().unwrap().data).unwrap();
    let source = c::SourceV1 {
        provider: "HithinkFinance".into(),
        source: "TEST_CODE_SYNTHETIC_NATIVE".into(),
        profile_id: "TEST_CODE_SYNTHETIC".into(),
        profile_version: 1,
        batch_id: "TEST_CODE_batch".into(),
        source_at: c::SourceAt::NotProvided,
        observed_at: now(),
    };
    let sessions: Vec<_> = r
        .calendar
        .expected_sessions
        .iter()
        .enumerate()
        .map(|(i, date)| {
            let price = match i {
                0 => "100",
                1 => "130",
                _ => "90",
            };
            let bar = OrdinaryDailyBar {
                instrument: r.instrument.clone(),
                date: *date,
                open: decimal(price, "CNY/share"),
                high: decimal(price, "CNY/share"),
                low: decimal(price, "CNY/share"),
                close: decimal(price, "CNY/share"),
                volume: decimal("1000", "share"),
                amount: decimal("100000", "CNY"),
                adjustment: "Unadjusted".into(),
            };
            let id = format!("s{i}");
            c::Session {
                id: id.clone(),
                date: *date,
                terminal: Terminal::Bar { bar },
                availability_ref: "av1".into(),
                revision_ref: "rev1".into(),
                evidence_refs: refs(&id),
            }
        })
        .collect();
    EvidenceV1 {
        request_binding: c::Binding {
            request_id: request.context.as_ref().unwrap().request_id.clone(),
            issued_query_sha256: c::sha(&request.encode_to_vec()),
            request: r.clone(),
        },
        source,
        native_requests: Vec::new(),
        native_responses: Vec::new(),
        identity_proof: c::IdentityProof {
            instrument: r.instrument.clone(),
            evidence_refs: refs("identity"),
        },
        adjustment_proof: c::AdjustmentProof {
            mode: "Unadjusted".into(),
            evidence_refs: refs("adjustment"),
        },
        range_terminal: c::RangeTerminal {
            from: r.from,
            to: r.to,
            source_query_identity: "query1".into(),
            exhaustive: "Exhausted".into(),
            pages: vec!["native1".into()],
            last_page: "native1".into(),
            snapshot_id: "snap1".into(),
            revision_id: "version1".into(),
            evidence_refs: refs("range"),
        },
        availability: vec![c::Availability {
            id: "av1".into(),
            available_at: r.as_of,
            time_precision: "Second".into(),
            source_timezone: "UTC".into(),
            selected_as_of: r.as_of,
            record_ids: sessions
                .iter()
                .map(|s| s.id.clone())
                .chain(["listing".into(), "actions".into()])
                .collect(),
            evidence_refs: refs("av1"),
        }],
        revision: vec![c::Revision {
            id: "rev1".into(),
            snapshot_id: "snap1".into(),
            revision_id: "version1".into(),
            correction: c::Correction::Initial {
                evidence_refs: refs("initial"),
            },
            selection_as_of_proof_refs: refs("rev1"),
        }],
        sessions,
        lifecycle: c::Lifecycle {
            instrument: r.instrument,
            provider: "HithinkFinance".into(),
            source: "TEST_CODE_SYNTHETIC_NATIVE".into(),
            from: r.from,
            to: r.to,
            listing_date: "2000-01-01".parse().unwrap(),
            delisting_date: None,
            action_coverage: "None".into(),
            actions: Vec::new(),
            evidence_refs: vec![refs("listing")[0].clone(), refs("actions")[0].clone()],
        },
        errors: Vec::new(),
    }
}
pub(crate) fn seal_native(e: &mut EvidenceV1) {
    let q = c::NativeQuery {
        protocol: "TEST_CODE_NATIVE_WINDOW_V1".into(),
        query_id: e.range_terminal.source_query_identity.clone(),
        request: e.request_binding.request.clone(),
    };
    let bytes = c::encode(&q, c::MIB).unwrap();
    e.native_requests = vec![c::NativeRequest {
        id: "req1".into(),
        native_hex: hex::encode(&bytes),
        sha256: c::sha(&bytes),
    }];
    let n = c::NativeFacts {
        protocol: "TEST_CODE_NATIVE_WINDOW_V1".into(),
        request_query_id: e.range_terminal.source_query_identity.clone(),
        source: e.source.clone(),
        identity: e.identity_proof.clone(),
        adjustment: e.adjustment_proof.clone(),
        range: e.range_terminal.clone(),
        availability: e.availability.clone(),
        revision: e.revision.clone(),
        sessions: e.sessions.clone(),
        lifecycle: e.lifecycle.clone(),
        errors: e.errors.clone(),
        prior_revisions: Vec::new(),
    };
    let bytes = c::encode(&n, c::MIB).unwrap();
    e.native_responses = vec![c::NativeResponse {
        id: "native1".into(),
        request_id: "req1".into(),
        native_hex: hex::encode(&bytes),
        sha256: c::sha(&bytes),
    }];
}
fn response(e: &EvidenceV1) -> QueryResponse {
    QueryResponse {
        request_id: e.request_binding.request_id.clone(),
        operation: Operation::HistoricalBars as i32,
        admission: AdmissionState::Admitted as i32,
        selected_provider: e.source.provider.clone(),
        batch_id: e.source.batch_id.clone(),
        complete: true,
        observed_at: e.source.observed_at.to_rfc3339(),
        source_at: match &e.source.source_at {
            c::SourceAt::NotProvided => String::new(),
            c::SourceAt::Present(t) => t.to_rfc3339(),
        },
        records: vec![CanonicalPayload {
            schema: c::RESULT_SCHEMA.into(),
            schema_version: 1,
            content_type: "application/json; charset=utf-8".into(),
            data: c::encode(e, c::PROOF_LIMIT).unwrap(),
        }],
        diagnostic_blocker: String::new(),
    }
}
pub(crate) fn reply(request: &QueryRequest) -> Result<QueryResponse, tonic::Status> {
    let mut e = evidence(request);
    seal_native(&mut e);
    Ok(response(&e))
}
pub(crate) fn empty_reply(request: &QueryRequest) -> Result<QueryResponse, tonic::Status> {
    let mut e = evidence(request);
    for s in &mut e.sessions {
        if let Terminal::Bar { bar } = &mut s.terminal {
            bar.open = decimal("100", "CNY/share");
            bar.high = bar.open.clone();
            bar.low = bar.open.clone();
            bar.close = bar.open.clone();
        }
    }
    seal_native(&mut e);
    Ok(response(&e))
}
pub(crate) fn changed_reply(request: &QueryRequest) -> Result<QueryResponse, tonic::Status> {
    let mut e = evidence(request);
    if let Terminal::Bar { bar } = &mut e.sessions[1].terminal {
        bar.volume = decimal("1001", "share");
    }
    seal_native(&mut e);
    Ok(response(&e))
}
pub(crate) async fn acquire_fixture(
    handler: fn(&QueryRequest) -> Result<QueryResponse, tonic::Status>,
) -> QualifiedDailyChangeWindow {
    acquire_fixture_at(handler, now()).await
}
pub(crate) async fn acquire_fixture_at(
    handler: fn(&QueryRequest) -> Result<QueryResponse, tonic::Status>,
    invoked: DateTime<Utc>,
) -> QualifiedDailyChangeWindow {
    let fixture = ExternalQueryWireFixture::bind_window(handler)
        .await
        .unwrap();
    fixture.release_capabilities();
    fixture.release();
    let q = acquire(input(), fixture.bundle_path(), invoked)
        .await
        .unwrap();
    let s = fixture.snapshot();
    assert_eq!((s.health_calls, s.capabilities_calls, s.calls), (1, 1, 1));
    assert_eq!(s.authorized, vec![true]);
    assert_eq!(s.methods, vec!["historical_bars"]);
    assert_eq!(s.tcp_accepts, 1);
    fixture.finish().await.unwrap();
    q
}
pub(crate) fn database() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE.sqlite");
    let mut conn = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
    review::install_owned_test_fixture(&mut conn);
    drop(conn);
    (dir, path)
}
#[tokio::test]
async fn wg07_connected_mtls_prepare_reopen_confirm_and_exact_consume() {
    let (_dir, path) = database();
    let fixture = ExternalQueryWireFixture::bind_window(reply).await.unwrap();
    fixture.release_capabilities();
    fixture.release();
    let receipt = prepare_at(input(), fixture.bundle_path(), &path, now())
        .await
        .unwrap();
    assert_eq!(receipt.window_status, WindowStatus::Candidates);
    assert_eq!(receipt.candidates.len(), 2);
    let mut conn = open_existing(&path, false).unwrap();
    for candidate in &receipt.candidates {
        let read = review::review_on_conn(&mut conn, &candidate.candidate_id, now()).unwrap();
        assert_eq!(read.status, "Pending");
        review::decide_on_conn(
            &mut conn,
            &read.candidate_id,
            &read.evidence_token,
            review::ReviewDecision::Confirm,
            "TEST_CODE_operator",
            "reviewed exact native bars",
            now(),
        )
        .unwrap();
    }
    drop(conn);
    fixture.release_capabilities();
    fixture.release();
    let admitted = consume_at(
        input(),
        fixture.bundle_path(),
        &path,
        now() + chrono::Duration::seconds(1),
    )
    .await
    .unwrap();
    assert_eq!(admitted.bars().len(), 3);
    assert_eq!(admitted.accepted_candidate_ids().len(), 2);
    assert_ne!(admitted.request_identity(), receipt.request_identity);
    let capture = fixture.snapshot();
    assert_eq!(
        (
            capture.health_calls,
            capture.capabilities_calls,
            capture.calls
        ),
        (2, 2, 2)
    );
    assert!(capture.authorized.iter().all(|x| *x));
    fixture.finish().await.unwrap();
    let fixture = ExternalQueryWireFixture::bind_window(changed_reply)
        .await
        .unwrap();
    fixture.release_capabilities();
    fixture.release();
    assert!(consume_at(input(), fixture.bundle_path(), &path, now())
        .await
        .is_err());
    fixture.finish().await.unwrap();
}
#[tokio::test]
async fn wg07_connected_no_changes_has_durable_window_and_no_candidates() {
    let (_d, path) = database();
    let fixture = ExternalQueryWireFixture::bind_window(empty_reply)
        .await
        .unwrap();
    fixture.release_capabilities();
    fixture.release();
    let receipt = prepare_at(input(), fixture.bundle_path(), &path, now())
        .await
        .unwrap();
    assert_eq!(receipt.window_status, WindowStatus::NoChanges);
    assert!(receipt.candidates.is_empty());
    let mut conn = open_existing(&path, true).unwrap();
    review::require_window_store(&mut conn).unwrap();
    fixture.release_capabilities();
    fixture.release();
    assert_eq!(
        consume_at(input(), fixture.bundle_path(), &path, now())
            .await
            .unwrap()
            .bars()
            .len(),
        3
    );
    fixture.finish().await.unwrap();
}
#[tokio::test]
async fn wg07_capability_scope_rejects_before_rpc_and_retains_health_caps() {
    let fixture = ExternalQueryWireFixture::bind_window(reply).await.unwrap();
    fixture.window_scope_unavailable();
    fixture.release_capabilities();
    let e = acquire(input(), fixture.bundle_path(), now())
        .await
        .err()
        .unwrap();
    assert!(e.retained_evidence.as_ref().unwrap().health_hex.is_some());
    assert!(e
        .retained_evidence
        .as_ref()
        .unwrap()
        .capabilities_hex
        .is_some());
    assert_eq!(fixture.snapshot().calls, 0);
    fixture.finish().await.unwrap();
}
#[test]
fn wg07_native_semantic_mutations_reject_and_aggregate_days() {
    let p = c::profile(c::TEST_PROFILE).unwrap();
    let frozen = c::freeze(input(), now(), &p).unwrap();
    let query = build_ordinary_window_query(&frozen, &p)
        .unwrap()
        .into_request();
    let bytes = query.encode_to_vec();
    let mutations: Vec<(FailureKind, fn(&mut EvidenceV1))> = vec![
        (FailureKind::NativeIdentityMissingOrMismatch, |e| {
            e.identity_proof.evidence_refs.clear()
        }),
        (FailureKind::NativeIdentityMissingOrMismatch, |e| {
            e.identity_proof.instrument =
                InstrumentId::new(Exchange::Shanghai, "600000", AssetClass::Equity).unwrap()
        }),
        (FailureKind::AdjustmentMissingOrMismatch, |e| {
            e.adjustment_proof.mode = "Adjusted".into()
        }),
        (FailureKind::IncompleteRange, |e| {
            e.range_terminal.to = e.range_terminal.from
        }),
        (FailureKind::UnknownSourceTerminal, |e| {
            e.range_terminal.exhaustive = "Unknown".into()
        }),
        (FailureKind::IncompleteRange, |e| {
            e.sessions.pop();
        }),
        (FailureKind::DuplicateOrUnexpectedSession, |e| {
            e.sessions[1].date = e.sessions[0].date
        }),
        (FailureKind::PublicationMissingOrAfterAsOf, |e| {
            e.availability[0].available_at += chrono::Duration::seconds(1)
        }),
        (FailureKind::PublicationMissingOrAfterAsOf, |e| {
            e.availability.clear()
        }),
        (FailureKind::RevisionMissingOrConflict, |e| {
            e.revision[0].revision_id = "conflict".into()
        }),
        (FailureKind::CorrectionProofMissing, |e| {
            e.revision[0].correction = c::Correction::Replaces {
                prior_revision_refs: Vec::new(),
                evidence_refs: refs("rev1"),
            }
        }),
        (FailureKind::LifecycleCoverageMissing, |e| {
            e.lifecycle.provider = "Tdx".into()
        }),
        (FailureKind::LifecycleCoverageMissing, |e| {
            e.lifecycle.evidence_refs.clear()
        }),
        (FailureKind::InvalidBar, |e| {
            if let Terminal::Bar { bar } = &mut e.sessions[1].terminal {
                bar.low = decimal("999", "CNY/share")
            }
        }),
        (FailureKind::SourceRejected, |e| {
            e.errors.push(c::SourceError {
                instrument: e.identity_proof.instrument.clone(),
                date: None,
                typed_source_reason: "source rejection".into(),
                retryable: false,
                evidence_refs: refs("range"),
            })
        }),
    ];
    for (kind, mutate) in mutations {
        let mut e = evidence(&query);
        mutate(&mut e);
        seal_native(&mut e);
        let err = c::interpret(&c::encode(&e, c::PROOF_LIMIT).unwrap(), &frozen, &bytes, &p)
            .err()
            .unwrap();
        assert!(
            err.failures().iter().any(|f| f.kind == kind),
            "expected {kind:?}: {err}"
        );
    }
    let mut e = evidence(&query);
    for s in &mut e.sessions {
        if let Terminal::Bar { bar } = &mut s.terminal {
            bar.close = decimal("0", "CNY/share");
        }
    }
    seal_native(&mut e);
    let e = c::interpret(&c::encode(&e, c::PROOF_LIMIT).unwrap(), &frozen, &bytes, &p)
        .err()
        .unwrap();
    assert_eq!(
        e.failures()
            .iter()
            .filter(|f| f.kind == FailureKind::InvalidBar)
            .count(),
        3
    );
}
#[test]
fn wg07_outer_echo_cannot_replace_native_identity_and_request_binding() {
    let p = c::profile(c::TEST_PROFILE).unwrap();
    let f = c::freeze(input(), now(), &p).unwrap();
    let q = build_ordinary_window_query(&f, &p).unwrap().into_request();
    let mut e = evidence(&q);
    seal_native(&mut e);
    e.identity_proof.instrument =
        InstrumentId::new(Exchange::Shanghai, "600000", AssetClass::Equity).unwrap();
    assert!(c::interpret(
        &c::encode(&e, c::PROOF_LIMIT).unwrap(),
        &f,
        &q.encode_to_vec(),
        &p
    )
    .is_err());
    let mut e = evidence(&q);
    seal_native(&mut e);
    e.request_binding.issued_query_sha256 = "0".repeat(64);
    assert_eq!(
        c::interpret(
            &c::encode(&e, c::PROOF_LIMIT).unwrap(),
            &f,
            &q.encode_to_vec(),
            &p
        )
        .err()
        .unwrap()
        .failures()[0]
            .kind,
        FailureKind::RequestBindingMismatch
    );
}
#[test]
fn wg07_resource_request_scalar_native_and_session_limits_before_owned_decode() {
    assert!(c::preflight(&vec![b' '; c::PROOF_LIMIT + 1], c::PROOF_LIMIT).is_err());
    let long = format!("{{\"x\":\"{}\"}}", "a".repeat(16385));
    assert!(c::preflight(long.as_bytes(), c::PROOF_LIMIT).is_err());
    assert!(c::encode(&"x".repeat(32), 8).is_err());
    let p = c::profile(c::TEST_PROFILE).unwrap();
    let mut i = input();
    i.from = "0001-01-01".parse().unwrap();
    assert!(c::freeze(i, now(), &p).is_err());
    let mut i = input();
    i.as_of = now() + chrono::Duration::days(1);
    assert!(c::freeze(i, now(), &p).is_err());
    let mut i = input();
    i.instrument = InstrumentId::new(Exchange::Shenzhen, "000001", AssetClass::Equity).unwrap();
    assert_eq!(
        c::freeze(i, now(), &p).err().unwrap().failures()[0].kind,
        FailureKind::CalendarUnavailableOrInapplicable
    );
    let native = format!("{{\"native_hex\":\"{}\"}}", "0".repeat(2 * c::MIB + 2));
    assert!(c::preflight(native.as_bytes(), c::PROOF_LIMIT).is_err());
}
#[test]
fn wg07_exact_decimal_twenty_percent_and_integer_overflow() {
    let p = c::profile(c::TEST_PROFILE).unwrap();
    let f = c::freeze(input(), now(), &p).unwrap();
    let q = build_ordinary_window_query(&f, &p).unwrap().into_request();
    let e = evidence(&q);
    let Terminal::Bar { bar: a } = e.sessions[0].terminal.clone() else {
        panic!()
    };
    let mut b = a.clone();
    b.close = decimal("120", "CNY/share");
    assert!(!c::anomalous(&a, &b).unwrap());
    b.close = SourceDecimal {
        value: "120.00000001".into(),
        unit: "CNY/share".into(),
        scale: 8,
    };
    assert!(c::anomalous(&a, &b).unwrap());
    assert!(decimal(&"9".repeat(63), "CNY/share")
        .integer("CNY/share")
        .is_err());
}

#[tokio::test]
async fn wg07_failed_health_and_capabilities_keep_actual_control_response_bytes() {
    let fixture = ExternalQueryWireFixture::bind_window(reply).await.unwrap();
    fixture.set_invalid_health_identity(true);
    let err = acquire(input(), fixture.bundle_path(), now())
        .await
        .err()
        .unwrap();
    let retained = err.retained_evidence.unwrap();
    assert!(retained.health_hex.is_some());
    assert!(retained.capabilities_hex.is_none());
    assert!(retained.wire.is_none());
    assert_eq!(fixture.snapshot().calls, 0);
    fixture.finish().await.unwrap();
    let fixture = ExternalQueryWireFixture::bind_window(reply).await.unwrap();
    fixture.window_invalid_capabilities();
    fixture.release_capabilities();
    let err = acquire(input(), fixture.bundle_path(), now())
        .await
        .err()
        .unwrap();
    let retained = err.retained_evidence.unwrap();
    assert!(retained.health_hex.is_some());
    assert!(retained.capabilities_hex.is_some());
    assert!(retained.wire.is_none());
    assert_eq!(fixture.snapshot().calls, 0);
    fixture.finish().await.unwrap();
}

#[tokio::test]
async fn wg07_connected_invalid_native_windows_never_append_candidates() {
    let mutations: Vec<fn(&mut EvidenceV1)> = vec![
        |e| e.identity_proof.evidence_refs.clear(),
        |e| e.adjustment_proof.mode = "Adjusted".into(),
        |e| e.range_terminal.exhaustive = "Unknown".into(),
        |e| {
            e.sessions.pop();
        },
        |e| e.sessions[1].date = e.sessions[0].date,
        |e| e.availability.clear(),
        |e| e.availability[0].available_at += chrono::Duration::seconds(1),
        |e| e.revision[0].revision_id = "wrong".into(),
        |e| {
            e.revision[0].correction = c::Correction::Replaces {
                prior_revision_refs: Vec::new(),
                evidence_refs: refs("rev1"),
            }
        },
        |e| e.lifecycle.evidence_refs.clear(),
        |e| {
            e.errors.push(c::SourceError {
                instrument: e.identity_proof.instrument.clone(),
                date: None,
                typed_source_reason: "TEST_CODE source error".into(),
                retryable: true,
                evidence_refs: refs("range"),
            })
        },
    ];
    for mutate in mutations {
        let (_d, path) = database();
        let fixture = ExternalQueryWireFixture::bind_window(move |q| {
            let mut e = evidence(q);
            mutate(&mut e);
            seal_native(&mut e);
            Ok(response(&e))
        })
        .await
        .unwrap();
        fixture.release_capabilities();
        fixture.release();
        let e = prepare_at(input(), fixture.bundle_path(), &path, now())
            .await
            .err()
            .unwrap();
        assert!(e.retained_evidence.as_ref().unwrap().wire.is_some());
        let mut conn = open_existing(&path, true).unwrap();
        #[derive(diesel::QueryableByName)]
        struct Count {
            #[diesel(sql_type=diesel::sql_types::BigInt)]
            n: i64,
        }
        use diesel::RunQueryDsl;
        let count: Count = diesel::sql_query("SELECT count(*) AS n FROM daily_change_review_event")
            .get_result(&mut conn)
            .unwrap();
        assert_eq!(count.n, 0);
        assert_eq!(fixture.snapshot().calls, 1);
        fixture.finish().await.unwrap();
    }
}
#[tokio::test]
async fn wg07_connected_confirmed_fact_mutations_never_reuse_permission() {
    let (_dir, path) = database();
    let q = acquire_fixture(reply).await;
    let mut conn = open_existing(&path, false).unwrap();
    let receipt = review::prepare_window_on_conn(&mut conn, &q, now()).unwrap();
    for r in &receipt.candidates {
        review::decide_on_conn(
            &mut conn,
            &r.candidate_id,
            &r.evidence_token,
            review::ReviewDecision::Confirm,
            "TEST_CODE_op",
            "exact",
            now(),
        )
        .unwrap();
    }
    drop(conn);
    let mutations: Vec<fn(&mut EvidenceV1)> = vec![
        |e| {
            if let Terminal::Bar { bar } = &mut e.sessions[0].terminal {
                bar.high = decimal("101", "CNY/share")
            }
        },
        |e| {
            if let Terminal::Bar { bar } = &mut e.sessions[1].terminal {
                bar.volume = decimal("1001", "share")
            }
        },
        |e| {
            e.source.source = "TEST_CODE_other_native".into();
            e.lifecycle.source = e.source.source.clone();
        },
        |e| {
            e.lifecycle.action_coverage = "Complete".into();
            e.lifecycle.actions.push(c::Action {
                effective_on: e.sessions[1].date,
                record_on: None,
                ex_on: None,
                payable_on: None,
                status: "Implemented".into(),
                category: "Distribution".into(),
                terms: "TEST_CODE cash 1 CNY/share".into(),
                evidence_refs: refs("actions"),
            });
        },
        |e| {
            e.sessions[1].terminal = Terminal::Suspended {
                reason: "ExchangeSuspension".into(),
                bridge: true,
            };
            if let Terminal::Bar { bar } = &mut e.sessions[2].terminal {
                bar.open = decimal("60", "CNY/share");
                bar.high = bar.open.clone();
                bar.low = bar.open.clone();
                bar.close = bar.open.clone();
            }
        },
    ];
    for mutate in mutations {
        let fixture = ExternalQueryWireFixture::bind_window(move |r| {
            let mut e = evidence(r);
            mutate(&mut e);
            seal_native(&mut e);
            Ok(response(&e))
        })
        .await
        .unwrap();
        fixture.release_capabilities();
        fixture.release();
        let result = consume_at(input(), fixture.bundle_path(), &path, now()).await;
        assert!(result.is_err());
        fixture.finish().await.unwrap();
    }
    for change in ["code", "date"] {
        let mut request = input();
        if change == "code" {
            request.instrument =
                InstrumentId::new(Exchange::Shanghai, "600000", AssetClass::Equity).unwrap();
        } else {
            request.from = "2026-09-10".parse().unwrap();
        }
        let fixture = ExternalQueryWireFixture::bind_window(reply).await.unwrap();
        fixture.release_capabilities();
        fixture.release();
        assert!(consume_at(request, fixture.bundle_path(), &path, now())
            .await
            .is_err());
        fixture.finish().await.unwrap();
    }
}
#[test]
fn wg07_suspension_bridges_and_native_manifests_keep_material_fact_stable() {
    let p = c::profile(c::TEST_PROFILE).unwrap();
    let f = c::freeze(input(), now(), &p).unwrap();
    let q = build_ordinary_window_query(&f, &p).unwrap().into_request();
    let mut e = evidence(&q);
    seal_native(&mut e);
    let original = c::interpret(
        &c::encode(&e, c::PROOF_LIMIT).unwrap(),
        &f,
        &q.encode_to_vec(),
        &p,
    )
    .unwrap();
    e.source.batch_id = "TEST_CODE_fresh_batch".into();
    e.range_terminal.snapshot_id = "fresh_snapshot".into();
    e.range_terminal.revision_id = "fresh_version".into();
    e.revision[0].snapshot_id = "fresh_snapshot".into();
    e.revision[0].revision_id = "fresh_version".into();
    seal_native(&mut e);
    let fresh = c::interpret(
        &c::encode(&e, c::PROOF_LIMIT).unwrap(),
        &f,
        &q.encode_to_vec(),
        &p,
    )
    .unwrap();
    assert_eq!(original.pairs, fresh.pairs);
    e.sessions[1].terminal = Terminal::Suspended {
        reason: "ExchangeSuspension".into(),
        bridge: true,
    };
    if let Terminal::Bar { bar } = &mut e.sessions[2].terminal {
        bar.open = decimal("60", "CNY/share");
        bar.high = bar.open.clone();
        bar.low = bar.open.clone();
        bar.close = bar.open.clone();
    }
    seal_native(&mut e);
    let bridged = c::interpret(
        &c::encode(&e, c::PROOF_LIMIT).unwrap(),
        &f,
        &q.encode_to_vec(),
        &p,
    )
    .unwrap();
    assert_eq!(bridged.pairs.len(), 1);
    assert_eq!(bridged.pairs[0].bridges.len(), 1);
    e.sessions[1].terminal = Terminal::Suspended {
        reason: "ExchangeSuspension".into(),
        bridge: false,
    };
    seal_native(&mut e);
    assert!(c::interpret(
        &c::encode(&e, c::PROOF_LIMIT).unwrap(),
        &f,
        &q.encode_to_vec(),
        &p
    )
    .is_err());
}
#[test]
fn wg07_strict_unknown_fields_nested_identity_and_collection_limits() {
    let p = c::profile(c::TEST_PROFILE).unwrap();
    let f = c::freeze(input(), now(), &p).unwrap();
    let q = build_ordinary_window_query(&f, &p).unwrap().into_request();
    let mut e = evidence(&q);
    seal_native(&mut e);
    let mut v = serde_json::to_value(&e).unwrap();
    v["identity_proof"]["instrument"]["extra"] = true.into();
    assert!(c::decode::<EvidenceV1>(&serde_json::to_vec(&v).unwrap(), c::PROOF_LIMIT).is_err());
    for (field, count) in [
        ("sessions", 261),
        ("actions", 1025),
        ("native_requests", 1025),
    ] {
        let bytes = format!("{{\"{field}\":[{}]}}", vec!["{}"; count].join(","));
        assert!(c::preflight(bytes.as_bytes(), c::PROOF_LIMIT).is_err());
    }
    let bytes = format!(
        "{{\"native_requests\":[{}],\"native_responses\":[{}]}}",
        vec!["{}"; 512].join(","),
        vec!["{}"; 513].join(",")
    );
    assert!(c::preflight(bytes.as_bytes(), c::PROOF_LIMIT).is_err());
}

#[test]
fn wg07_native_replacement_manifest_is_checked_without_changing_material_identity() {
    let p = c::profile(c::TEST_PROFILE).unwrap();
    let f = c::freeze(input(), now(), &p).unwrap();
    let q = build_ordinary_window_query(&f, &p).unwrap().into_request();
    let mut e = evidence(&q);
    seal_native(&mut e);
    let original = c::interpret(
        &c::encode(&e, c::PROOF_LIMIT).unwrap(),
        &f,
        &q.encode_to_vec(),
        &p,
    )
    .unwrap();
    e.revision[0].correction = c::Correction::Replaces {
        prior_revision_refs: refs("prior1"),
        evidence_refs: refs("rev1"),
    };
    seal_native(&mut e);
    let mut n: c::NativeFacts = c::decode(
        &hex::decode(&e.native_responses[0].native_hex).unwrap(),
        c::MIB,
    )
    .unwrap();
    n.prior_revisions.push(c::PriorRevision {
        id: "prior1".into(),
        snapshot_id: "snapshot0".into(),
        revision_id: "version0".into(),
        available_at: e.availability[0].available_at - chrono::Duration::seconds(1),
        record_ids: e.sessions.iter().map(|s| s.id.clone()).collect(),
    });
    let bytes = c::encode(&n, c::MIB).unwrap();
    e.native_responses[0].native_hex = hex::encode(&bytes);
    e.native_responses[0].sha256 = c::sha(&bytes);
    let current = c::interpret(
        &c::encode(&e, c::PROOF_LIMIT).unwrap(),
        &f,
        &q.encode_to_vec(),
        &p,
    )
    .unwrap();
    assert_eq!(current.pairs, original.pairs);
    n.prior_revisions[0].available_at += chrono::Duration::days(1);
    let bytes = c::encode(&n, c::MIB).unwrap();
    e.native_responses[0].native_hex = hex::encode(&bytes);
    e.native_responses[0].sha256 = c::sha(&bytes);
    assert!(c::interpret(
        &c::encode(&e, c::PROOF_LIMIT).unwrap(),
        &f,
        &q.encode_to_vec(),
        &p
    )
    .is_err());
}
#[tokio::test]
async fn wg07_remote_status_preserves_wire_details_and_trailer_without_writes() {
    let fixture = ExternalQueryWireFixture::bind_window(|q| {
        use crate::grpc_client::external_pb::magic::market::v1::ErrorDetail;
        let bytes = ErrorDetail {
            request_id: q.context.as_ref().unwrap().request_id.clone(),
            operation: Operation::HistoricalBars as i32,
            provider: "HithinkFinance".into(),
            reason_code: "unavailable".into(),
            retryable: true,
            admission: AdmissionState::Admitted as i32,
            ..Default::default()
        }
        .encode_to_vec();
        let mut status = tonic::Status::with_details(
            tonic::Code::Unavailable,
            "TEST_CODE native unavailable",
            bytes.clone().into(),
        );
        status.metadata_mut().insert_bin(
            "magic-error-detail-bin",
            tonic::metadata::MetadataValue::from_bytes(&bytes),
        );
        Err(status)
    })
    .await
    .unwrap();
    fixture.release_capabilities();
    fixture.release();
    let err = acquire(input(), fixture.bundle_path(), now())
        .await
        .err()
        .unwrap();
    assert_eq!(err.failures()[0].kind, FailureKind::TransportFailure);
    let retained = err.retained_evidence.unwrap();
    assert!(retained.status.is_some());
    assert!(retained.wire.is_some());
    assert_eq!(fixture.snapshot().calls, 1);
    fixture.finish().await.unwrap();
}

#[test]
fn wg07_literal_stable_fact_golden_and_acquisition_domain_separation() {
    let pair: c::PairFact = c::decode(
        include_bytes!("../../grpc_handoffs/fixtures/wg07/synthetic-pair-fact-v1.json"),
        c::PROOF_LIMIT,
    )
    .unwrap();
    assert_eq!(
        c::digest(
            b"BR171_ORDINARY_WINDOW_FACT_V1\0",
            &c::encode(&pair, c::PROOF_LIMIT).unwrap()
        ),
        "db136d6eada54e696d7a0109364daed4864e1c6f7627b3446f74ba643616456b"
    );
    assert_ne!(
        c::digest(b"BR171_ORDINARY_WINDOW_REQUEST_V1\0", b"literal"),
        c::digest(b"BR171_ORDINARY_WINDOW_ACQUISITION_V1\0", b"literal")
    );
}

#[test]
fn wg07_literal_request_and_acquisition_domain_goldens() {
    assert_eq!(
        c::digest(b"BR171_ORDINARY_WINDOW_REQUEST_V1\0", b"literal"),
        "9316179ec51ee934ceffb4171322a5452f4f7d06272b8146f4ccea932b9ee432"
    );
    assert_eq!(
        c::digest(b"BR171_ORDINARY_WINDOW_PROOF_V1\0", b"literal"),
        "cf2eccb8be76955b74a46b5cc0a823ac3d878ffb51081a67d076a3107d733a8b"
    );
    assert_eq!(
        c::digest(b"BR171_ORDINARY_WINDOW_ACQUISITION_V1\0", b"literal"),
        "059897fa07ac5159a4055939f589be9d3ffb16e37acc41b7de523a510b3e65ec"
    );
}

#[test]
fn wg07_published_synthetic_wire_fixture_matches_native_interpreter() {
    let profile = c::profile(c::TEST_PROFILE).unwrap();
    let frozen = c::freeze(input(), now(), &profile).unwrap();
    let request: c::RequestV1 = c::decode(
        include_bytes!("../../grpc_handoffs/fixtures/wg07/synthetic-request-v1.json"),
        c::MIB,
    )
    .unwrap();
    assert_eq!(frozen.request, request);
    let wire = hex::decode(
        include_str!("../../grpc_handoffs/fixtures/wg07/synthetic-query-request.hex").trim(),
    )
    .unwrap();
    let interpreted = c::interpret(
        include_bytes!("../../grpc_handoffs/fixtures/wg07/synthetic-result-v1.json"),
        &frozen,
        &wire,
        &profile,
    )
    .unwrap();
    assert_eq!(interpreted.pairs.len(), 2);
    assert_eq!(interpreted.bars.len(), 3);
}

#[tokio::test]
async fn wg07_qualified_all_suspended_window_persists_no_changes() {
    let (_dir, path) = database();
    let fixture = ExternalQueryWireFixture::bind_window(|q| {
        let mut e = evidence(q);
        for s in &mut e.sessions {
            s.terminal = Terminal::Suspended {
                reason: "ExchangeSuspension".into(),
                bridge: true,
            };
        }
        seal_native(&mut e);
        Ok(response(&e))
    })
    .await
    .unwrap();
    fixture.release_capabilities();
    fixture.release();
    let receipt = prepare_at(input(), fixture.bundle_path(), &path, now())
        .await
        .unwrap();
    assert_eq!(receipt.window_status, WindowStatus::NoChanges);
    assert!(receipt.candidates.is_empty());
    let mut conn = open_existing(&path, true).unwrap();
    review::require_window_store(&mut conn).unwrap();
    fixture.finish().await.unwrap();
}

#[test]
fn wg07_wider_window_preserves_same_material_pairs() {
    let profile = c::profile(c::TEST_PROFILE).unwrap();
    let frozen = c::freeze(input(), now(), &profile).unwrap();
    let q = build_ordinary_window_query(&frozen, &profile)
        .unwrap()
        .into_request();
    let mut e = evidence(&q);
    seal_native(&mut e);
    let old = c::interpret(
        &c::encode(&e, c::PROOF_LIMIT).unwrap(),
        &frozen,
        &q.encode_to_vec(),
        &profile,
    )
    .unwrap();
    let mut input = input();
    input.from = "2026-09-10".parse().unwrap();
    let frozen = c::freeze(input, now(), &profile).unwrap();
    let q = build_ordinary_window_query(&frozen, &profile)
        .unwrap()
        .into_request();
    let mut wider = evidence(&q);
    for s in &mut wider.sessions {
        if let Some(original) = e.sessions.iter().find(|old| old.date == s.date) {
            s.terminal = original.terminal.clone();
        }
    }
    seal_native(&mut wider);
    let new = c::interpret(
        &c::encode(&wider, c::PROOF_LIMIT).unwrap(),
        &frozen,
        &q.encode_to_vec(),
        &profile,
    )
    .unwrap();
    assert_eq!(old.pairs, new.pairs);
}

#[tokio::test]
async fn wg07_review_fix_i2_stored_response_preflight_precedes_owned_decode() {
    let q = acquire_fixture(reply).await;
    let original = q.proof().clone();
    REPLAY_RESPONSE_DECODE_HITS.with(|hits| hits.set(0));
    assert!(inspect_proof(&original).is_ok());
    REPLAY_RESPONSE_DECODE_HITS.with(|hits| assert_eq!(hits.get(), 1));
    let decoded =
        QueryResponse::decode(hex::decode(&original.response_hex).unwrap().as_slice()).unwrap();
    for mutation in 0..3 {
        let mut response = decoded.clone();
        match mutation {
            0 => response.records.push(response.records[0].clone()),
            1 => response.batch_id = "x".repeat(16385),
            _ => {
                response.records[0].data =
                    format!("{{\"sessions\":[{}]}}", vec!["null"; 261].join(",")).into_bytes()
            }
        }
        let mut proof = original.clone();
        proof.response_hex = hex::encode(response.encode_to_vec());
        REPLAY_RESPONSE_DECODE_HITS.with(|hits| hits.set(0));
        assert!(inspect_proof(&proof).is_err());
        REPLAY_RESPONSE_DECODE_HITS.with(|hits| assert_eq!(hits.get(), 0));
    }
}

#[tokio::test]
async fn wg07_review_fix_i3_connected_lifecycle_interval_before_no_changes() {
    for mode in 0..3 {
        let (_dir, path) = database();
        let fixture = ExternalQueryWireFixture::bind_window(move |q| {
            let mut e = evidence(q);
            let listing = if mode == 2 {
                "1990-01-01"
            } else {
                "2030-01-01"
            }
            .parse()
            .unwrap();
            e.lifecycle.listing_date = listing;
            e.lifecycle.delisting_date = if mode == 1 {
                None
            } else {
                Some("2000-01-01".parse().unwrap())
            };
            for s in &mut e.sessions {
                s.terminal = if mode == 2 {
                    Terminal::Delisted {
                        delisting_date: e.lifecycle.delisting_date.unwrap(),
                    }
                } else {
                    Terminal::NotYetListed {
                        listing_date: listing,
                    }
                };
            }
            seal_native(&mut e);
            Ok(response(&e))
        })
        .await
        .unwrap();
        fixture.release_capabilities();
        fixture.release();
        let result = prepare_at(input(), fixture.bundle_path(), &path, now()).await;
        let mut conn = open_existing(&path, true).unwrap();
        #[derive(diesel::QueryableByName)]
        struct Count {
            #[diesel(sql_type=diesel::sql_types::BigInt)]
            n: i64,
        }
        use diesel::RunQueryDsl;
        let count: Count = diesel::sql_query("SELECT count(*) AS n FROM daily_change_review_event")
            .get_result(&mut conn)
            .unwrap();
        if mode == 0 {
            let failure = result.err().unwrap();
            assert!(failure
                .failures()
                .iter()
                .any(|f| f.kind == FailureKind::LifecycleCoverageMissing));
            assert_eq!(count.n, 0);
        } else {
            let receipt = result.unwrap();
            assert_eq!(receipt.window_status, WindowStatus::NoChanges);
            assert!(receipt.candidates.is_empty());
            assert_eq!(count.n, 1);
            review::require_window_store(&mut conn).unwrap();
        }
        assert_eq!(fixture.snapshot().calls, 1);
        fixture.finish().await.unwrap();
    }
}

#[tokio::test]
async fn wg07_review_fix_m1_public_preacquisition_failures_keep_context_and_stage() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");
    let empty = dir.path().join("empty.sqlite");
    drop(SqliteConnection::establish(empty.to_str().unwrap()).unwrap());
    for consume in [false, true] {
        for (stage, database) in [("profile", &missing), ("open", &missing), ("store", &empty)] {
            let mut request = input();
            if stage == "profile" {
                request.source_profile = "TEST_CODE_UNKNOWN@1".into();
            }
            let error = if consume {
                consume_window(request.clone(), &missing, database)
                    .await
                    .err()
                    .unwrap()
            } else {
                prepare_window(request.clone(), &missing, database)
                    .await
                    .err()
                    .unwrap()
            };
            assert_eq!(error.stage(), stage);
            assert!(error.request_identity().is_none());
            assert!(error.retained_evidence.is_none());
            assert!(error.retained_window_proof.is_none());
            assert!(error
                .failures()
                .iter()
                .all(|f| f.instrument.as_ref() == Some(&request.instrument)
                    && f.range == Some((request.from, request.to))));
        }
    }
}

#[tokio::test]
async fn wg07_review_fix_m1_fresh_unconfirmed_consumer_reports_admission_stage() {
    let (_dir, path) = database();
    let fixture = ExternalQueryWireFixture::bind_window(reply).await.unwrap();
    fixture.release_capabilities();
    fixture.release();
    let error = consume_at(input(), fixture.bundle_path(), &path, now())
        .await
        .err()
        .unwrap();
    assert_eq!(error.stage(), "admission");
    assert!(error.request_identity().is_some());
    assert!(error.retained_window_proof.is_some());
    assert!(error
        .failures()
        .iter()
        .all(|f| f.instrument.as_ref() == Some(&input().instrument)
            && f.range == Some((input().from, input().to))));
    fixture.finish().await.unwrap();
}

#[tokio::test]
async fn wg07_review_fix_m1_transport_setup_failure_reports_known_context_only() {
    let (dir, path) = database();
    let missing_bundle = dir.path().join("absent-client-bundle");
    let error = prepare_at(input(), &missing_bundle, &path, now())
        .await
        .err()
        .unwrap();
    assert_eq!(error.stage(), "transport");
    assert!(error.request_identity().is_some());
    assert!(error.retained_window_proof.is_none());
    let retained = error.retained_evidence.as_ref().unwrap();
    assert!(retained.wire.is_none());
    assert!(error
        .failures()
        .iter()
        .all(|f| f.instrument.as_ref() == Some(&input().instrument)
            && f.range == Some((input().from, input().to))));
}

pub(crate) fn reset_stored_decode_hits() {
    REPLAY_STORED_DECODE_HITS.with(|hits| hits.set([0; 4]));
}
pub(crate) fn stored_decode_hits() -> [usize; 4] {
    REPLAY_STORED_DECODE_HITS.with(|hits| hits.get())
}
pub(crate) fn oversized_stored_proofs(
    original: &CompleteWindowProof,
) -> Vec<(CompleteWindowProof, [usize; 4])> {
    use crate::grpc_client::external_pb::magic::market::v1::{
        CapabilitiesResponse, HealthResponse,
    };
    let oversized = "x".repeat(16385);
    let mut cases = Vec::new();
    for field in 0..5 {
        let mut request =
            QueryRequest::decode(hex::decode(&original.request_hex).unwrap().as_slice()).unwrap();
        match field {
            0 => request.context.as_mut().unwrap().request_id = oversized.clone(),
            1 => request.preferred_provider = oversized.clone(),
            2 => request.payload.as_mut().unwrap().schema = oversized.clone(),
            3 => request.payload.as_mut().unwrap().content_type = oversized.clone(),
            _ => {
                request.payload.as_mut().unwrap().data =
                    serde_json::to_vec(&serde_json::json!({"short":oversized})).unwrap()
            }
        }
        let raw = request.encode_to_vec();
        assert!(raw.len() < c::MIB);
        let mut proof = original.clone();
        proof.request_hex = hex::encode(raw);
        cases.push((proof, [1, 1, 0, 0]));
    }
    for field in 0..7 {
        let mut health =
            HealthResponse::decode(hex::decode(&original.health_hex).unwrap().as_slice()).unwrap();
        match field {
            0 => health.request_id = oversized.clone(),
            1 => health.state = oversized.clone(),
            2 => health.build_identity.as_mut().unwrap().service_version = oversized.clone(),
            3 => health.build_identity.as_mut().unwrap().source_revision = oversized.clone(),
            4 => health.build_identity.as_mut().unwrap().contract_sha256 = oversized.clone(),
            5 => health.build_identity.as_mut().unwrap().binary_sha256 = oversized.clone(),
            _ => health.build_identity.as_mut().unwrap().identity_error = oversized.clone(),
        }
        let mut proof = original.clone();
        proof.health_hex = hex::encode(health.encode_to_vec());
        cases.push((proof, [0, 0, 0, 0]));
    }
    for field in 0..4 {
        let mut caps = CapabilitiesResponse::decode(
            hex::decode(&original.capabilities_hex).unwrap().as_slice(),
        )
        .unwrap();
        match field {
            0 => caps.request_id = oversized.clone(),
            1 => caps.capabilities[0].provider = oversized.clone(),
            2 => caps.capabilities[0].exact_scope = oversized.clone(),
            _ => caps.capabilities[0].blocker = oversized.clone(),
        }
        let mut proof = original.clone();
        proof.capabilities_hex = hex::encode(caps.encode_to_vec());
        cases.push((proof, [1, 0, 0, 0]));
    }
    cases
}

#[tokio::test]
async fn wg07_review_fix2_stored_controls_and_request_reject_before_each_owned_decoder() {
    let q = acquire_fixture(reply).await;
    reset_stored_decode_hits();
    assert!(inspect_proof(q.proof()).is_ok());
    assert_eq!(stored_decode_hits(), [1, 1, 1, 1]);
    let cases = oversized_stored_proofs(q.proof());
    assert_eq!(cases.len(), 16);
    for (index, (proof, expected_hits)) in cases.into_iter().enumerate() {
        // These are canonical, within aggregate proof/request bounds; the
        // offending short field must be refused before its first owned call.
        proof_identities(&proof).unwrap();
        reset_stored_decode_hits();
        assert!(inspect_proof(&proof).is_err(), "mutation {index}");
        assert_eq!(stored_decode_hits(), expected_hits, "mutation {index}");
    }
}
