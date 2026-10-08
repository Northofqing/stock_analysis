use super::*;

pub(in crate::durable_delivery) fn retained_fixture(label: &str) -> Fixture {
    let mut fixture = Fixture::new(label);
    drop(fixture.coordinator.take().unwrap());
    let connection = Connection::open(&fixture.database_path).unwrap();
    let objects = connection.prepare(
        "SELECT type,name FROM sqlite_master WHERE type IN ('trigger','table') AND name NOT LIKE 'sqlite_%' ORDER BY type DESC",
    ).unwrap().query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    for (kind, name) in objects {
        connection
            .execute_batch(&format!("DROP {kind} \"{name}\""))
            .unwrap();
    }
    connection
        .execute_batch(super::super::monitor_schema9::DDL)
        .unwrap();
    for row in compiled_policy_catalog() {
        connection
            .execute(
                "INSERT INTO delivery_policy_catalog VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    row.push_kind.as_str(),
                    row.sub_kind.as_str(),
                    row.cooldown_scope.as_str(),
                    row.base_cooldown_secs,
                    row.override_cooldown_secs,
                    row.window_mode.as_str(),
                    i64::from(row.counts_against_daily_budget),
                    row.policy_version
                ],
            )
            .unwrap();
    }
    connection.pragma_update(None, "user_version", 9).unwrap();
    drop(connection);
    fixture.coordinator = FixtureCoordinator(Some(Arc::new(
        DurableDeliveryCoordinator::open_existing_monitor_schema9(config(&fixture)).unwrap(),
    )));
    fixture
}

fn config(fixture: &Fixture) -> CoordinatorConfig {
    let code = fixture
        .database_path
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    CoordinatorConfig::test(
        &fixture.database_path,
        code,
        "TEST_CODE_SCHEMA9_OWNER_0123456789abcdef",
    )
}

fn catalog(fixture: &Fixture) -> Vec<(String, String, Option<String>)> {
    Connection::open(&fixture.database_path)
        .unwrap()
        .prepare("SELECT type,name,sql FROM sqlite_master ORDER BY type,name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn monitor_schema9_delivery_reopen_and_exact_retry_keep_catalog_and_send_once() {
    let mut fixture = retained_fixture("SCHEMA9_DELIVERY");
    let before = catalog(&fixture);
    let item = envelope(
        "SCHEMA9",
        PushKind::DataMode,
        DeliverySubKind::None,
        "2026-07-15",
        false,
    );
    let append = MemoryAppendPort::default();
    prepare_reserved(&fixture, &item, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    fixture
        .coordinator
        .resume_deliverable(&item.decision_identity, &sinks, now())
        .unwrap();
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::Delivered,
        &item.decision_identity,
    );
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&item.decision_identity)
            .unwrap(),
        DecisionState::Delivered
    );
    drop(fixture.coordinator.take().unwrap());
    fixture.coordinator = FixtureCoordinator(Some(Arc::new(
        DurableDeliveryCoordinator::open_existing_monitor_schema9(config(&fixture)).unwrap(),
    )));
    fixture.coordinator.prepare(&item, 1, now()).unwrap();
    fixture
        .coordinator
        .resume_deliverable(&item.decision_identity, &sinks, now())
        .unwrap();
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(catalog(&fixture), before);
    assert_eq!(fixture.query_i64("PRAGMA user_version"), 9);
}

#[test]
fn monitor_schema9_missing_trigger_and_changed_policy_fail_without_healing() {
    for policy in [false, true] {
        let mut fixture = retained_fixture("SCHEMA9_NO_HEAL");
        drop(fixture.coordinator.take().unwrap());
        let c = Connection::open(&fixture.database_path).unwrap();
        if policy {
            c.execute("UPDATE delivery_policy_catalog SET base_cooldown_secs=123 WHERE push_kind='DataMode'", []).unwrap();
        } else {
            c.execute_batch("DROP TRIGGER immutable_state_event_update")
                .unwrap();
        }
        drop(c);
        let before = catalog(&fixture);
        assert!(
            DurableDeliveryCoordinator::open_existing_monitor_schema9(config(&fixture)).is_err()
        );
        assert_eq!(catalog(&fixture), before);
        assert_eq!(fixture.query_i64("PRAGMA user_version"), 9);
        if policy {
            assert_eq!(fixture.query_i64("SELECT base_cooldown_secs FROM delivery_policy_catalog WHERE push_kind='DataMode'"), 123);
        }
    }
}

#[test]
fn monitor_schema9_rejects_other_headers_and_platform_extensions() {
    for version in [0, 8, 14] {
        let mut fixture = retained_fixture("SCHEMA9_HEADER");
        drop(fixture.coordinator.take().unwrap());
        Connection::open(&fixture.database_path)
            .unwrap()
            .pragma_update(None, "user_version", version)
            .unwrap();
        let before = catalog(&fixture);
        assert!(
            DurableDeliveryCoordinator::open_existing_monitor_schema9(config(&fixture)).is_err()
        );
        assert_eq!(catalog(&fixture), before);
        assert_eq!(fixture.query_i64("PRAGMA user_version"), version);
    }
    let mut fixture = retained_fixture("SCHEMA9_EXTENSION");
    drop(fixture.coordinator.take().unwrap());
    Connection::open(&fixture.database_path)
        .unwrap()
        .execute_batch("CREATE TABLE g5b_day_heads(TEST_CODE INTEGER)")
        .unwrap();
    let before = catalog(&fixture);
    assert!(DurableDeliveryCoordinator::open_existing_monitor_schema9(config(&fixture)).is_err());
    assert_eq!(catalog(&fixture), before);
}

#[test]
fn monitor_schema9_runtime_drift_blocks_new_delivery_without_schema_repair() {
    let fixture = retained_fixture("SCHEMA9_DRIFT");
    Connection::open(&fixture.database_path)
        .unwrap()
        .execute_batch("DROP TRIGGER immutable_attempt_event_update")
        .unwrap();
    let item = envelope(
        "SCHEMA9_DRIFT",
        PushKind::DataMode,
        DeliverySubKind::None,
        "2026-07-15",
        false,
    );
    assert!(fixture.coordinator.prepare(&item, 1, now()).is_err());
    assert_eq!(
        fixture.query_i64("SELECT count(*) FROM delivery_decisions"),
        0
    );
    assert_eq!(fixture.query_i64("PRAGMA user_version"), 9);
}

#[test]
fn monitor_schema9_uncertain_survives_reopen_without_automatic_resend() {
    let mut fixture = retained_fixture("SCHEMA9_UNCERTAIN");
    let item = envelope(
        "SCHEMA9_UNCERTAIN",
        PushKind::DataMode,
        DeliverySubKind::None,
        "2026-07-15",
        false,
    );
    let append = MemoryAppendPort::default();
    prepare_reserved(&fixture, &item, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    fixture
        .coordinator
        .resume_deliverable(&item.decision_identity, &sinks, now())
        .unwrap();
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::UncertainManualReview,
        &item.decision_identity,
    );
    let before = catalog(&fixture);
    drop(fixture.coordinator.take().unwrap());
    fixture.coordinator = FixtureCoordinator(Some(Arc::new(
        DurableDeliveryCoordinator::open_existing_monitor_schema9(config(&fixture)).unwrap(),
    )));
    let resumed = fixture
        .coordinator
        .resume_deliverable(&item.decision_identity, &sinks, now())
        .unwrap();
    assert_eq!(resumed.state, DecisionState::UncertainManualReview);
    assert_eq!(resumed.sink_calls, 0);
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(catalog(&fixture), before);
}

#[test]
fn monitor_schema9_manual_rejected_finalizes_and_retries_without_resend_or_catalog_change() {
    for task_bound in [false, true] {
        let mut fixture = retained_fixture("SCHEMA9_MANUAL_REJECTED");
        let before = catalog(&fixture);
        let item = envelope(
            "SCHEMA9_MANUAL_REJECTED",
            PushKind::CloseCall,
            DeliverySubKind::None,
            "2026-07-30",
            task_bound,
        );
        let append = MemoryAppendPort::default();
        prepare_reserved(&fixture, &item, &append);
        let sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
        let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
        fixture
            .coordinator
            .resume_deliverable(&item.decision_identity, &sinks, now())
            .unwrap();
        reconcile_terminal(
            &fixture,
            &append,
            DecisionState::UncertainManualReview,
            &item.decision_identity,
        );
        let original_result: Vec<u8> = Connection::open(&fixture.database_path)
            .unwrap()
            .query_row("SELECT result_canonical FROM sink_results", [], |r| {
                r.get(0)
            })
            .unwrap();
        let original_disposition: (String, Vec<u8>) = Connection::open(&fixture.database_path)
        .unwrap()
        .query_row(
            "SELECT disposition_identity,disposition_canonical FROM delivery_disposition_payloads",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
        let command = ManualResolutionCommand {
            decision_identity: item.decision_identity.clone(),
            disposition: ManualDisposition::Rejected,
            operator_identity: "TEST_CODE_SCHEMA9_MANUAL_OPERATOR".to_owned(),
            reason: "TEST_CODE_cancel_with_historical_delivery_unknown".to_owned(),
            external_evidence: b"TEST_CODE_exact_manual_authorization_bytes\n".to_vec(),
            resolved_at: now(),
        };
        assert_eq!(
            fixture
                .coordinator
                .resolve_uncertain(&command, &append)
                .unwrap(),
            DecisionState::ManualRejectedAuditPending
        );
        assert_eq!(
            fixture.query_i64("SELECT count(*) FROM cooldown_reservations WHERE state='Released'"),
            1
        );
        assert_eq!(
            fixture
                .query_i64("SELECT count(*) FROM daily_budget_reservations WHERE state='Released'"),
            1
        );
        assert_eq!(
            fixture.query_i64("SELECT retry_authorized FROM delivery_decisions"),
            0
        );
        drop(fixture.coordinator.take().unwrap());
        fixture.coordinator = FixtureCoordinator(Some(Arc::new(
            DurableDeliveryCoordinator::open_existing_monitor_schema9(config(&fixture)).unwrap(),
        )));
        let evidence: Vec<u8> = Connection::open(&fixture.database_path)
            .unwrap()
            .query_row(
                "SELECT evidence_canonical FROM manual_resolutions",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(evidence, command.external_evidence);
        // An authorized retry continues the pending finalizer; it does not resolve or send again.
        reconcile_terminal(
            &fixture,
            &append,
            DecisionState::ManualResolvedRejected,
            &item.decision_identity,
        );
        assert_eq!(fixture.query_i64("SELECT count(*) FROM delivery_state_events WHERE to_state='ManualRejectedTaskTransitionPending'"), i64::from(task_bound));
        if task_bound {
            let c = Connection::open(&fixture.database_path).unwrap();
            let (identity, canonical, hash, immutable_ref): (String, Vec<u8>, String, String) = c.query_row(
            "SELECT t.transition_identity,t.transition_canonical,t.transition_sha256,t.immutable_audit_ref FROM task_transition_payloads t JOIN delivery_disposition_payloads d USING(disposition_identity) WHERE d.disposition='ManualRejected' AND t.append_state='Appended' AND t.hydration_state='Pending'", [],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).unwrap();
            let records = append.records.lock().unwrap();
            let record = records.get(&identity).unwrap();
            assert_eq!(record.record_kind, "BR-140TaskTransition");
            assert_eq!(record.canonical_bytes, canonical);
            assert_eq!(record.sha256, hash);
            assert_eq!(record.immutable_ref, immutable_ref);
        } else {
            assert_eq!(
                fixture.query_i64("SELECT count(*) FROM task_transition_payloads"),
                0
            );
        }
        let events = fixture.query_i64("SELECT count(*) FROM delivery_state_events");
        let records = append.records.lock().unwrap().clone();
        reconcile_terminal(
            &fixture,
            &append,
            DecisionState::ManualResolvedRejected,
            &item.decision_identity,
        );
        let resumed = fixture
            .coordinator
            .resume_deliverable(&item.decision_identity, &sinks, now())
            .unwrap();
        assert_eq!(resumed.state, DecisionState::ManualResolvedRejected);
        assert_eq!(resumed.sink_calls, 0);
        assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            fixture.coordinator.resolve_uncertain(&command, &append),
            Err(DurableDeliveryError::InvalidManualResolution(_))
        ));
        assert_eq!(
            fixture.query_i64("SELECT count(*) FROM delivery_state_events"),
            events
        );
        assert_eq!(
            fixture.query_i64("SELECT count(*) FROM manual_resolutions"),
            1
        );
        assert_eq!(*append.records.lock().unwrap(), records);
        let c = Connection::open(&fixture.database_path).unwrap();
        assert_eq!(
            c.query_row("SELECT result_canonical FROM sink_results", [], |r| r
                .get::<_, Vec<u8>>(0))
                .unwrap(),
            original_result
        );
        assert_eq!(c.query_row("SELECT disposition_canonical FROM delivery_disposition_payloads WHERE disposition_identity=?1", [&original_disposition.0], |r| r.get::<_, Vec<u8>>(0)).unwrap(), original_disposition.1);
        assert_eq!(catalog(&fixture), before);
        assert_eq!(fixture.query_i64("PRAGMA user_version"), 9);
    }
}

#[test]
fn monitor_schema9_manual_resolution_keeps_accepted_and_foundation_guards() {
    for foundation in [false, true] {
        let fixture = retained_fixture("SCHEMA9_MANUAL_GUARD");
        let before = catalog(&fixture);
        let item = if foundation {
            w12_foundation_envelope("SCHEMA9_FOUNDATION_GUARD")
        } else {
            envelope(
                "SCHEMA9_ACCEPTED_GUARD",
                PushKind::DataMode,
                DeliverySubKind::None,
                "2026-07-30",
                false,
            )
        };
        let append = MemoryAppendPort::default();
        prepare_reserved(&fixture, &item, &append);
        let sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
        fixture
            .coordinator
            .resume_deliverable(&item.decision_identity, &[sink], now())
            .unwrap();
        reconcile_terminal(
            &fixture,
            &append,
            DecisionState::UncertainManualReview,
            &item.decision_identity,
        );
        let uncertain_cooldowns = fixture.query_i64(
            "SELECT count(*) FROM cooldown_reservations WHERE state='Uncertain'",
        );
        let command = ManualResolutionCommand {
            decision_identity: item.decision_identity.clone(),
            disposition: if foundation {
                ManualDisposition::Rejected
            } else {
                ManualDisposition::Accepted {
                    receipt: Some(receipt(now())),
                }
            },
            operator_identity: "TEST_CODE_SCHEMA9_GUARD_OPERATOR".to_owned(),
            reason: "TEST_CODE_ROUTE_MUST_NOT_BYPASS_PLATFORM_GUARD".to_owned(),
            external_evidence: b"TEST_CODE_MANUAL_GUARD_AUTH".to_vec(),
            resolved_at: now(),
        };
        assert!(matches!(
            fixture.coordinator.resolve_uncertain(&command, &append),
            Err(DurableDeliveryError::InvalidConfiguration(_))
        ));
        assert_eq!(
            fixture
                .coordinator
                .decision_state(&item.decision_identity)
                .unwrap(),
            DecisionState::UncertainManualReview
        );
        assert_eq!(
            fixture.query_i64("SELECT count(*) FROM manual_resolutions"),
            0
        );
        assert_eq!(
            fixture.query_i64("SELECT count(*) FROM cooldown_reservations WHERE state='Uncertain'"),
            uncertain_cooldowns
        );
        assert_eq!(catalog(&fixture), before);
    }
}

#[test]
fn monitor_schema9_retained_quote_provider_owner_survives_reopen_and_keeps_rolling_policy() {
    let mut fixture = retained_fixture("SCHEMA9_RETAINED_QUOTES");
    let before = catalog(&fixture);
    let occurrence = "retained-quote-observation:2026-07-15:intraday:14:30";
    let item = DeliveryEnvelope::new(
        "2026-07-15",
        PushKind::IntradayMarket,
        DeliverySubKind::None,
        "GLOBAL",
        occurrence,
        "TEST_CODE_ORIGINAL_QUOTE_HASH",
        b"TEST_CODE_ORIGINAL_QUOTE_CANONICAL".to_vec(),
        "TEST_CODE_QUOTE_SUBJECT",
        b"TEST_CODE Frozen quote-only; no current position authority".to_vec(),
        false,
        None,
    )
    .unwrap()
    .with_provider_evidence(
        Some("2026-07-15T06:30:01Z".into()),
        Some("2026-07-15".into()),
        vec!["TEST_CODE_NATIVE_QUOTE_BATCH".into()],
    )
    .unwrap();
    let append = MemoryAppendPort::default();
    prepare_reserved(&fixture, &item, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    fixture
        .coordinator
        .resume_deliverable(&item.decision_identity, &sinks, now())
        .unwrap();
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::Delivered,
        &item.decision_identity,
    );
    drop(fixture.coordinator.take().unwrap());
    fixture.coordinator = FixtureCoordinator(Some(Arc::new(
        DurableDeliveryCoordinator::open_existing_monitor_schema9(config(&fixture)).unwrap(),
    )));
    let owner = fixture
        .coordinator
        .inspect_exact_occurrence_owner(
            "2026-07-15",
            PushKind::IntradayMarket,
            DeliverySubKind::None,
            "GLOBAL",
            occurrence,
        )
        .unwrap()
        .unwrap();
    assert_eq!(owner.envelope, item);
    assert_eq!(owner.state, DecisionState::Delivered);
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.query_i64("SELECT base_cooldown_secs FROM delivery_policy_catalog WHERE push_kind='IntradayMarket'"), 900);
    assert_eq!(fixture.query_i64("SELECT counts_against_daily_budget FROM delivery_policy_catalog WHERE push_kind='IntradayMarket'"), 1);
    assert_eq!(catalog(&fixture), before);
    assert_eq!(fixture.query_i64("PRAGMA user_version"), 9);
}
