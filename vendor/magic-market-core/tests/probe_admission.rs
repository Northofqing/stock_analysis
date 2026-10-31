use magic_market_core::{
    verify_admitted_batch, verify_verified_empty, DataBatch, EvidenceTimestamp,
    ProbeAdmissionError, ProbeAdmissionPolicy, ProbeStatus, Provenance, ProviderId, SourceEvidence,
    VerifiedEmpty,
};
use std::time::Duration;

#[derive(Debug)]
struct Record {
    identity: &'static str,
    evidence: SourceEvidence,
}

fn evidence(
    provider: ProviderId,
    observed_at: &str,
    source_at: &str,
    batch_id: &str,
) -> SourceEvidence {
    SourceEvidence::new(provider, observed_at, batch_id)
        .unwrap()
        .with_source_at(source_at)
        .unwrap()
}

fn provenance(observed_at: &str, source_at: &str, batch_id: &str) -> Provenance {
    Provenance::new("fixture", observed_at)
        .unwrap()
        .with_source_at(source_at)
        .unwrap()
        .with_batch_id(batch_id)
        .unwrap()
}

fn policy() -> ProbeAdmissionPolicy {
    ProbeAdmissionPolicy::new(ProviderId::Tonghuashun)
        .require_source_at()
        .with_max_source_age(Duration::from_secs(3_600))
        .unwrap()
}

#[test]
fn evidence_timestamps_preserve_nanoseconds_and_reject_excess_precision() {
    let source = EvidenceTimestamp::parse_instant("2026-07-23T10:00:00Z").unwrap();
    let exact = EvidenceTimestamp::parse_instant("2026-07-23T10:00:05Z").unwrap();
    let over = EvidenceTimestamp::parse_instant("2026-07-23T10:00:05.000000001Z").unwrap();
    let epoch_over = EvidenceTimestamp::parse_instant("1784786405.000000001").unwrap();
    let epoch_source = EvidenceTimestamp::parse_instant("1784786400").unwrap();

    assert_eq!(exact.duration_since(source), Some(Duration::from_secs(5)));
    assert_eq!(
        over.duration_since(source),
        Some(Duration::from_secs(5) + Duration::from_nanos(1))
    );
    assert_eq!(
        epoch_over.duration_since(epoch_source),
        Some(Duration::from_secs(5) + Duration::from_nanos(1))
    );
    for invalid in ["2026-07-23T10:00:05.0000000001Z", "1784786405.0000000001"] {
        assert!(
            EvidenceTimestamp::parse_instant(invalid).is_err(),
            "excess timestamp precision was silently accepted: {invalid}"
        );
    }
}

#[test]
fn maximum_source_age_requires_an_unambiguous_source_instant() {
    let policy = ProbeAdmissionPolicy::new(ProviderId::Eastmoney)
        .with_max_source_age(Duration::from_secs(5))
        .unwrap();
    let batch_id = "eastmoney:strict-instant";

    let evidence_without_source =
        SourceEvidence::new(ProviderId::Eastmoney, "2026-07-23T10:00:00Z", batch_id).unwrap();
    let provenance_without_source = Provenance::new("fixture", "2026-07-23T10:00:00Z")
        .unwrap()
        .with_batch_id(batch_id)
        .unwrap();
    let missing = DataBatch::strict(
        vec![Record {
            identity: "AP-1",
            evidence: evidence_without_source,
        }],
        provenance_without_source,
    );
    assert!(matches!(
        verify_admitted_batch(
            &missing,
            &policy,
            |record| &record.evidence,
            |record| record.identity.to_owned()
        ),
        Err(ProbeAdmissionError::MissingSourceTime)
    ));

    for (source, observed) in [
        ("2026-07-23", "2026-07-23"),
        ("2026-07-23T09:59:59", "2026-07-23T10:00:00"),
    ] {
        let candidate = DataBatch::strict(
            vec![Record {
                identity: "AP-1",
                evidence: evidence(ProviderId::Eastmoney, observed, source, batch_id),
            }],
            provenance(observed, source, batch_id),
        );
        assert!(
            matches!(
                verify_admitted_batch(
                    &candidate,
                    &policy,
                    |record| &record.evidence,
                    |record| record.identity.to_owned()
                ),
                Err(ProbeAdmissionError::InvalidTimestamp { .. })
            ),
            "ambiguous max-age timestamp was admitted: source={source} observed={observed}"
        );
    }
}

#[test]
fn stable_machine_states_do_not_conflate_diagnostics_with_admission() {
    assert_eq!(ProbeStatus::Admitted.as_str(), "admitted");
    assert_eq!(ProbeStatus::VerifiedEmpty.as_str(), "verified_empty");
    assert_eq!(
        ProbeStatus::DiagnosticCompleteUnadmitted.as_str(),
        "diagnostic_complete_unadmitted"
    );
    assert_eq!(
        ProbeStatus::SkippedMissingSecret.as_str(),
        "skipped_missing_secret"
    );
    assert_eq!(ProbeStatus::Failed.as_str(), "failed");
    assert!(ProbeStatus::Admitted.satisfies_capability());
    assert!(ProbeStatus::VerifiedEmpty.satisfies_capability());
    assert!(!ProbeStatus::DiagnosticCompleteUnadmitted.satisfies_capability());
    assert!(!ProbeStatus::SkippedMissingSecret.satisfies_capability());
    assert!(!ProbeStatus::Failed.satisfies_capability());
    assert_eq!(ProbeStatus::Admitted.to_string(), "admitted");
}

#[test]
fn complete_non_empty_batch_with_matching_evidence_is_admitted() {
    let observed = "2026-07-23T10:00:00+08:00";
    let source = "2026-07-23T09:59:30+08:00";
    let batch_id = "ths:consensus:1";
    let batch = DataBatch::strict(
        vec![Record {
            identity: "600519.SH",
            evidence: evidence(ProviderId::Tonghuashun, observed, source, batch_id),
        }],
        provenance(observed, source, batch_id),
    );

    let status = verify_admitted_batch(
        &batch,
        &policy(),
        |record| &record.evidence,
        |record| record.identity.to_owned(),
    )
    .unwrap();

    assert_eq!(status, ProbeStatus::Admitted);
}

#[test]
fn millisecond_prefixed_observation_time_is_checked_not_ignored() {
    let observed = "unix-ms:4102444800000";
    let source = "2026-07-23";
    let batch_id = "eastmoney:reports:1";
    let batch = DataBatch::strict(
        vec![Record {
            identity: "AP-1",
            evidence: evidence(ProviderId::Eastmoney, observed, source, batch_id),
        }],
        provenance(observed, source, batch_id),
    );
    let policy = ProbeAdmissionPolicy::new(ProviderId::Eastmoney).require_source_at();

    assert_eq!(
        verify_admitted_batch(
            &batch,
            &policy,
            |record| &record.evidence,
            |record| record.identity.to_owned()
        )
        .unwrap(),
        ProbeStatus::Admitted
    );
}

#[test]
fn ordinary_empty_incomplete_and_duplicate_batches_fail() {
    let observed = "2026-07-23T10:00:00+08:00";
    let source = "2026-07-23T09:59:30+08:00";
    let batch_id = "ths:consensus:1";

    let empty = DataBatch::<Record>::strict(vec![], provenance(observed, source, batch_id));
    assert!(matches!(
        verify_admitted_batch(
            &empty,
            &policy(),
            |record| &record.evidence,
            |record| { record.identity.to_owned() }
        ),
        Err(ProbeAdmissionError::EmptyBatch)
    ));

    let incomplete = DataBatch::best_effort(
        vec![Record {
            identity: "600519.SH",
            evidence: evidence(ProviderId::Tonghuashun, observed, source, batch_id),
        }],
        provenance(observed, source, batch_id),
        vec!["missing estimate".into()],
    )
    .unwrap();
    assert!(matches!(
        verify_admitted_batch(
            &incomplete,
            &policy(),
            |record| &record.evidence,
            |record| record.identity.to_owned()
        ),
        Err(ProbeAdmissionError::IncompleteQuality { .. })
    ));

    let duplicate = DataBatch::strict(
        vec![
            Record {
                identity: "600519.SH",
                evidence: evidence(ProviderId::Tonghuashun, observed, source, batch_id),
            },
            Record {
                identity: "600519.SH",
                evidence: evidence(ProviderId::Tonghuashun, observed, source, batch_id),
            },
        ],
        provenance(observed, source, batch_id),
    );
    assert!(matches!(
        verify_admitted_batch(
            &duplicate,
            &policy(),
            |record| &record.evidence,
            |record| record.identity.to_owned()
        ),
        Err(ProbeAdmissionError::DuplicateIdentity { identity })
            if identity == "600519.SH"
    ));
}

#[test]
fn mismatched_future_and_stale_record_evidence_fail() {
    let observed = "2026-07-23T10:00:00+08:00";
    let source = "2026-07-23T09:59:30+08:00";
    let batch_id = "ths:consensus:1";

    let mismatch = DataBatch::strict(
        vec![Record {
            identity: "600519.SH",
            evidence: evidence(
                ProviderId::Tonghuashun,
                observed,
                source,
                "ths:consensus:other",
            ),
        }],
        provenance(observed, source, batch_id),
    );
    assert!(matches!(
        verify_admitted_batch(
            &mismatch,
            &policy(),
            |record| &record.evidence,
            |record| record.identity.to_owned()
        ),
        Err(ProbeAdmissionError::BatchIdMismatch { .. })
    ));

    let future = DataBatch::strict(
        vec![Record {
            identity: "600519.SH",
            evidence: evidence(
                ProviderId::Tonghuashun,
                observed,
                "2026-07-23T10:00:01+08:00",
                batch_id,
            ),
        }],
        provenance(observed, "2026-07-23T10:00:01+08:00", batch_id),
    );
    assert!(matches!(
        verify_admitted_batch(
            &future,
            &policy(),
            |record| &record.evidence,
            |record| record.identity.to_owned()
        ),
        Err(ProbeAdmissionError::FutureSourceTime { .. })
    ));

    let future_millisecond = DataBatch::strict(
        vec![Record {
            identity: "600519.SH",
            evidence: evidence(
                ProviderId::Tonghuashun,
                observed,
                "2026-07-23T10:00:00.001+08:00",
                batch_id,
            ),
        }],
        provenance(observed, "2026-07-23T10:00:00.001+08:00", batch_id),
    );
    assert!(matches!(
        verify_admitted_batch(
            &future_millisecond,
            &policy(),
            |record| &record.evidence,
            |record| record.identity.to_owned()
        ),
        Err(ProbeAdmissionError::FutureSourceTime { .. })
    ));

    let stale = DataBatch::strict(
        vec![Record {
            identity: "600519.SH",
            evidence: evidence(
                ProviderId::Tonghuashun,
                observed,
                "2026-07-23T08:59:59+08:00",
                batch_id,
            ),
        }],
        provenance(observed, "2026-07-23T08:59:59+08:00", batch_id),
    );
    assert!(matches!(
        verify_admitted_batch(
            &stale,
            &policy(),
            |record| &record.evidence,
            |record| record.identity.to_owned()
        ),
        Err(ProbeAdmissionError::StaleSourceTime { .. })
    ));

    let malformed_numeric = DataBatch::strict(
        vec![Record {
            identity: "600519.SH",
            evidence: evidence(
                ProviderId::Tonghuashun,
                observed,
                "1780000000.trailing",
                batch_id,
            ),
        }],
        provenance(observed, "1780000000.trailing", batch_id),
    );
    assert!(matches!(
        verify_admitted_batch(
            &malformed_numeric,
            &policy(),
            |record| &record.evidence,
            |record| record.identity.to_owned()
        ),
        Err(ProbeAdmissionError::InvalidTimestamp {
            field: "source_at",
            ..
        })
    ));
}

#[test]
fn source_proven_verified_empty_is_the_only_empty_success() {
    let observed = "2026-07-23T10:00:00+08:00";
    let source = "2026-07-23T09:59:30+08:00";
    let batch_id = "ths:consensus:empty";
    let empty = VerifiedEmpty::new(
        "consensus",
        "600396.SH",
        "source explicitly reports no current institutional forecast",
        evidence(ProviderId::Tonghuashun, observed, source, batch_id),
        provenance(observed, source, batch_id),
    )
    .unwrap();

    assert_eq!(
        verify_verified_empty(&empty, &policy()).unwrap(),
        ProbeStatus::VerifiedEmpty
    );
    assert_eq!(empty.family(), "consensus");
    assert_eq!(empty.request_identity(), "600396.SH");
    assert_eq!(
        empty.reason(),
        "source explicitly reports no current institutional forecast"
    );
    assert_eq!(empty.evidence().provider(), ProviderId::Tonghuashun);
    assert_eq!(empty.provenance().batch_id(), Some(batch_id));
    assert_eq!(
        empty.to_string(),
        "family=consensus request_identity=600396.SH reason=source explicitly reports no current institutional forecast"
    );
}

#[test]
fn policy_and_verified_empty_reject_invalid_configuration_and_evidence() {
    assert!(matches!(
        ProbeAdmissionPolicy::new(ProviderId::Tonghuashun).with_max_source_age(Duration::ZERO),
        Err(magic_market_core::CoreError::InvalidRequest(_))
    ));

    let observed = "2026-07-23T10:00:00+08:00";
    let source = "2026-07-23T09:59:30+08:00";
    let batch_id = "ths:consensus:empty";
    for (family, request_identity, reason) in [
        ("", "600396.SH", "source empty"),
        ("consensus", "", "source empty"),
        ("consensus", "600396.SH", ""),
    ] {
        assert!(VerifiedEmpty::new(
            family,
            request_identity,
            reason,
            evidence(ProviderId::Tonghuashun, observed, source, batch_id),
            provenance(observed, source, batch_id),
        )
        .is_err());
    }

    let mismatched = VerifiedEmpty::new(
        "consensus",
        "600396.SH",
        "source empty",
        evidence(
            ProviderId::Tonghuashun,
            observed,
            source,
            "ths:consensus:other",
        ),
        provenance(observed, source, batch_id),
    );
    assert!(matches!(
        mismatched,
        Err(ProbeAdmissionError::BatchIdMismatch { .. })
    ));
}

#[test]
fn every_evidence_mismatch_is_rejected_explicitly() {
    let observed = "2026-07-23T10:00:00+08:00";
    let source = "2026-07-23T09:59:30+08:00";
    let batch_id = "ths:consensus:1";

    let cases = [
        (
            DataBatch::strict(
                vec![Record {
                    identity: "600519.SH",
                    evidence: evidence(ProviderId::Eastmoney, observed, source, batch_id),
                }],
                provenance(observed, source, batch_id),
            ),
            "provider",
        ),
        (
            DataBatch::strict(
                vec![Record {
                    identity: "600519.SH",
                    evidence: evidence(
                        ProviderId::Tonghuashun,
                        "2026-07-23T10:00:01+08:00",
                        source,
                        batch_id,
                    ),
                }],
                provenance(observed, source, batch_id),
            ),
            "observed",
        ),
        (
            DataBatch::strict(
                vec![Record {
                    identity: "600519.SH",
                    evidence: evidence(
                        ProviderId::Tonghuashun,
                        observed,
                        "2026-07-23T09:59:29+08:00",
                        batch_id,
                    ),
                }],
                provenance(observed, source, batch_id),
            ),
            "source",
        ),
    ];

    for (batch, expected) in cases {
        let error = verify_admitted_batch(
            &batch,
            &policy(),
            |record| &record.evidence,
            |record| record.identity.to_owned(),
        )
        .unwrap_err();
        assert!(
            matches!(
                (&error, expected),
                (ProbeAdmissionError::ProviderMismatch { .. }, "provider")
                    | (ProbeAdmissionError::ObservedAtMismatch { .. }, "observed")
                    | (ProbeAdmissionError::SourceAtMismatch { .. }, "source")
            ),
            "unexpected {expected} mismatch error: {error}"
        );
    }

    let provenance_without_batch_id: Provenance = serde_json::from_value(serde_json::json!({
        "source": "fixture",
        "source_at": source,
        "fetched_at": observed,
        "batch_id": null
    }))
    .unwrap();
    let missing_batch_id = DataBatch::strict(
        vec![Record {
            identity: "600519.SH",
            evidence: evidence(ProviderId::Tonghuashun, observed, source, batch_id),
        }],
        provenance_without_batch_id,
    );
    let error = verify_admitted_batch(
        &missing_batch_id,
        &policy(),
        |record| &record.evidence,
        |record| record.identity.to_owned(),
    )
    .unwrap_err();
    assert_eq!(error, ProbeAdmissionError::MissingBatchId);
}

#[test]
fn missing_source_time_and_invalid_identities_are_not_admitted() {
    let observed = "2026-07-23T10:00:00+08:00";
    let batch_id = "ths:consensus:1";
    let no_source = DataBatch::strict(
        vec![Record {
            identity: "600519.SH",
            evidence: SourceEvidence::new(ProviderId::Tonghuashun, observed, batch_id).unwrap(),
        }],
        Provenance::new("fixture", observed)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap(),
    );
    assert!(matches!(
        verify_admitted_batch(
            &no_source,
            &policy(),
            |record| &record.evidence,
            |record| record.identity.to_owned()
        ),
        Err(ProbeAdmissionError::MissingSourceTime)
    ));
    assert_eq!(
        verify_admitted_batch(
            &no_source,
            &ProbeAdmissionPolicy::new(ProviderId::Tonghuashun),
            |record| &record.evidence,
            |record| record.identity.to_owned()
        )
        .unwrap(),
        ProbeStatus::Admitted
    );

    for identity in [" ", "600519.SH\ncontrol"] {
        let batch = DataBatch::strict(
            vec![Record {
                identity,
                evidence: SourceEvidence::new(ProviderId::Tonghuashun, observed, batch_id).unwrap(),
            }],
            Provenance::new("fixture", observed)
                .unwrap()
                .with_batch_id(batch_id)
                .unwrap(),
        );
        assert!(matches!(
            verify_admitted_batch(
                &batch,
                &ProbeAdmissionPolicy::new(ProviderId::Tonghuashun),
                |record| &record.evidence,
                |record| record.identity.to_owned()
            ),
            Err(ProbeAdmissionError::EmptyIdentity)
        ));
    }
}

#[test]
fn supported_timestamp_forms_and_calendar_edges_are_verified() {
    let batch_id = "eastmoney:reports:timestamp";
    let accepted = [
        ("1784786400", "1784786400"),
        ("1784786400.123", "1784786400.999"),
        ("2024-02-29", "2024-02-29"),
        ("2026-07-23 10:00:00", "2026-07-23T10:00:00Z"),
        (
            "2026-07-23T10:00:00.123+08:00",
            "2026-07-23T10:00:00.123+08:00",
        ),
        ("2026-07-23T10:00:00-08:00", "2026-07-24T02:00:00Z"),
    ];
    for (source, observed) in accepted {
        let batch = DataBatch::strict(
            vec![Record {
                identity: "AP-1",
                evidence: evidence(ProviderId::Eastmoney, observed, source, batch_id),
            }],
            provenance(observed, source, batch_id),
        );
        assert_eq!(
            verify_admitted_batch(
                &batch,
                &ProbeAdmissionPolicy::new(ProviderId::Eastmoney).require_source_at(),
                |record| &record.evidence,
                |record| record.identity.to_owned()
            )
            .unwrap(),
            ProbeStatus::Admitted,
            "source={source} observed={observed}"
        );
    }

    for invalid in [
        "unix-ms:",
        "unix-ms:1x",
        "2023-02-29",
        "2026-13-01",
        "2026-07-23T10:00",
        "2026-07-23X10:00:00Z",
        "2026-07-23T24:00:00Z",
        "2026-07-23T10:60:00Z",
        "2026-07-23T10:00:60Z",
        "2026-07-23T10:00:00+24:00",
        "2026-07-23T10:00:00+08:60",
        "2026-07-23T10:00:00+0800",
    ] {
        let batch = DataBatch::strict(
            vec![Record {
                identity: "AP-1",
                evidence: evidence(
                    ProviderId::Eastmoney,
                    "2026-07-24T10:00:00+08:00",
                    invalid,
                    batch_id,
                ),
            }],
            provenance("2026-07-24T10:00:00+08:00", invalid, batch_id),
        );
        assert!(
            matches!(
                verify_admitted_batch(
                    &batch,
                    &ProbeAdmissionPolicy::new(ProviderId::Eastmoney).require_source_at(),
                    |record| &record.evidence,
                    |record| record.identity.to_owned()
                ),
                Err(ProbeAdmissionError::InvalidTimestamp {
                    field: "source_at",
                    ..
                })
            ),
            "invalid timestamp admitted: {invalid}"
        );
    }

    let invalid_observed = DataBatch::strict(
        vec![Record {
            identity: "AP-1",
            evidence: evidence(
                ProviderId::Eastmoney,
                "invalid-observed",
                "2026-07-23",
                batch_id,
            ),
        }],
        provenance("invalid-observed", "2026-07-23", batch_id),
    );
    assert!(matches!(
        verify_admitted_batch(
            &invalid_observed,
            &ProbeAdmissionPolicy::new(ProviderId::Eastmoney).require_source_at(),
            |record| &record.evidence,
            |record| record.identity.to_owned()
        ),
        Err(ProbeAdmissionError::InvalidTimestamp {
            field: "observed_at",
            ..
        })
    ));
}

#[test]
fn stale_diagnostics_preserve_nanosecond_precision() {
    let observed = "2026-07-23T10:00:05.000000001+08:00";
    let source = "2026-07-23T10:00:00+08:00";
    let batch_id = "eastmoney:reports:nanosecond-stale";
    let batch = DataBatch::strict(
        vec![Record {
            identity: "AP-1",
            evidence: evidence(ProviderId::Eastmoney, observed, source, batch_id),
        }],
        provenance(observed, source, batch_id),
    );
    let policy = ProbeAdmissionPolicy::new(ProviderId::Eastmoney)
        .require_source_at()
        .with_max_source_age(Duration::from_secs(5))
        .unwrap();

    let error = verify_admitted_batch(
        &batch,
        &policy,
        |record| &record.evidence,
        |record| record.identity.to_owned(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ProbeAdmissionError::StaleSourceTime {
            age_nanos: 5_000_000_001,
            max_age_nanos: 5_000_000_000
        }
    ));
    assert_eq!(
        error.to_string(),
        "source_at is stale by 5000000001ns; maximum allowed age is 5000000000ns"
    );
}
