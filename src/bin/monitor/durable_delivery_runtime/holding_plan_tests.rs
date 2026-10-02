//! Actual isolated coordinator/runtime fixtures, never an owner-from-parts seam.
use super::*;
use crate::holding_plan::{prepare_tick_with, HoldingPlanCandidate};
use std::cell::Cell;
use std::sync::atomic::AtomicUsize;
use stock_analysis::data_gateway::{BatchEvidence, QuoteCoverageDisposition};
use stock_analysis::database::user_position_snapshot::UserPositionSnapshot;
use stock_analysis::market_domain::ProviderId;
use stock_analysis::portfolio::user_position_snapshot::UserPositionItemInput;

struct CountingSink {
    calls: Arc<AtomicUsize>,
    uncertain: bool,
}
impl AuthoritativeSinkPort for CountingSink {
    fn sink_identity(&self) -> &str {
        "TEST_CODE_T03_RUNTIME_SINK"
    }
    fn deliver(&self, _: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.uncertain {
            AuthoritativeSinkResult::Uncertain(stock_analysis::durable_delivery::TypedUncertainty {
                reason_code: "TEST_CODE_T03_UNCERTAIN".into(),
                evidence: b"TEST_CODE_RAW_UNCERTAIN".to_vec(),
                observed_at: Utc::now(),
            })
        } else {
            AuthoritativeSinkResult::Accepted(stock_analysis::durable_delivery::TypedReceipt {
                channel: "TEST_CODE_T03_CHANNEL".into(),
                provider: "TEST_CODE_T03_PROVIDER".into(),
                message_id: "TEST_CODE_T03_MESSAGE".into(),
                platform_message_id: Some("TEST_CODE_T03_PLATFORM".into()),
                accepted_at: Utc::now(),
                latency_ms: Some(1),
            })
        }
    }
}
struct Fixture {
    state: RuntimeState,
    calls: Arc<AtomicUsize>,
    namespace: super::super::tests::TestNamespaceDir,
    test_code: String,
}
impl Fixture {
    fn new(uncertain: bool) -> Self {
        let test_code = format!(
            "TEST_CODE_T03_BIN_{}_{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        );
        let namespace = super::super::tests::TestNamespaceDir::new(&test_code);
        let coordinator = Arc::new(
            DurableDeliveryCoordinator::open(CoordinatorConfig::test(
                std::path::PathBuf::from("data/test")
                    .join(&test_code)
                    .join("durable_delivery.sqlite3"),
                &test_code,
                format!("owner-{test_code}-0123456789abcdef"),
            ))
            .unwrap(),
        );
        let calls = Arc::new(AtomicUsize::new(0));
        Self {
            state: RuntimeState {
                namespace: RuntimeNamespace::Test {
                    test_code: test_code.clone(),
                },
                coordinator,
                append: Arc::new(
                    DurableDeliveryImmutableAppend::for_test_code(&test_code).unwrap(),
                ),
                sink: Arc::new(CountingSink {
                    calls: Arc::clone(&calls),
                    uncertain,
                }),
                counted_delivery_critical_section: Mutex::new(()),
                producer_ready: AtomicBool::new(true),
                schedule_hydrations: Mutex::new(Vec::new()),
                queued_schedule_hydration_ids: Mutex::new(Default::default()),
            },
            calls,
            namespace,
            test_code,
        }
    }
    fn counts(&self) -> (i64, i64, i64, i64) {
        let connection =
            rusqlite::Connection::open(self.namespace.path().join("durable_delivery.sqlite3"))
                .unwrap();
        let count = |table: &str| {
            connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
        };
        (
            count("delivery_decisions"),
            count("immutable_audit_outbox"),
            count("daily_budget_reservations"),
            count("delivery_attempts"),
        )
    }
    fn owned(&self, instrument: &InstrumentId) -> HoldingPlanOwnedOccurrence {
        match self
            .state
            .coordinator
            .inspect_holding_plan_occurrence(local_now().date_naive(), instrument)
            .unwrap()
        {
            HoldingPlanOccurrenceObservation::Owned(actual) => actual,
            _ => panic!("actual owner missing"),
        }
    }
}
fn local_now() -> chrono::DateTime<chrono::FixedOffset> {
    Utc::now().with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
}
fn snapshot(codes: &[&str]) -> UserPositionSnapshot {
    UserPositionSnapshot {
        snapshot_row_id: 41,
        snapshot_id: "TEST_CODE_T03_SNAPSHOT".into(),
        effective_at: local_now(),
        confirmed_at: local_now(),
        source: "TEST_CODE_USER_CONFIRMED".into(),
        confirm_empty: false,
        evidence_sha256: "a".repeat(64),
        items: codes
            .iter()
            .map(|code| UserPositionItemInput {
                code: (*code).into(),
                name: "TEST_CODE_NAME".into(),
                quantity: 300,
                cost_price: 8.0,
            })
            .collect(),
    }
}
fn quotes(codes: &[String]) -> crate::market_data::TopStockBatch {
    crate::market_data::TopStockBatch {
        stocks: codes
            .iter()
            .map(|code| stock_analysis::market_data::TopStock {
                code: code.clone(),
                name: "TEST_CODE_NAME".into(),
                price: 8.5,
                change_pct: 1.0,
                volume_ratio: None,
                main_net_yi: None,
            })
            .collect(),
        evidence: BatchEvidence {
            provider: ProviderId::Tencent,
            source: "TEST_CODE_QUOTE_SOURCE".into(),
            source_at: Some(local_now().to_rfc3339()),
            observed_at: local_now().to_rfc3339(),
            batch_id: "TEST_CODE_T03_BATCH".into(),
        },
        coverage: QuoteCoverageDisposition::Complete,
        requested: codes.to_vec(),
        rejected: Vec::new(),
        missing: Vec::new(),
    }
}
fn fresh(code: &str, changed: bool) -> crate::PreparedHoldingPlan {
    let mut input = snapshot(&[code]);
    if changed {
        input.snapshot_id = "TEST_CODE_CHANGED_SNAPSHOT".into();
        input.items[0].cost_price = 9.0;
    }
    crate::holding_plan::prepare_holding_plan_messages_with(
        &crate::push_templates::BannerCtx::test_default(),
        || Ok(Some(input)),
        |codes| Ok(quotes(codes)),
        local_now,
    )
    .unwrap()
    .pop()
    .unwrap()
}
fn original(f: &Fixture, code: &str) -> HoldingPlanCandidate {
    let prepared = fresh(code, false);
    let instrument = match prepared.binding.scope() {
        CountedDeliveryScope::Ticket { instrument } => instrument.clone(),
        _ => unreachable!(),
    };
    let envelope = envelope_from_binding(
        prepared.binding,
        PushKind::HoldingPlan,
        &prepared.text,
        None,
    )
    .unwrap();
    f.state
        .coordinator
        .prepare_holding_plan_occurrence(&envelope, 1, Utc::now())
        .unwrap();
    HoldingPlanCandidate::Original {
        owned: f.owned(&instrument),
        instrument,
    }
}

#[test]
fn t03_exact_owner_bin_mixed_snapshot_quotes_only_fresh_subset_once() {
    let f = Fixture::new(false);
    let original = original(&f, "600000");
    let original_bytes = match &original {
        HoldingPlanCandidate::Original { owned, .. } => owned.envelope().clone(),
        _ => unreachable!(),
    };
    let snapshots = Cell::new(0);
    let quote_calls = Cell::new(0);
    let result = prepare_tick_with(
        Some(&crate::push_templates::BannerCtx::test_default()),
        || {
            snapshots.set(snapshots.get() + 1);
            Ok(Some(snapshot(&["600000", "000001", "000002"])))
        },
        |date, instrument| {
            f.state
                .coordinator
                .inspect_holding_plan_occurrence(date, instrument)
                .map_err(|e| e.to_string())
        },
        |_| Ok(std::collections::HashSet::from(["000002".to_owned()])),
        |codes| {
            quote_calls.set(quote_calls.get() + 1);
            assert_eq!(codes, &["000001".to_owned()]);
            Ok(quotes(codes))
        },
        local_now(),
    )
    .unwrap();
    assert_eq!(snapshots.get(), 1);
    assert_eq!(quote_calls.get(), 1);
    assert_eq!(result.candidates.len(), 2);
    assert!(result.failures[0].contains("holding_plan_legacy_unknown"));
    for candidate in result.candidates {
        match candidate {
            HoldingPlanCandidate::Original { owned, .. } => {
                assert_eq!(owned.envelope(), &original_bytes)
            }
            HoldingPlanCandidate::Fresh(prepared) => {
                let raw: serde_json::Value =
                    serde_json::from_slice(prepared.binding.source_binding_canonical()).unwrap();
                assert_eq!(raw["requested_codes"], serde_json::json!(["000001"]));
                assert_eq!(
                    raw["quote_batch"]["requested"],
                    serde_json::json!(["000001"])
                );
                assert_eq!(raw["snapshot"]["snapshot_id"], "TEST_CODE_T03_SNAPSHOT");
            }
        }
    }
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn t03_exact_owner_bin_existing_survives_legacy_error_and_missing_banner_without_quote() {
    let f = Fixture::new(false);
    let _ = original(&f, "600000");
    let result = prepare_tick_with(
        None,
        || Ok(Some(snapshot(&["600000", "000001"]))),
        |date, instrument| {
            f.state
                .coordinator
                .inspect_holding_plan_occurrence(date, instrument)
                .map_err(|e| e.to_string())
        },
        |_| Err("TEST_CODE_LEGACY_IO_FAILED".into()),
        |_| panic!("no fresh quote permitted"),
        local_now(),
    )
    .unwrap();
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.failures.len(), 1);
    assert!(matches!(
        &result.candidates[0],
        HoldingPlanCandidate::Original { .. }
    ));
    let quote_calls = Cell::new(0);
    let result = prepare_tick_with(
        Some(&crate::push_templates::BannerCtx::test_default()),
        || Ok(Some(snapshot(&["600000", "000001"]))),
        |date, instrument| {
            f.state
                .coordinator
                .inspect_holding_plan_occurrence(date, instrument)
                .map_err(|e| e.to_string())
        },
        |_| Ok(Default::default()),
        |_| {
            quote_calls.set(quote_calls.get() + 1);
            Err("TEST_CODE_QUOTE_IO_FAILURE".into())
        },
        local_now(),
    )
    .unwrap();
    assert_eq!(quote_calls.get(), 1);
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.failures.len(), 1);
    assert!(matches!(
        &result.candidates[0],
        HoldingPlanCandidate::Original { .. }
    ));
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn t03_exact_owner_bin_prepare_race_returns_original_after_releasing_section_without_side_effects()
{
    let f = Fixture::new(false);
    let losing = fresh("600000", true);
    let winner = original(&f, "600000");
    let exact = match winner {
        HoldingPlanCandidate::Original { owned, .. } => owned.envelope().clone(),
        _ => unreachable!(),
    };
    let before = f.counts();
    let outcome =
        deliver_candidate_blocking(&f.state, HoldingPlanCandidate::Fresh(losing)).unwrap();
    let original = match outcome {
        HoldingPlanDispatchResult::AlreadyOwned(candidate) => candidate,
        _ => panic!("must re-govern winner"),
    };
    assert!(f.state.counted_delivery_critical_section.try_lock().is_ok());
    assert_eq!(f.counts(), before);
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    match original {
        HoldingPlanCandidate::Original { owned, .. } => assert_eq!(owned.envelope(), &exact),
        _ => panic!("must retain original"),
    }
}

#[test]
fn t03_exact_owner_bin_actual_physical_accepted_restart_never_requotes_or_resends() {
    let f = Fixture::new(false);
    assert!(matches!(
        deliver_candidate_blocking(
            &f.state,
            HoldingPlanCandidate::Fresh(fresh("600000", false))
        )
        .unwrap(),
        HoldingPlanDispatchResult::PhysicalAccepted
    ));
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    let restarted = RuntimeState {
        namespace: RuntimeNamespace::Test {
            test_code: f.test_code.clone(),
        },
        coordinator: Arc::new(
            DurableDeliveryCoordinator::open(CoordinatorConfig::test(
                std::path::PathBuf::from("data/test")
                    .join(&f.test_code)
                    .join("durable_delivery.sqlite3"),
                &f.test_code,
                format!("owner-restarted-{}-0123456789abcdef", f.test_code),
            ))
            .unwrap(),
        ),
        append: Arc::clone(&f.state.append),
        sink: Arc::clone(&f.state.sink),
        counted_delivery_critical_section: Mutex::new(()),
        producer_ready: AtomicBool::new(true),
        schedule_hydrations: Mutex::new(Vec::new()),
        queued_schedule_hydration_ids: Mutex::new(Default::default()),
    };
    let before = f.counts();
    let result = prepare_tick_with(
        None,
        || {
            let mut input = snapshot(&["600000"]);
            input.items[0].cost_price = 99.0;
            Ok(Some(input))
        },
        |date, instrument| {
            restarted
                .coordinator
                .inspect_holding_plan_occurrence(date, instrument)
                .map_err(|e| e.to_string())
        },
        |_| panic!("existing owner needs no legacy lookup"),
        |_| panic!("existing owner needs no quote"),
        local_now(),
    )
    .unwrap();
    assert!(matches!(
        reconcile_candidate_blocking(&restarted, result.candidates.into_iter().next().unwrap())
            .unwrap(),
        HoldingPlanLocalProgress::PhysicalAccepted
    ));
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.counts(), before);
}

#[test]
fn t03_exact_owner_bin_uncertain_three_observations_never_send_again_or_claim_complete() {
    let f = Fixture::new(true);
    assert!(matches!(
        deliver_candidate_blocking(
            &f.state,
            HoldingPlanCandidate::Fresh(fresh("600000", false))
        )
        .unwrap(),
        HoldingPlanDispatchResult::NotCompleted(_)
    ));
    let instrument = InstrumentId::new(Exchange::Shanghai, "600000", AssetClass::Equity).unwrap();
    let before = f.counts();
    for _ in 0..3 {
        let candidate = HoldingPlanCandidate::Original {
            instrument: instrument.clone(),
            owned: f.owned(&instrument),
        };
        assert!(matches!(
            reconcile_candidate_blocking(&f.state, candidate).unwrap(),
            HoldingPlanLocalProgress::NotSendable(_)
        ));
    }
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.counts(), before);
    let connection =
        rusqlite::Connection::open(f.namespace.path().join("durable_delivery.sqlite3")).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='holding_plan_daily'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn t03_exact_owner_bin_manual_accepted_is_not_physical_completion_or_resend() {
    let f = Fixture::new(true);
    let _ = deliver_candidate_blocking(
        &f.state,
        HoldingPlanCandidate::Fresh(fresh("600000", false)),
    )
    .unwrap();
    let instrument = InstrumentId::new(Exchange::Shanghai, "600000", AssetClass::Equity).unwrap();
    let owned = f.owned(&instrument);
    f.state
        .coordinator
        .resolve_uncertain(
            &stock_analysis::durable_delivery::ManualResolutionCommand {
                decision_identity: owned.envelope().decision_identity.clone(),
                disposition: stock_analysis::durable_delivery::ManualDisposition::Accepted {
                    receipt: None,
                },
                operator_identity: "TEST_CODE_T03_OPERATOR".into(),
                reason: "TEST_CODE_T03_MANUAL_ACCEPTED".into(),
                external_evidence: b"TEST_CODE_MANUAL_EXTERNAL_EVIDENCE".to_vec(),
                resolved_at: Utc::now(),
            },
            f.state.append.as_ref(),
        )
        .unwrap();
    let candidate = HoldingPlanCandidate::Original {
        instrument: instrument.clone(),
        owned: f.owned(&instrument),
    };
    assert!(matches!(
        reconcile_candidate_blocking(&f.state, candidate).unwrap(),
        HoldingPlanLocalProgress::NotSendable(_)
    ));
    let actual = f.owned(&instrument);
    assert_eq!(actual.state(), DecisionState::Delivered);
    assert_eq!(
        actual.receipt_kind(),
        HoldingPlanReceiptKind::ManualAccepted
    );
    assert!(actual.local_drained());
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
}

struct RejectingSink;
impl AuthoritativeSinkPort for RejectingSink {
    fn sink_identity(&self) -> &str {
        "TEST_CODE_T03_REJECTED_SINK"
    }
    fn deliver(&self, _: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        AuthoritativeSinkResult::Rejected(stock_analysis::durable_delivery::TypedRejection {
            reason_code: "TEST_CODE_T03_REJECTED".into(),
            evidence: b"TEST_CODE_REJECTED_RAW".to_vec(),
            retry_authorized: false,
            observed_at: Utc::now(),
        })
    }
}
#[test]
fn t03_exact_owner_bin_actual_retry_flag_is_observed_without_new_authorization() {
    let f = Fixture::new(false);
    let mut prepared = fresh("600000", false);
    prepared.binding.retry_authorized = false;
    let instrument = InstrumentId::new(Exchange::Shanghai, "600000", AssetClass::Equity).unwrap();
    let original_envelope = envelope_from_binding(
        prepared.binding,
        PushKind::HoldingPlan,
        &prepared.text,
        None,
    )
    .unwrap();
    f.state
        .coordinator
        .prepare_holding_plan_occurrence(&original_envelope, 1, Utc::now())
        .unwrap();
    f.state
        .coordinator
        .reconcile_all_pending(f.state.append.as_ref(), Utc::now())
        .unwrap();
    let rejected: AuthoritativeSink = Arc::new(RejectingSink);
    f.state
        .coordinator
        .resume_deliverable(
            &original_envelope.decision_identity,
            &[rejected],
            Utc::now(),
        )
        .unwrap();
    let candidate = HoldingPlanCandidate::Original {
        instrument: instrument.clone(),
        owned: f.owned(&instrument),
    };
    assert!(matches!(
        reconcile_candidate_blocking(&f.state, candidate).unwrap(),
        HoldingPlanLocalProgress::NotSendable(_)
    ));
    assert!(!f.owned(&instrument).retry_authorized());
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    // Only this explicit test operator authorizes. Neither bin adapter calls it.
    f.state
        .coordinator
        .authorize_rejected_retry(&original_envelope.decision_identity)
        .unwrap();
    let authorized = f.owned(&instrument);
    assert!(authorized.retry_authorized());
    assert!(!authorized.envelope().retry_authorized);
    let candidate = HoldingPlanCandidate::Original {
        instrument: instrument.clone(),
        owned: authorized,
    };
    let candidate = match reconcile_candidate_blocking(&f.state, candidate).unwrap() {
        HoldingPlanLocalProgress::Candidate(candidate) => candidate,
        _ => panic!("real stored authorization should permit original resume"),
    };
    assert!(matches!(
        deliver_candidate_blocking(&f.state, candidate).unwrap(),
        HoldingPlanDispatchResult::PhysicalAccepted
    ));
    assert_eq!(f.owned(&instrument).envelope(), &original_envelope);
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    let before = f.counts();
    let candidate = HoldingPlanCandidate::Original {
        instrument: instrument.clone(),
        owned: f.owned(&instrument),
    };
    assert!(matches!(
        reconcile_candidate_blocking(&f.state, candidate).unwrap(),
        HoldingPlanLocalProgress::PhysicalAccepted
    ));
    assert_eq!(f.counts(), before);
}

#[test]
fn t03_exact_owner_bin_non_shanghai_offset_same_instant_recovers_actual_owner_without_quote() {
    let f = Fixture::new(false);
    let captured = chrono::DateTime::parse_from_rfc3339("2026-10-01T20:30:02+00:00").unwrap();
    let shanghai = captured
        .with_timezone(&chrono::FixedOffset::east_opt(8 * 60 * 60).expect("valid Shanghai offset"))
        .fixed_offset();
    assert_ne!(captured.date_naive(), shanghai.date_naive());
    let prepared = crate::holding_plan::prepare_holding_plan_messages_with(
        &crate::push_templates::BannerCtx::test_default(),
        || Ok(Some(snapshot(&["600000"]))),
        |codes| Ok(quotes(codes)),
        || shanghai,
    )
    .unwrap()
    .pop()
    .unwrap();
    assert!(matches!(
        deliver_candidate_blocking(&f.state, HoldingPlanCandidate::Fresh(prepared)).unwrap(),
        HoldingPlanDispatchResult::PhysicalAccepted
    ));
    let before = f.counts();
    let result = prepare_tick_with(
        None,
        || Ok(Some(snapshot(&["600000"]))),
        |date, instrument| {
            assert_eq!(date, shanghai.date_naive());
            f.state
                .coordinator
                .inspect_holding_plan_occurrence(date, instrument)
                .map_err(|e| e.to_string())
        },
        |_| panic!("same instant must recover Shanghai owner"),
        |_| panic!("original owner cannot re-quote"),
        captured,
    )
    .unwrap();
    assert_eq!(result.candidates.len(), 1);
    assert!(result.failures.is_empty());
    let original = result.candidates.into_iter().next().unwrap();
    let exact = match &original {
        HoldingPlanCandidate::Original { owned, .. } => owned.envelope().clone(),
        _ => panic!("must recover actual owner"),
    };
    assert_eq!(exact.business_date, "2026-10-02");
    assert!(matches!(
        reconcile_candidate_blocking(&f.state, original).unwrap(),
        HoldingPlanLocalProgress::PhysicalAccepted
    ));
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.counts(), before);
}

#[test]
fn t03_exact_owner_bin_beijing_92_owner_does_not_open_shenzhen_scope() {
    let f = Fixture::new(false);
    let mut prepared = fresh("920001", false);
    match prepared.binding.scope() {
        CountedDeliveryScope::Ticket { instrument } => {
            assert_eq!(instrument.exchange(), Exchange::Beijing)
        }
        _ => panic!("must be exact equity ticket"),
    }
    // The real existing owner is explicitly Beijing, independent of any
    // subsequent producer lookup. Its capability still comes from actual DB.
    prepared.binding.scope = CountedDeliveryScope::Ticket {
        instrument: InstrumentId::new(Exchange::Beijing, "920001", AssetClass::Equity).unwrap(),
    };
    assert!(matches!(
        deliver_candidate_blocking(&f.state, HoldingPlanCandidate::Fresh(prepared)).unwrap(),
        HoldingPlanDispatchResult::PhysicalAccepted
    ));
    let before = f.counts();
    let result = prepare_tick_with(
        None,
        || Ok(Some(snapshot(&["920001"]))),
        |date, instrument| {
            assert_eq!(instrument.exchange(), Exchange::Beijing);
            f.state
                .coordinator
                .inspect_holding_plan_occurrence(date, instrument)
                .map_err(|e| e.to_string())
        },
        |_| panic!("Beijing actual owner has no legacy lookup"),
        |_| panic!("Beijing actual owner must not re-quote"),
        local_now(),
    )
    .unwrap();
    assert!(result.failures.is_empty());
    assert_eq!(result.candidates.len(), 1);
    let candidate = result.candidates.into_iter().next().unwrap();
    match &candidate {
        HoldingPlanCandidate::Original { owned, .. } => {
            assert_eq!(owned.envelope().scope_key, "BEIJING:EQUITY:920001")
        }
        _ => panic!("must recover original Beijing owner"),
    }
    assert!(matches!(
        reconcile_candidate_blocking(&f.state, candidate).unwrap(),
        HoldingPlanLocalProgress::PhysicalAccepted
    ));
    let wrong = InstrumentId::new(Exchange::Shenzhen, "920001", AssetClass::Equity).unwrap();
    assert_eq!(
        f.state
            .coordinator
            .inspect_holding_plan_occurrence(local_now().date_naive(), &wrong)
            .unwrap(),
        HoldingPlanOccurrenceObservation::Missing
    );
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.counts(), before);
}

#[test]
fn t03_exact_owner_bin_alias_b_share_unknown_and_test_codes_have_zero_new_admission() {
    let f = Fixture::new(false);
    let before = f.counts();
    let rejected = [
        "430001",
        "830001",
        "870001",
        "880001",
        "900901",
        "700001",
        "TEST_CODE_600000",
        "92001",
    ];
    let result = prepare_tick_with(
        Some(&crate::push_templates::BannerCtx::test_default()),
        || Ok(Some(snapshot(&rejected))),
        |_, _| panic!("unresolved equity cannot inspect/mint another scope"),
        |_| panic!("unresolved equity cannot consult fresh legacy eligibility"),
        |_| panic!("unresolved equity cannot quote"),
        local_now(),
    )
    .unwrap();
    assert!(result.candidates.is_empty());
    assert_eq!(result.failures.len(), rejected.len());
    assert_eq!(f.counts(), before);
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    for code in rejected {
        let result = crate::holding_plan::prepare_holding_plan_messages_with(
            &crate::push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot(&[code]))),
            |_| panic!("test renderer seam must use same production resolver before quote"),
            local_now,
        );
        assert!(result.is_err(), "{code}");
    }
    assert_eq!(f.counts(), before);
}
