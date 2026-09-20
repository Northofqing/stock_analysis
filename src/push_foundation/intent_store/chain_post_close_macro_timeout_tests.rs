use super::*;

#[tokio::test(flavor = "current_thread")]
async fn single_user_local_macro_timeout_with_armed_effects_stops_unconfirmed() {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut macro_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(90),
        async {
            let baseline = control_tests::setup_external_parent(
                &mut business,
                &mut parent_server,
                "TEST_CODE_RUN_MACRO_TIMEOUT_ARMED",
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
                        "TEST_CODE_MACRO_TIMEOUT_OWNER",
                        started_at,
                        started_at + 300_000,
                        baseline.head,
                    ),
                )
                .unwrap();
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
            let prepared = prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                baseline.stocks.clone(),
                None,
                &mut io,
            );
            tokio::pin!(prepared);
            // Hold every Gateway request and never release it: each effect is begun
            // with its result unconfirmed, which is exactly the state the budget
            // contract says must not be degraded to an empty background.
            tokio::select! {
                result = &mut prepared => panic!(
                    "TEST_CODE macro returned before five Gateway requests were held: {result:?}; calls={:?}",
                    server.snapshot().calls.iter().map(|call| call.call.clone()).collect::<Vec<_>>()
                ),
                _ = server.wait_for_gateway_count(5) => {}
            }
            let held = server.snapshot();
            assert_eq!(held.calls.len(), 5);
            assert!(held.calls.iter().all(|call|
                matches!(call.call, Call::Gateway(_))
                    && call.authorized
                    && call.response.is_none()));

            // Let the macro budget expire with five armed effects still outstanding.
            // The run must hard-stop as unconfirmed; today it silently becomes an
            // empty background and preparation continues to ModelsSearchAndReport,
            // which is the defect this test exists to pin.
            let stopped = prepared.await.expect_err(
                "TEST_CODE armed macro budget expiry must not degrade to an empty background",
            );
            assert!(
                matches!(
                    stopped.downcast_ref::<PreparationStop>(),
                    Some(PreparationStop::ResultUnconfirmed { intent_id })
                        if intent_id == baseline.intent.as_str()
                ),
                "TEST_CODE armed macro expiry must stop ResultUnconfirmed; actual={stopped:?}",
            );
            let failure = stopped
                .downcast_ref::<PreparationFailure>()
                .expect("TEST_CODE stop retains preparation observations");
            assert_eq!(failure.stage(), PreparationStage::Macro);
            assert!(
                !failure.completed_stages().contains(&PreparationStage::Macro),
                "TEST_CODE an unconfirmed macro must not be recorded as completed",
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
        .expect("TEST_CODE macro timeout Macro finish panic")
        .expect("TEST_CODE macro timeout Macro finish");
    parent_cleanup.expect("TEST_CODE macro timeout parent finish panic after join");
    if let Some(result) = database_cleanup {
        result.expect("TEST_CODE macro timeout database close");
    }
    match body {
        Ok(result) => result.expect("TEST_CODE macro timeout fixture watchdog"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
