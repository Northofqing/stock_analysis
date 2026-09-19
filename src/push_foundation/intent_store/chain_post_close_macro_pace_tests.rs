use super::*;

#[derive(Debug, PartialEq)]
struct PaceDatabaseSnapshot {
    requests: Vec<Vec<rusqlite::types::Value>>,
    begins: Vec<Vec<rusqlite::types::Value>>,
    results: Vec<Vec<rusqlite::types::Value>>,
    terminals: Vec<Vec<rusqlite::types::Value>>,
    dimensions: Vec<Vec<rusqlite::types::Value>>,
    audits: Vec<Vec<rusqlite::types::Value>>,
}

fn pace_database_snapshot(connection: &rusqlite::Connection, intent: &str) -> PaceDatabaseSnapshot {
    PaceDatabaseSnapshot {
        requests: historical_rows(connection, "chain_post_close_macro_request_plans", intent),
        begins: historical_rows(connection, "chain_post_close_macro_attempt_begins", intent),
        results: historical_rows(connection, "chain_post_close_macro_attempt_results", intent),
        terminals: historical_rows(connection, "chain_post_close_macro_query_terminals", intent),
        dimensions: historical_rows(
            connection,
            "chain_post_close_macro_dimension_terminals",
            intent,
        ),
        audits: historical_all_rows(
            connection,
            "SELECT * FROM data_acquisition_audit ORDER BY id",
        ),
    }
}

fn dimension_one(
    connection: &rusqlite::Connection,
    intent: &str,
) -> Option<(u64, Vec<u8>, i64, Option<u32>, i64)> {
    match connection.query_row(
        "SELECT run_version,bytes,recorded_at,selected_candidate_ordinal,pace_due \
         FROM chain_post_close_macro_dimension_terminals \
         WHERE intent_id=?1 AND dimension=1",
        [intent],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    ) {
        Ok(value) => Some(value),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(error) => panic!("TEST_CODE read dimension-one terminal: {error}"),
    }
}

fn web_call_count(server: &MacroFullLoopbackServer) -> usize {
    server
        .snapshot()
        .calls
        .iter()
        .filter(|call| matches!(call.call, Call::Web { .. }))
        .count()
}

async fn wait_for_web_calls(server: &MacroFullLoopbackServer, count: usize) {
    loop {
        if web_call_count(server) >= count {
            return;
        }
        tokio::select! {
            _ = server.wait_for_web_count(count) => return,
            _ = tokio::time::sleep(Duration::from_millis(5)) => {}
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn single_user_local_web_pace_reopens_after_only_saved_remaining_delay() {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut macro_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(90),
        async {
            let baseline = control_tests::setup_external_parent(
                &mut business,
                &mut parent_server,
                "TEST_CODE_RUN_WEB_PACE_REOPEN",
            )
            .await;
            assert_eq!(
                business
                    .chain_post_close()
                    .migrate_schema_v11_to_v12()
                    .unwrap()
                    .schema_version(),
                12
            );
            macro_server = Some(MacroFullLoopbackServer::bind().await);
            let server = macro_server.as_ref().unwrap();
            server.enable_web_response_gate_for_test();
            let macro_source = GrpcSource::from_macro_loopback_test_client(
                server.connect().await,
                server.endpoint().to_owned(),
            );
            let registered = [
                GeneralWebResearchProvider::SerpApi,
                GeneralWebResearchProvider::Bocha,
                GeneralWebResearchProvider::Tavily,
            ];
            let search_service = macro_search_service(&registered);
            let started_at = micros("2026-09-14T15:30:00+08:00");
            let clock = MacroClock {
                now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
                observation: DateTime::parse_from_rfc3339(
                    "2026-09-14T15:30:00+08:00",
                )
                .unwrap(),
                observation_calls: Cell::new(0),
            };
            let database = business.database();
            let parent_wire = parent_server
                .as_ref()
                .unwrap()
                .snapshot_with_tcp_for_test();
            let parent_memberships = parent_server
                .as_ref()
                .unwrap()
                .membership_snapshot();

            let mut local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&baseline.config)
                .unwrap();
            let lease = local
                .resume_run(
                    &baseline.intent,
                    macro_lease(
                        "TEST_CODE_WEB_PACE_OWNER_A",
                        started_at,
                        started_at + 300_000,
                        baseline.head,
                    ),
                )
                .unwrap();
            let first_generation = lease.generation();
            let mut io = local
                .macro_preparation_io_v12(
                    lease,
                    &baseline.queries,
                    &clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &baseline.source,
                    &macro_source,
                    &search_service,
                )
                .unwrap();
            let mut reader = BusinessIntentStore::open(&database).unwrap();
            let mut read_local = reader
                .single_user_local_chain_post_close(&baseline.config)
                .unwrap();
            let sql_reader = BusinessIntentStore::open(&database).unwrap();
            let mut prepared = Box::pin(prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                baseline.stocks.clone(),
                None,
                &mut io,
            ));
            tokio::select! {
                result = &mut prepared => panic!("TEST_CODE pace setup returned before five Gateways: {result:?}"),
                _ = server.wait_for_gateway_count(5) => {}
            }
            let held_gateways = server.snapshot();
            assert_eq!(held_gateways.calls.len(), 5);
            assert!(held_gateways.calls.iter().all(|call|
                matches!(call.call, Call::Gateway(_))
                    && call.authorized
                    && call.response.is_none()));
            server.release_all();
            // Wait for all five Gateway results to be persisted *before* moving the
            // clock: terminal_at is sampled from the controlled clock, so advancing
            // early would move the Gateway pace due and stall the durable wait again.
            loop {
                let observed = read_local.inspect_macro(&baseline.intent).unwrap();
                if observed
                    .attempts()
                    .iter()
                    .filter(|attempt| attempt.result_version().is_some())
                    .count()
                    == 5
                {
                    break;
                }
                tokio::select! {
                    result = &mut prepared => panic!("TEST_CODE pace setup returned before five confirmed Gateways: {result:?}"),
                    _ = tokio::time::sleep(Duration::from_millis(1)) => {}
                }
            }
            // Deterministic clock model. All five Gateway terminals were recorded
            // while the controlled clock stood at started_at, so the Gateway pace due
            // is exactly started_at + 200_000. The durable wait (macro_driver.rs)
            // only leaves its loop once the clock reaches the due, so the clock is
            // stepped to that due here -- neither frozen (which would stall the wait
            // until the outer wall-clock 15s budget expires and silently degrades the
            // whole Macro to an empty background) nor driven by real elapsed time
            // (which made the lease window unpredictable).
            //
            // Holding the clock at this exact value through dimension one also keeps
            // the observation point deterministic: finish_dimension samples the clock
            // for `recorded_at`, so pace_due is exactly started_at + 500_000.
            clock
                .now
                .set(UtcMicros::try_new(started_at + 200_000).unwrap());
            tokio::select! {
                result = &mut prepared => panic!("TEST_CODE pace setup returned before Web dimension one: {result:?}"),
                _ = wait_for_web_calls(server, 1) => {}
            }
            assert_eq!(
                server.snapshot().calls.last().unwrap().call,
                Call::Web {
                    dimension: 0,
                    provider: GeneralWebResearchProvider::SerpApi,
                }
            );
            server.release_web_response_for_test();
            tokio::select! {
                result = &mut prepared => panic!("TEST_CODE pace setup returned before Web dimension-one success: {result:?}"),
                _ = wait_for_web_calls(server, 2) => {}
            }
            assert_eq!(
                server.snapshot().calls.last().unwrap().call,
                Call::Web {
                    dimension: 0,
                    provider: GeneralWebResearchProvider::Bocha,
                }
            );
            server.release_web_response_for_test();

            let dimension = loop {
                if let Some(dimension) =
                    dimension_one(&sql_reader.connection, baseline.intent.as_str())
                {
                    break dimension;
                }
                tokio::select! {
                    result = &mut prepared => panic!("TEST_CODE pace setup returned before dimension terminal: {result:?}"),
                    _ = tokio::time::sleep(Duration::from_millis(1)) => {}
                }
            };
            let (dimension_version, dimension_bytes, recorded_at, selected, pace_due) = dimension;
            assert_eq!(selected, Some(2));
            assert_eq!(pace_due - recorded_at, 300_000);
            assert!(clock.now.get().get() < pace_due);
            let dimension_json: serde_json::Value =
                serde_json::from_slice(&dimension_bytes).unwrap();
            assert_eq!(dimension_json["version"], 2);
            assert_eq!(dimension_json["dimension"], 1);
            assert_eq!(dimension_json["selected_candidate"], 2);
            assert_eq!(dimension_json["eligible_candidates"], serde_json::json!([1, 2, 3]));
            assert_eq!(dimension_json["pace_due"], pace_due);
            // The canonical form is struct field order, so re-encoding a parsed
            // serde_json::Value (whose map is sorted) is not a fixed point. Verify the
            // canonical bytes through the typed codec instead: macro_codec::decode
            // itself refuses any payload where encode(decode(b)) != b.
            let typed: crate::push_foundation::intent_store::chain_post_close::macro_native::DimensionTerminal =
                macro_codec::decode(&dimension_bytes).unwrap();
            assert_eq!(macro_codec::encode(&typed).unwrap(), dimension_bytes);

            let confirmed = read_local.inspect_macro(&baseline.intent).unwrap();
            assert!(!confirmed.is_complete());
            assert!(!confirmed.has_unconfirmed_effect());
            assert_eq!(confirmed.plan().started_at().get(), started_at);
            assert_eq!(confirmed.plan().deadline_at().get(), started_at + 15_000_000);
            assert_eq!(confirmed.parent_final_bytes(), baseline.final_bytes.as_slice());
            assert_eq!(confirmed.attempts().len(), 7);
            assert!(confirmed
                .attempts()
                .iter()
                .all(|attempt| attempt.result_version().is_some()));
            assert!(confirmed
                .query_terminal(QueryKey::Web {
                    dimension: 1,
                    candidate: 1,
                })
                .is_some());
            assert!(confirmed
                .query_terminal(QueryKey::Web {
                    dimension: 1,
                    candidate: 2,
                })
                .is_some());
            assert!(confirmed
                .query_terminal(QueryKey::Web {
                    dimension: 2,
                    candidate: 1,
                })
                .is_none());
            assert_eq!(
                confirmed
                    .query_terminal(QueryKey::Web {
                        dimension: 1,
                        candidate: 2,
                    })
                    .unwrap()
                    .version()
                    + 1,
                dimension_version
            );
            let initial_wire = server.snapshot();
            assert_eq!(initial_wire.calls.len(), 7);
            assert!(initial_wire.calls.iter().all(|call| call.response.is_some()));
            let plan_bytes = confirmed.plan_bytes().to_vec();
            let deadline = confirmed.plan().deadline_at().get();
            let confirmed_attempts = confirmed
                .attempts()
                .iter()
                .map(|attempt| {
                    (
                        attempt.query_key(),
                        attempt.request_bytes().to_vec(),
                        attempt.response_bytes().unwrap().to_vec(),
                    )
                })
                .collect::<Vec<_>>();
            let confirmed_database =
                pace_database_snapshot(&sql_reader.connection, baseline.intent.as_str());
            assert_eq!(confirmed_database.dimensions.len(), 1);
            let first_head = read_local.inspect_run(&baseline.intent).unwrap().head_version();
            drop(prepared);
            drop(io);
            drop(read_local);
            reader.connection.close().unwrap();
            sql_reader.connection.close().unwrap();
            drop(local);
            drop(macro_source);
            business.reopen();

            // Establish all network connections before pausing Tokio time. The
            // controlled section below measures only the recovered 100ms wait.
            let reopened_source = GrpcSource::from_macro_loopback_test_client(
                server.connect().await,
                server.endpoint().to_owned(),
            );
            clock
                .now
                .set(UtcMicros::try_new(pace_due - 100_000).unwrap());
            let mut local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&baseline.config)
                .unwrap();
            let lease = local
                .resume_run(
                    &baseline.intent,
                    macro_lease(
                        "TEST_CODE_WEB_PACE_OWNER_B",
                        pace_due - 100_000,
                        pace_due + 50_000,
                        first_head,
                    ),
                )
                .unwrap();
            assert_eq!(lease.generation(), first_generation + 1);
            let reopened_head = lease.head_version();
            assert_eq!(reopened_head, first_head + 1);
            let mut io = local
                .macro_preparation_io_v12(
                    lease,
                    &baseline.queries,
                    &clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &baseline.source,
                    &reopened_source,
                    &search_service,
                )
                .unwrap();
            let mut reader = BusinessIntentStore::open(&database).unwrap();
            let mut read_local = reader
                .single_user_local_chain_post_close(&baseline.config)
                .unwrap();
            let sql_reader = BusinessIntentStore::open(&database).unwrap();
            let mut prepared = Box::pin(prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                baseline.stocks.clone(),
                None,
                &mut io,
            ));
            // The controlled clock is held 100ms before the saved pace due. The durable
            // wait recomputes remaining = pace_due - clock.now() on every round and
            // sleeps on the monotonic clock, so it keeps re-waiting and dimension two
            // must not begin. Observing is done over real time instead of pausing the
            // runtime: pausing Tokio's timer wheel would also freeze timers the earlier
            // prepare stages rely on, so the runner may never reach this wait at all.
            let early_anchor = tokio::time::Instant::now();
            while early_anchor.elapsed() < Duration::from_millis(300) {
                assert_eq!(server.snapshot(), initial_wire);
                assert_eq!(
                    pace_database_snapshot(&sql_reader.connection, baseline.intent.as_str()),
                    confirmed_database
                );
                assert!(clock.now.get().get() < pace_due);
                assert_eq!(
                    read_local.inspect_run(&baseline.intent).unwrap().head_version(),
                    reopened_head
                );
                tokio::select! {
                    biased;
                    result = &mut prepared => panic!("TEST_CODE Web dimension two began before saved pace due: {result:?}"),
                    _ = tokio::time::sleep(Duration::from_millis(5)) => {}
                }
            }
            assert_eq!(server.snapshot(), initial_wire);
            assert_eq!(
                pace_database_snapshot(&sql_reader.connection, baseline.intent.as_str()),
                confirmed_database
            );
            assert_eq!(read_local.inspect_run(&baseline.intent).unwrap().head_version(), reopened_head);

            // Only the saved remaining 100ms is left: reaching the original due is what
            // releases dimension two. Nothing else moves the controlled clock here.
            clock.now.set(UtcMicros::try_new(pace_due).unwrap());
            let request_watchdog =
                std::time::Instant::now() + Duration::from_secs(5);
            while web_call_count(server) == 2 {
                assert!(
                    std::time::Instant::now() < request_watchdog,
                    "TEST_CODE dimension-two request did not start at original pace due"
                );
                tokio::select! {
                    biased;
                    result = &mut prepared => panic!("TEST_CODE pace runner returned before held dimension-two request: {result:?}"),
                    _ = tokio::time::sleep(Duration::from_millis(1)) => {}
                }
            }
            let due_wire = server.snapshot();
            assert_eq!(due_wire.calls.len(), initial_wire.calls.len() + 1);
            assert_eq!(
                due_wire.calls.last().unwrap().call,
                Call::Web {
                    dimension: 1,
                    provider: GeneralWebResearchProvider::SerpApi,
                }
            );
            assert!(due_wire.calls.last().unwrap().authorized);
            assert!(due_wire.calls.last().unwrap().response.is_none());
            let request = QueryRequest::decode(
                due_wire.calls.last().unwrap().request.as_slice(),
            )
            .unwrap();
            let payload = request.payload.as_ref().unwrap();
            assert_eq!(payload.schema, "market.semantic_search");
            assert_eq!(payload.schema_version, 1);
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&payload.data).unwrap(),
                serde_json::json!({
                    "provider": "SerpApi",
                    "query": QUERIES[1],
                    "limit": 3,
                })
            );

            let unknown = read_local.inspect_macro(&baseline.intent).unwrap();
            assert_eq!(unknown.plan_bytes(), plan_bytes);
            assert_eq!(unknown.plan().deadline_at().get(), deadline);
            assert_eq!(unknown.attempts().len(), confirmed_attempts.len() + 1);
            for (attempt, (key, request, response)) in unknown
                .attempts()
                .iter()
                .zip(&confirmed_attempts)
            {
                assert_eq!(attempt.query_key(), *key);
                assert_eq!(attempt.request_bytes(), request);
                assert_eq!(attempt.response_bytes(), Some(response.as_slice()));
            }
            let begun = unknown.attempts().last().unwrap();
            assert_eq!(
                begun.query_key(),
                QueryKey::Web {
                    dimension: 2,
                    candidate: 1,
                }
            );
            assert_eq!(begun.attempt_ordinal(), 1);
            assert_eq!(begun.request_bytes(), due_wire.calls.last().unwrap().request);
            assert!(begun.result_version().is_none());
            assert!(begun.response_bytes().is_none());
            assert!(unknown.has_unconfirmed_effect());
            assert!(unknown
                .query_terminal(QueryKey::Web {
                    dimension: 2,
                    candidate: 1,
                })
                .is_none());
            let unknown_database =
                pace_database_snapshot(&sql_reader.connection, baseline.intent.as_str());
            // Beginning a query that has no saved request plan appends one, and the begin
            // row references it by foreign key, so exactly one new plan is expected here --
            // for the query this reopen just began -- while every pre-existing plan row
            // must stay byte-identical. Demanding full equality would forbid the very
            // begin that the surrounding assertions (begins +1) require.
            assert_eq!(
                &unknown_database.requests[..confirmed_database.requests.len()],
                confirmed_database.requests.as_slice()
            );
            assert_eq!(
                unknown_database.requests.len(),
                confirmed_database.requests.len() + 1
            );
            assert_eq!(unknown_database.results, confirmed_database.results);
            assert_eq!(unknown_database.terminals, confirmed_database.terminals);
            assert_eq!(unknown_database.dimensions, confirmed_database.dimensions);
            assert_eq!(unknown_database.begins.len(), confirmed_database.begins.len() + 1);
            let begin_head = read_local.inspect_run(&baseline.intent).unwrap().head_version();
            assert_eq!(begin_head, begun.begin_version());
            assert_eq!(clock.observation_calls.get(), 1);
            assert_eq!(
                parent_server
                    .as_ref()
                    .unwrap()
                    .snapshot_with_tcp_for_test(),
                parent_wire
            );
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                parent_memberships
            );

            drop(prepared);
            drop(io);
            drop(read_local);
            reader.connection.close().unwrap();
            sql_reader.connection.close().unwrap();
            drop(local);
            server.release_web_response_for_test();
            let release_watchdog =
                std::time::Instant::now() + Duration::from_millis(100);
            while std::time::Instant::now() < release_watchdog {
                tokio::task::yield_now().await;
            }
            let unknown_wire = server.snapshot();
            drop(reopened_source);
            business.reopen();

            clock
                .now
                .set(UtcMicros::try_new(pace_due + 100_000).unwrap());
            let final_source = GrpcSource::from_macro_loopback_test_client(
                server.connect().await,
                server.endpoint().to_owned(),
            );
            let mut local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&baseline.config)
                .unwrap();
            let lease = local
                .resume_run(
                    &baseline.intent,
                    macro_lease(
                        "TEST_CODE_WEB_PACE_OWNER_C",
                        pace_due + 100_000,
                        started_at + 60_000_000,
                        begin_head,
                    ),
                )
                .unwrap();
            let final_head = lease.head_version();
            assert_eq!(final_head, begin_head + 1);
            let mut io = local
                .macro_preparation_io_v12(
                    lease,
                    &baseline.queries,
                    &clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &baseline.source,
                    &final_source,
                    &search_service,
                )
                .unwrap();
            let stopped = tokio::time::timeout(
                Duration::from_secs(5),
                prepare_chain_analysis_with_io(
                    NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                    baseline.stocks.clone(),
                    None,
                    &mut io,
                ),
            )
            .await
            .expect("TEST_CODE Unknown reopen stop deadline")
            .expect_err("TEST_CODE sent Web request cannot be replayed after reopen");
            assert!(matches!(
                stopped.downcast_ref::<PreparationStop>(),
                Some(PreparationStop::IncompleteOnReopen { intent_id })
                    if intent_id == baseline.intent.as_str()
            ));
            assert!(matches!(
                stopped.downcast_ref::<ChainPostCloseError>(),
                Some(ChainPostCloseError::IncompleteEffect { intent_id })
                    if intent_id == baseline.intent.as_str()
            ));
            drop(io);
            let final_recovery = local.inspect_macro(&baseline.intent).unwrap();
            assert!(final_recovery.has_unconfirmed_effect());
            assert!(!final_recovery.is_complete());
            assert_eq!(final_recovery.plan_bytes(), plan_bytes);
            assert_eq!(final_recovery.plan().deadline_at().get(), deadline);
            assert_eq!(final_recovery.attempts().len(), confirmed_attempts.len() + 1);
            for (attempt, (key, request, response)) in final_recovery
                .attempts()
                .iter()
                .zip(&confirmed_attempts)
            {
                assert_eq!(attempt.query_key(), *key);
                assert_eq!(attempt.request_bytes(), request);
                assert_eq!(attempt.response_bytes(), Some(response.as_slice()));
            }
            assert!(final_recovery.attempts().last().unwrap().result_version().is_none());
            assert_eq!(local.inspect_run(&baseline.intent).unwrap().head_version(), final_head);
            drop(local);
            assert_eq!(
                pace_database_snapshot(business.connection(), baseline.intent.as_str()),
                unknown_database
            );
            assert_eq!(server.snapshot(), unknown_wire);
            assert_eq!(
                parent_server
                    .as_ref()
                    .unwrap()
                    .snapshot_with_tcp_for_test(),
                parent_wire
            );
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                parent_memberships
            );
        },
    ))
    .catch_unwind()
    .await;

    let macro_cleanup = match macro_server.take() {
        Some(server) => {
            std::panic::AssertUnwindSafe(server.finish())
                .catch_unwind()
                .await
        }
        None => Ok(Ok(())),
    };
    let parent_cleanup = match parent_server.take() {
        Some(server) => std::panic::AssertUnwindSafe(server.finish())
            .catch_unwind()
            .await
            .map(|_| ()),
        None => Ok(()),
    };
    let database_cleanup = business.store.take().map(|store| {
        store.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        })
    });
    drop(business);
    macro_cleanup
        .expect("TEST_CODE Web pace Macro finish panic")
        .expect("TEST_CODE Web pace Macro finish");
    parent_cleanup.expect("TEST_CODE Web pace parent finish panic after join");
    if let Some(result) = database_cleanup {
        result.expect("TEST_CODE Web pace database close");
    }
    match body {
        Ok(result) => result.expect("TEST_CODE Web pace fixture watchdog"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
