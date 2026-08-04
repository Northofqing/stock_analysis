use std::sync::{Arc, Mutex};
use stock_analysis::news::aggregator::raw_v2::{
    registered_global_news_feeds, REGISTERED_GLOBAL_NEWS_LIMIT,
};
use stock_analysis::selection::acquisition_v2::{
    build_generation_acquisition_cadence_receipt, build_generation_acquisition_uncertainty_record,
    freeze_feed_plan_at_activation, freeze_registered_global_news_feed_plan_at_activation,
    parse_generation_acquisition_cadence_receipt, parse_generation_acquisition_uncertainty_record,
    recover_prior_boot_unsealed_intent, registered_feed_descriptor_hash,
    run_serial_ingress_acquisition, verify_feed_acquisition_response,
    verify_feed_plan_before_cadence, verify_ingress_resolution_prefix, AcquisitionModeNamespace,
    CanonicalCarrier, FeedAcquisitionOutcomeKind, FeedAcquisitionResolution, FeedPlanActivation,
    IngressAggregateOutcomeKind, IngressCycleTerminalKind, OwnedCanonicalCarrier,
    OwnedFeedAcquisitionEvidence, PriorBootUnsealedFeedIntent, ResolvedFeedEvidence,
    SelectionFailedNonRetryableCode, SelectionPendingDependencyCode, SerialIngressJournal,
    SerialIngressProvider, VerifiedCadenceFeedPlan, VerifiedFeedIntent,
    VerifiedIngressResolutionPrefix,
};
use stock_analysis::selection::activation_runtime::SelectionDisabledReason;
use stock_analysis::selection::schema_v2::sha256_bytes;

fn present(bytes: &'static [u8]) -> CanonicalCarrier<'static> {
    CanonicalCarrier::present(bytes, sha256_bytes(bytes))
}

#[test]
fn br193_cadence_receipt_exact_bytes_hash_and_restart_window_are_closed() {
    let namespace =
        AcquisitionModeNamespace::for_test_code("TEST_CODE_selection_v2_scheduler").unwrap();
    let receipt = build_generation_acquisition_cadence_receipt(
        "018f8f3e-7b2a-7abc-8def-1234567890a1",
        &namespace,
        "TEST_CODE_activation_run",
        "a".repeat(64),
        "018f8f3e-7b2a-7abc-8def-1234567890a2",
        "2026-07-31T01:02:03.123456789Z",
        None,
        "018f8f3e-7b2a-7abc-8def-1234567890a3",
        "2026-07-31T01:02:03.223456789Z",
    )
    .expect("valid first cadence receipt");

    let json = std::str::from_utf8(receipt.canonical_bytes()).unwrap();
    let fields = [
        "\"domain\"",
        "\"schema_version\"",
        "\"cadence_receipt_id\"",
        "\"mode_namespace\"",
        "\"activation_run_id\"",
        "\"activation_receipt_hash\"",
        "\"scheduler_cycle_id\"",
        "\"acquisition_started_at\"",
        "\"next_acquisition_eligible_at\"",
        "\"prior_cadence_receipt_hash\"",
        "\"boot_instance_id\"",
        "\"committed_at\"",
    ];
    let positions = fields
        .iter()
        .map(|field| json.find(field).expect("exact cadence field"))
        .collect::<Vec<_>>();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "cadence fields must retain the frozen logical order"
    );
    assert_eq!(
        receipt.next_acquisition_eligible_at(),
        "2026-07-31T01:04:03.123456789Z"
    );

    let parsed = parse_generation_acquisition_cadence_receipt(
        receipt.canonical_bytes(),
        receipt.content_hash(),
    )
    .expect("exact synced bytes/hash read back");
    assert_eq!(parsed, receipt);

    let mut wrong_window: serde_json::Value =
        serde_json::from_slice(receipt.canonical_bytes()).unwrap();
    wrong_window["next_acquisition_eligible_at"] =
        serde_json::json!("2026-07-31T01:04:02.123456789Z");
    let wrong_window = serde_json::to_vec(&wrong_window).unwrap();
    assert!(
        parse_generation_acquisition_cadence_receipt(&wrong_window, &sha256_bytes(&wrong_window))
            .is_err(),
        "next eligibility must be exactly acquisition start plus 120 seconds"
    );
    assert!(
        parse_generation_acquisition_cadence_receipt(receipt.canonical_bytes(), &"0".repeat(64))
            .is_err(),
        "readback hash must bind the exact cadence bytes"
    );
}

#[test]
fn br193_ingress_uncertain_resolution_closes_stopped_prefix_and_suffix() {
    const ATTEMPTS: &[u8] = br#"[{"attempt_id":"TEST_CODE_attempt_1"}]"#;
    const SUCCESS: &[u8] = br#"[{"record_id":"TEST_CODE_record_1"}]"#;
    const EMPTY: &[u8] = b"[]";

    let success = verify_feed_acquisition_response(
        FeedAcquisitionOutcomeKind::SuccessNonempty,
        present(SUCCESS),
        CanonicalCarrier::absent(),
        present(ATTEMPTS),
    )
    .expect("success seal");
    let empty = verify_feed_acquisition_response(
        FeedAcquisitionOutcomeKind::VerifiedEmpty,
        present(EMPTY),
        CanonicalCarrier::absent(),
        present(ATTEMPTS),
    )
    .expect("empty seal");
    let prefix = vec![
        ResolvedFeedEvidence::sealed("1".repeat(64), "a".repeat(64), success)
            .expect("sealed success"),
        ResolvedFeedEvidence::sealed("2".repeat(64), "b".repeat(64), empty).expect("sealed empty"),
        ResolvedFeedEvidence::uncertain("3".repeat(64), "c".repeat(64))
            .expect("uncertain final resolution"),
    ];

    let verified = verify_ingress_resolution_prefix(4, prefix)
        .expect("one final uncertainty closes a contiguous stopped prefix");
    assert_eq!(
        verified.aggregate_outcome_kind(),
        IngressAggregateOutcomeKind::PendingDependency
    );
    assert_eq!(
        verified.terminal_kind(),
        IngressCycleTerminalKind::PendingDependency
    );
    assert_eq!(
        verified.pending_dependency_code(),
        Some(SelectionPendingDependencyCode::AcquisitionOutcomeUncertain)
    );
    assert_eq!(verified.resolved_feed_count(), 3);
    assert_eq!(verified.uncontacted_suffix_count(), 1);
    assert_eq!(verified.stopped_after_feed_ordinal(), Some(2));
    assert_eq!(verified.verified_empty_feed_count(), 1);
    assert_eq!(verified.total_response_record_count(), 1);

    let nonfinal_uncertain = vec![
        ResolvedFeedEvidence::uncertain("4".repeat(64), "d".repeat(64)).unwrap(),
        ResolvedFeedEvidence::sealed(
            "5".repeat(64),
            "e".repeat(64),
            verify_feed_acquisition_response(
                FeedAcquisitionOutcomeKind::VerifiedEmpty,
                present(EMPTY),
                CanonicalCarrier::absent(),
                present(ATTEMPTS),
            )
            .unwrap(),
        )
        .unwrap(),
    ];
    assert!(
        verify_ingress_resolution_prefix(4, nonfinal_uncertain).is_err(),
        "Uncertain must be the one final resolved feed"
    );

    let partial_normal_prefix = vec![ResolvedFeedEvidence::sealed(
        "6".repeat(64),
        "f".repeat(64),
        verify_feed_acquisition_response(
            FeedAcquisitionOutcomeKind::VerifiedEmpty,
            present(EMPTY),
            CanonicalCarrier::absent(),
            present(ATTEMPTS),
        )
        .unwrap(),
    )
    .unwrap()];
    assert!(
        verify_ingress_resolution_prefix(4, partial_normal_prefix).is_err(),
        "a normal sealed prefix cannot fabricate an early cycle terminal"
    );
}

#[test]
fn br193_feed_resolution_union_rejects_invalid_hashes_and_cross_fields() {
    let sealed = format!(
        r#"{{"kind":"sealed","intent_hash":"{}","seal_hash":"{}"}}"#,
        "1".repeat(64),
        "a".repeat(64)
    );
    let uncertain = format!(
        r#"{{"kind":"uncertain","intent_hash":"{}","uncertainty_record_hash":"{}"}}"#,
        "2".repeat(64),
        "b".repeat(64)
    );
    let sealed_value: FeedAcquisitionResolution = serde_json::from_str(&sealed).unwrap();
    let uncertain_value: FeedAcquisitionResolution = serde_json::from_str(&uncertain).unwrap();
    assert_eq!(serde_json::to_string(&sealed_value).unwrap(), sealed);
    assert_eq!(serde_json::to_string(&uncertain_value).unwrap(), uncertain);

    for rejected in [
        format!(
            r#"{{"kind":"sealed","intent_hash":"{}","seal_hash":"{}"}}"#,
            "1".repeat(64),
            "A".repeat(64)
        ),
        format!(
            r#"{{"kind":"uncertain","intent_hash":"{}","seal_hash":"{}"}}"#,
            "2".repeat(64),
            "b".repeat(64)
        ),
        format!(
            r#"{{"kind":"uncertain","intent_hash":"{}","uncertainty_record_hash":"{}","unknown":true}}"#,
            "2".repeat(64),
            "b".repeat(64)
        ),
        format!(
            r#"{{"kind":"legacy","intent_hash":"{}","seal_hash":"{}"}}"#,
            "1".repeat(64),
            "a".repeat(64)
        ),
    ] {
        assert!(
            serde_json::from_str::<FeedAcquisitionResolution>(&rejected).is_err(),
            "invalid resolution carrier unexpectedly decoded: {rejected}"
        );
    }
}

#[test]
fn br193_feed_outcome_enum_and_response_error_matrix_are_closed() {
    const ATTEMPTS: &[u8] = br#"[{"attempt_id":"TEST_CODE_attempt_1"}]"#;
    const SUCCESS: &[u8] = br#"[{"record_id":"TEST_CODE_record_1"}]"#;
    const EMPTY: &[u8] = b"[]";
    const OVER_LIMIT: &[u8] = b"[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20]";
    const TRANSPORT_ERROR: &[u8] = br#"{"domain":"stock_analysis.selection_v2_generation_feed_error.v1","schema_version":1,"code":"feed_unavailable","redacted_detail_sha256_or_null":null,"retryable":true}"#;
    const CANCELLED_ERROR: &[u8] = br#"{"domain":"stock_analysis.selection_v2_generation_feed_error.v1","schema_version":1,"code":"provider_cancelled","redacted_detail_sha256_or_null":null,"retryable":true}"#;
    const OVER_LIMIT_ERROR: &[u8] = br#"{"domain":"stock_analysis.selection_v2_generation_feed_error.v1","schema_version":1,"code":"feed_response_limit_exceeded","redacted_detail_sha256_or_null":null,"retryable":false}"#;

    let rows = [
        (
            FeedAcquisitionOutcomeKind::SuccessNonempty,
            present(SUCCESS),
            CanonicalCarrier::absent(),
            1,
        ),
        (
            FeedAcquisitionOutcomeKind::VerifiedEmpty,
            present(EMPTY),
            CanonicalCarrier::absent(),
            0,
        ),
        (
            FeedAcquisitionOutcomeKind::TransportFailure,
            CanonicalCarrier::absent(),
            present(TRANSPORT_ERROR),
            0,
        ),
        (
            FeedAcquisitionOutcomeKind::ProviderCancelled,
            CanonicalCarrier::absent(),
            present(CANCELLED_ERROR),
            0,
        ),
        (
            FeedAcquisitionOutcomeKind::FeedResponseLimitExceeded,
            present(OVER_LIMIT),
            present(OVER_LIMIT_ERROR),
            21,
        ),
    ];

    for (outcome, response, typed_error, expected_count) in rows {
        let verified =
            verify_feed_acquisition_response(outcome, response, typed_error, present(ATTEMPTS))
                .expect("the exact matrix row must verify");
        assert_eq!(verified.sealed_response_record_count(), expected_count);
    }

    for token in [
        "\"success_nonempty\"",
        "\"verified_empty\"",
        "\"transport_failure\"",
        "\"provider_cancelled\"",
        "\"feed_response_limit_exceeded\"",
    ] {
        serde_json::from_str::<FeedAcquisitionOutcomeKind>(token)
            .expect("one exact closed token must decode");
    }
    for rejected in [
        "\"SuccessNonempty\"",
        "\"response_limit_exceeded\"",
        "\"unknown\"",
        "null",
    ] {
        assert!(
            serde_json::from_str::<FeedAcquisitionOutcomeKind>(rejected).is_err(),
            "legacy/unknown outcome unexpectedly decoded: {rejected}"
        );
    }

    assert!(
        verify_feed_acquisition_response(
            FeedAcquisitionOutcomeKind::SuccessNonempty,
            present(EMPTY),
            CanonicalCarrier::absent(),
            present(ATTEMPTS),
        )
        .is_err(),
        "success_nonempty cannot seal zero records"
    );
    assert!(
        verify_feed_acquisition_response(
            FeedAcquisitionOutcomeKind::VerifiedEmpty,
            present(SUCCESS),
            CanonicalCarrier::absent(),
            present(ATTEMPTS),
        )
        .is_err(),
        "verified_empty cannot carry records"
    );
    assert!(
        verify_feed_acquisition_response(
            FeedAcquisitionOutcomeKind::SuccessNonempty,
            present(OVER_LIMIT),
            CanonicalCarrier::absent(),
            present(ATTEMPTS),
        )
        .is_err(),
        "21 records cannot be truncated into normal success"
    );
    assert!(
        verify_feed_acquisition_response(
            FeedAcquisitionOutcomeKind::TransportFailure,
            present(SUCCESS),
            present(TRANSPORT_ERROR),
            present(ATTEMPTS),
        )
        .is_err(),
        "transport failure cannot carry response bytes"
    );
    assert!(
        verify_feed_acquisition_response(
            FeedAcquisitionOutcomeKind::TransportFailure,
            CanonicalCarrier::absent(),
            present(CANCELLED_ERROR),
            present(ATTEMPTS),
        )
        .is_err(),
        "error code/retryability must match the outcome"
    );
    assert!(
        verify_feed_acquisition_response(
            FeedAcquisitionOutcomeKind::VerifiedEmpty,
            present(EMPTY),
            CanonicalCarrier::absent(),
            CanonicalCarrier::absent(),
        )
        .is_err(),
        "ordered attempt evidence is mandatory"
    );
    assert!(
        verify_feed_acquisition_response(
            FeedAcquisitionOutcomeKind::VerifiedEmpty,
            CanonicalCarrier::present(EMPTY, "0".repeat(64)),
            CanonicalCarrier::absent(),
            present(ATTEMPTS),
        )
        .is_err(),
        "carrier bytes/hash asymmetry must reject"
    );
}

#[test]
fn br193_nonempty_feed_plan_is_required_at_activation_and_tick() {
    let disabled = freeze_feed_plan_at_activation(Vec::new())
        .expect("an empty plan is a typed disabled activation, not success");
    assert_eq!(
        disabled,
        FeedPlanActivation::Disabled(SelectionDisabledReason::IngressContractUnavailable)
    );

    let descriptor_hashes = vec!["1".repeat(64), "2".repeat(64)];
    let FeedPlanActivation::Active(frozen) =
        freeze_feed_plan_at_activation(descriptor_hashes.clone()).expect("valid active plan")
    else {
        panic!("nonempty valid plan must activate");
    };
    assert_eq!(frozen.feed_count(), 2);

    let cadence = verify_feed_plan_before_cadence(&frozen, &descriptor_hashes)
        .expect("the byte-identical nonempty plan remains active");
    assert_eq!(cadence.feed_count(), 2);
    assert_eq!(cadence.plan_hash(), frozen.plan_hash());

    for drifted in [
        Vec::new(),
        vec!["1".repeat(64)],
        vec!["2".repeat(64), "1".repeat(64)],
        vec!["1".repeat(64), "3".repeat(64)],
    ] {
        let error = verify_feed_plan_before_cadence(&frozen, &drifted)
            .expect_err("empty/count/order/identity drift must fail before cadence");
        assert_eq!(error.code(), "config_snapshot_conflict");
    }
}

#[test]
fn br193_registered_feed_plan_preserves_registration_order_and_limit() {
    let registrations = registered_global_news_feeds();
    let frozen =
        freeze_registered_global_news_feed_plan_at_activation().expect("frozen production plan");
    assert_eq!(frozen.feed_count(), registrations.len());
    assert_eq!(REGISTERED_GLOBAL_NEWS_LIMIT as usize, 20);

    let expected = registrations
        .iter()
        .map(registered_feed_descriptor_hash)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        frozen.descriptor_hashes_in_registration_order(),
        expected,
        "provider identity sorting must not replace frozen registration order"
    );

    let mut reordered = expected;
    reordered.reverse();
    let FeedPlanActivation::Active(reordered) = freeze_feed_plan_at_activation(reordered).unwrap()
    else {
        panic!("the reordered fixture is still nonempty");
    };
    assert_ne!(reordered.plan_hash(), frozen.plan_hash());
}

#[test]
fn br193_ingress_aggregate_mapping_and_counters_are_recomputed() {
    const ATTEMPTS: &[u8] = br#"[{"attempt_id":"TEST_CODE_attempt_1"}]"#;
    const SUCCESS: &[u8] = br#"[{"record_id":"TEST_CODE_record_1"}]"#;
    const EMPTY: &[u8] = b"[]";
    const OVER_LIMIT: &[u8] = b"[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20]";
    const TRANSPORT_ERROR: &[u8] = br#"{"domain":"stock_analysis.selection_v2_generation_feed_error.v1","schema_version":1,"code":"feed_unavailable","redacted_detail_sha256_or_null":null,"retryable":true}"#;
    const CANCELLED_ERROR: &[u8] = br#"{"domain":"stock_analysis.selection_v2_generation_feed_error.v1","schema_version":1,"code":"provider_cancelled","redacted_detail_sha256_or_null":null,"retryable":true}"#;
    const OVER_LIMIT_ERROR: &[u8] = br#"{"domain":"stock_analysis.selection_v2_generation_feed_error.v1","schema_version":1,"code":"feed_response_limit_exceeded","redacted_detail_sha256_or_null":null,"retryable":false}"#;

    fn sealed(
        ordinal: usize,
        outcome: FeedAcquisitionOutcomeKind,
        response: CanonicalCarrier<'static>,
        error: CanonicalCarrier<'static>,
    ) -> ResolvedFeedEvidence {
        let verified =
            verify_feed_acquisition_response(outcome, response, error, present(ATTEMPTS)).unwrap();
        ResolvedFeedEvidence::sealed(
            format!("{ordinal:064x}"),
            format!("{:064x}", ordinal + 100),
            verified,
        )
        .unwrap()
    }

    let all_empty = verify_ingress_resolution_prefix(
        2,
        vec![
            sealed(
                1,
                FeedAcquisitionOutcomeKind::VerifiedEmpty,
                present(EMPTY),
                CanonicalCarrier::absent(),
            ),
            sealed(
                2,
                FeedAcquisitionOutcomeKind::VerifiedEmpty,
                present(EMPTY),
                CanonicalCarrier::absent(),
            ),
        ],
    )
    .unwrap();
    assert_eq!(
        all_empty.aggregate_outcome_kind(),
        IngressAggregateOutcomeKind::VerifiedEmpty
    );
    assert_eq!(
        all_empty.terminal_kind(),
        IngressCycleTerminalKind::VerifiedEmpty
    );
    assert_eq!(all_empty.verified_empty_feed_count(), 2);
    assert_eq!(all_empty.total_response_record_count(), 0);

    let mixed = verify_ingress_resolution_prefix(
        2,
        vec![
            sealed(
                3,
                FeedAcquisitionOutcomeKind::VerifiedEmpty,
                present(EMPTY),
                CanonicalCarrier::absent(),
            ),
            sealed(
                4,
                FeedAcquisitionOutcomeKind::SuccessNonempty,
                present(SUCCESS),
                CanonicalCarrier::absent(),
            ),
        ],
    )
    .unwrap();
    assert_eq!(
        mixed.aggregate_outcome_kind(),
        IngressAggregateOutcomeKind::SuccessNonempty
    );
    assert_eq!(
        mixed.terminal_kind(),
        IngressCycleTerminalKind::SourceIngressCommitted
    );
    assert_eq!(mixed.verified_empty_feed_count(), 1);
    assert_eq!(mixed.total_response_record_count(), 1);

    for (outcome, error, expected_code) in [
        (
            FeedAcquisitionOutcomeKind::TransportFailure,
            present(TRANSPORT_ERROR),
            SelectionPendingDependencyCode::FeedUnavailable,
        ),
        (
            FeedAcquisitionOutcomeKind::ProviderCancelled,
            present(CANCELLED_ERROR),
            SelectionPendingDependencyCode::ProviderCancelled,
        ),
    ] {
        let stopped = verify_ingress_resolution_prefix(
            3,
            vec![
                sealed(
                    5,
                    FeedAcquisitionOutcomeKind::VerifiedEmpty,
                    present(EMPTY),
                    CanonicalCarrier::absent(),
                ),
                sealed(6, outcome, CanonicalCarrier::absent(), error),
            ],
        )
        .unwrap();
        assert_eq!(
            stopped.aggregate_outcome_kind(),
            IngressAggregateOutcomeKind::PendingDependency
        );
        assert_eq!(stopped.pending_dependency_code(), Some(expected_code));
        assert_eq!(stopped.resolved_feed_count(), 2);
        assert_eq!(stopped.uncontacted_suffix_count(), 1);
        assert_eq!(stopped.stopped_after_feed_ordinal(), Some(1));
        assert_eq!(stopped.verified_empty_feed_count(), 1);
        assert_eq!(stopped.total_response_record_count(), 0);
    }

    let over_limit = verify_ingress_resolution_prefix(
        1,
        vec![sealed(
            7,
            FeedAcquisitionOutcomeKind::FeedResponseLimitExceeded,
            present(OVER_LIMIT),
            present(OVER_LIMIT_ERROR),
        )],
    )
    .unwrap();
    assert_eq!(
        over_limit.aggregate_outcome_kind(),
        IngressAggregateOutcomeKind::FailedNonRetryable
    );
    assert_eq!(
        over_limit.failed_non_retryable_code(),
        Some(SelectionFailedNonRetryableCode::FeedResponseLimitExceeded)
    );
    assert_eq!(over_limit.total_response_record_count(), 21);
    assert_eq!(over_limit.stopped_after_feed_ordinal(), Some(0));
}

#[derive(Clone)]
struct SerialTrace(Arc<Mutex<Vec<String>>>);

impl SerialTrace {
    fn push(&self, value: impl Into<String>) {
        self.0.lock().unwrap().push(value.into());
    }

    fn snapshot(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

struct SpyJournal {
    trace: SerialTrace,
}

#[async_trait::async_trait]
impl SerialIngressJournal for SpyJournal {
    async fn append_sync_read_back_plan_intent(
        &mut self,
        _plan: &VerifiedCadenceFeedPlan,
    ) -> Result<String, stock_analysis::selection::acquisition_v2::AcquisitionV2Error> {
        self.trace.push("plan_intent");
        Ok("a".repeat(64))
    }

    async fn append_sync_read_back_feed_intent(
        &mut self,
        _plan_intent_hash: &str,
        ordinal: usize,
        _descriptor_hash: &str,
    ) -> Result<String, stock_analysis::selection::acquisition_v2::AcquisitionV2Error> {
        self.trace.push(format!("feed_intent:{ordinal}"));
        Ok(format!("{:064x}", ordinal + 1))
    }

    async fn append_sync_read_back_feed_seal(
        &mut self,
        _intent: &VerifiedFeedIntent,
        _evidence: &OwnedFeedAcquisitionEvidence,
    ) -> Result<String, stock_analysis::selection::acquisition_v2::AcquisitionV2Error> {
        self.trace
            .push(format!("feed_seal:{}", _intent.feed_ordinal()));
        Ok(format!("{:064x}", _intent.feed_ordinal() + 101))
    }

    async fn append_sync_read_back_uncertainty(
        &mut self,
        intent: &PriorBootUnsealedFeedIntent,
    ) -> Result<
        stock_analysis::selection::acquisition_v2::VerifiedAcquisitionUncertaintyRecord,
        stock_analysis::selection::acquisition_v2::AcquisitionV2Error,
    > {
        self.trace
            .push(format!("uncertainty:{}", intent.feed_ordinal()));
        build_generation_acquisition_uncertainty_record(
            "018f8f3e-7b2a-7abc-8def-1234567890ad",
            intent,
            "018f8f3e-7b2a-7abc-8def-1234567890ae",
            "2026-07-31T01:02:03.123456789Z",
        )
    }

    async fn append_sync_read_back_cycle_terminal(
        &mut self,
        _plan_intent_hash: &str,
        prefix: &VerifiedIngressResolutionPrefix,
    ) -> Result<String, stock_analysis::selection::acquisition_v2::AcquisitionV2Error> {
        self.trace
            .push(format!("cycle_terminal:{}", prefix.resolved_feed_count()));
        Ok("f".repeat(64))
    }
}

struct SpyProvider {
    trace: SerialTrace,
    outcomes: Vec<FeedAcquisitionOutcomeKind>,
}

#[async_trait::async_trait]
impl SerialIngressProvider for SpyProvider {
    async fn fetch_after_intent_read_back(
        &mut self,
        intent: &VerifiedFeedIntent,
    ) -> Result<
        OwnedFeedAcquisitionEvidence,
        stock_analysis::selection::acquisition_v2::AcquisitionV2Error,
    > {
        let ordinal = intent.feed_ordinal();
        self.trace.push(format!("provider:{ordinal}"));
        let attempts =
            OwnedCanonicalCarrier::present(br#"[{"attempt_id":"TEST_CODE_attempt_1"}]"#.to_vec());
        let evidence = match self.outcomes[ordinal] {
            FeedAcquisitionOutcomeKind::SuccessNonempty => OwnedFeedAcquisitionEvidence::new(
                FeedAcquisitionOutcomeKind::SuccessNonempty,
                OwnedCanonicalCarrier::present(
                    br#"[{"record_id":"TEST_CODE_record_1"}]"#.to_vec(),
                ),
                OwnedCanonicalCarrier::absent(),
                attempts,
            ),
            FeedAcquisitionOutcomeKind::VerifiedEmpty => OwnedFeedAcquisitionEvidence::new(
                FeedAcquisitionOutcomeKind::VerifiedEmpty,
                OwnedCanonicalCarrier::present(b"[]".to_vec()),
                OwnedCanonicalCarrier::absent(),
                attempts,
            ),
            FeedAcquisitionOutcomeKind::TransportFailure => {
                OwnedFeedAcquisitionEvidence::new(
                    FeedAcquisitionOutcomeKind::TransportFailure,
                    OwnedCanonicalCarrier::absent(),
                    OwnedCanonicalCarrier::present(
                        br#"{"domain":"stock_analysis.selection_v2_generation_feed_error.v1","schema_version":1,"code":"feed_unavailable","redacted_detail_sha256_or_null":null,"retryable":true}"#
                            .to_vec(),
                    ),
                    attempts,
                )
            }
            _ => panic!("fixture only needs success/empty/transport"),
        };
        Ok(evidence)
    }
}

#[tokio::test]
async fn br193_feed_acquisition_is_serial_intent_response_seal() {
    let descriptors = vec!["1".repeat(64), "2".repeat(64), "3".repeat(64)];
    let FeedPlanActivation::Active(frozen) =
        freeze_feed_plan_at_activation(descriptors.clone()).unwrap()
    else {
        panic!("fixture plan must activate");
    };
    let plan = verify_feed_plan_before_cadence(&frozen, &descriptors).unwrap();
    let trace = SerialTrace(Arc::new(Mutex::new(Vec::new())));
    let mut journal = SpyJournal {
        trace: trace.clone(),
    };
    let mut provider = SpyProvider {
        trace: trace.clone(),
        outcomes: vec![
            FeedAcquisitionOutcomeKind::SuccessNonempty,
            FeedAcquisitionOutcomeKind::VerifiedEmpty,
            FeedAcquisitionOutcomeKind::SuccessNonempty,
        ],
    };

    let completed =
        run_serial_ingress_acquisition(&plan, &descriptors, &mut journal, &mut provider)
            .await
            .unwrap();
    assert_eq!(completed.prefix().resolved_feed_count(), 3);
    assert_eq!(completed.terminal_receipt_hash(), "f".repeat(64));
    assert_eq!(
        trace.snapshot(),
        [
            "plan_intent",
            "feed_intent:0",
            "provider:0",
            "feed_seal:0",
            "feed_intent:1",
            "provider:1",
            "feed_seal:1",
            "feed_intent:2",
            "provider:2",
            "feed_seal:2",
            "cycle_terminal:3",
        ]
    );

    let stop_trace = SerialTrace(Arc::new(Mutex::new(Vec::new())));
    let mut stop_journal = SpyJournal {
        trace: stop_trace.clone(),
    };
    let mut stop_provider = SpyProvider {
        trace: stop_trace.clone(),
        outcomes: vec![
            FeedAcquisitionOutcomeKind::SuccessNonempty,
            FeedAcquisitionOutcomeKind::TransportFailure,
            FeedAcquisitionOutcomeKind::SuccessNonempty,
        ],
    };
    let stopped =
        run_serial_ingress_acquisition(&plan, &descriptors, &mut stop_journal, &mut stop_provider)
            .await
            .unwrap();
    assert_eq!(
        stopped.prefix().pending_dependency_code(),
        Some(SelectionPendingDependencyCode::FeedUnavailable)
    );
    assert_eq!(stopped.prefix().uncontacted_suffix_count(), 1);
    assert_eq!(
        stop_trace.snapshot(),
        [
            "plan_intent",
            "feed_intent:0",
            "provider:0",
            "feed_seal:0",
            "feed_intent:1",
            "provider:1",
            "feed_seal:1",
            "cycle_terminal:2",
        ],
        "the uncontacted suffix must have no intent, provider future or seal"
    );
}

#[tokio::test]
async fn br193_prior_boot_unsealed_intent_is_uncertain_without_reissue() {
    let descriptors = vec!["1".repeat(64), "2".repeat(64), "3".repeat(64)];
    let FeedPlanActivation::Active(frozen) =
        freeze_feed_plan_at_activation(descriptors.clone()).unwrap()
    else {
        panic!("fixture plan must activate");
    };
    let plan = verify_feed_plan_before_cadence(&frozen, &descriptors).unwrap();
    let sealed_success = verify_feed_acquisition_response(
        FeedAcquisitionOutcomeKind::SuccessNonempty,
        present(br#"[{"record_id":"TEST_CODE_record_1"}]"#),
        CanonicalCarrier::absent(),
        present(br#"[{"attempt_id":"TEST_CODE_attempt_1"}]"#),
    )
    .unwrap();
    let sealed_prefix =
        vec![ResolvedFeedEvidence::sealed("1".repeat(64), "a".repeat(64), sealed_success).unwrap()];
    let prior_boot_intent = PriorBootUnsealedFeedIntent::new(
        "018f8f3e-7b2a-7abc-8def-1234567890ac",
        "2".repeat(64),
        1,
        descriptors[1].clone(),
        "018f8f3e-7b2a-7abc-8def-1234567890ab",
    )
    .unwrap();
    let trace = SerialTrace(Arc::new(Mutex::new(Vec::new())));
    let mut journal = SpyJournal {
        trace: trace.clone(),
    };

    let recovered = recover_prior_boot_unsealed_intent(
        &plan,
        &descriptors,
        "9".repeat(64),
        sealed_prefix,
        prior_boot_intent,
        &mut journal,
    )
    .await
    .unwrap();
    assert_eq!(
        recovered.prefix().pending_dependency_code(),
        Some(SelectionPendingDependencyCode::AcquisitionOutcomeUncertain)
    );
    assert_eq!(recovered.prefix().resolved_feed_count(), 2);
    assert_eq!(recovered.prefix().uncontacted_suffix_count(), 1);
    assert_eq!(
        trace.snapshot(),
        ["uncertainty:1", "cycle_terminal:2"],
        "recovery must not reissue the prior intent or contact its provider/suffix"
    );
}

#[test]
fn br193_uncertainty_record_hash_and_closed_carrier_are_strict() {
    let prior = PriorBootUnsealedFeedIntent::new(
        "018f8f3e-7b2a-7abc-8def-1234567890ac",
        "2".repeat(64),
        1,
        "2".repeat(64),
        "018f8f3e-7b2a-7abc-8def-1234567890ab",
    )
    .unwrap();
    let record = build_generation_acquisition_uncertainty_record(
        "018f8f3e-7b2a-7abc-8def-1234567890ad",
        &prior,
        "018f8f3e-7b2a-7abc-8def-1234567890ae",
        "2026-07-31T01:02:03.123456789Z",
    )
    .unwrap();
    let parsed = parse_generation_acquisition_uncertainty_record(record.canonical_bytes()).unwrap();
    assert_eq!(
        parsed.uncertainty_record_hash(),
        record.uncertainty_record_hash()
    );
    assert_eq!(parsed.intent_hash(), prior.intent_hash());

    let canonical: serde_json::Value = serde_json::from_slice(record.canonical_bytes()).unwrap();
    for mutated in [
        {
            let mut value = canonical.clone();
            value["preimage"]["reason_code"] = serde_json::json!("provider_failed");
            serde_json::to_vec(&value).unwrap()
        },
        {
            let mut value = canonical.clone();
            value["uncertainty_record_hash"] = serde_json::json!("0".repeat(64));
            serde_json::to_vec(&value).unwrap()
        },
        {
            let mut value = canonical.clone();
            value["preimage"]["unknown"] = serde_json::json!(true);
            serde_json::to_vec(&value).unwrap()
        },
    ] {
        assert!(
            parse_generation_acquisition_uncertainty_record(&mutated).is_err(),
            "reason/hash/unknown-field mutation must reject"
        );
    }
}
