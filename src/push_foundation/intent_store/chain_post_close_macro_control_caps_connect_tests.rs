use super::*;
use crate::grpc_client::client::external_control_attempt::ExternalControlKind;
use crate::grpc_client::client::external_control_loopback_fixture::ExternalMtlsMacroFixture;
use crate::grpc_client::client::ContractProfile;
use crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroControlOutcome;

const AUTHORITY: &str = "grpc-mtls:macro.test.invalid";
const STARTED_LOCAL: &str = "2026-09-14T15:31:00+08:00";

#[tokio::test]
async fn single_user_external_macro_confirmed_health_then_capabilities_connect_failure_reopens_without_reconnect(
) {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(120),
        async {
            let baseline = control_tests::setup_external_parent(
                &mut business,
                &mut parent_server,
                "TEST_CODE_RUN_EXTERNAL_MACRO_CAPABILITIES_CONNECT_UNAVAILABLE",
            )
            .await;
            external_server = Some(
                ExternalMtlsMacroFixture::bind_data_success_for_test()
                    .await
                    .expect("TEST_CODE Capabilities-connect mTLS fixture"),
            );
            let external = external_server
                .as_ref()
                .expect("TEST_CODE Capabilities-connect mTLS owner");
            let checkpoint = control_recovery_tests::reach_confirmed_health_checkpoint(
                &mut business,
                &baseline,
                external,
                "TEST_CODE_EXTERNAL_CAPABILITIES_CONNECT_OWNER_A",
            )
            .await;
            let database = business.database();
            let started_at = micros(STARTED_LOCAL);
            let registered = [
                GeneralWebResearchProvider::SerpApi,
                GeneralWebResearchProvider::Bocha,
                GeneralWebResearchProvider::Tavily,
            ];
            let search_service = macro_search_service(&registered);

            external.set_reject_new_connections_for_test(true);
            drop(baseline.queries);
            drop(baseline.source);
            business.reopen();
            let macro_source = GrpcSource::from_external_macro_bundle_for_test(
                external.bundle_path().to_path_buf(),
            );
            let parent_source = GrpcSource::from_board_loopback_test_client(
                connect_parent_instance(&baseline.endpoint).await,
            );
            let queries = parent_source.connected_board_queries().await.unwrap();
            let clock = MacroClock {
                now: Cell::new(UtcMicros::try_new(started_at + 3_000_000).unwrap()),
                observation: DateTime::parse_from_rfc3339("2026-09-14T15:32:00+08:00")
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
                        "TEST_CODE_EXTERNAL_CAPABILITIES_CONNECT_OWNER_B",
                        started_at + 3_000_000,
                        started_at + 5_000_000,
                        checkpoint.head_version,
                    ),
                )
                .unwrap();
            let owner_b_baseline = local.inspect_run(&baseline.intent).unwrap().head_version();
            assert!(owner_b_baseline > checkpoint.head_version);
            let mut io = local
                .macro_preparation_io_v11(
                    lease,
                    &queries,
                    &clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &parent_source,
                    &macro_source,
                    &search_service,
                )
                .unwrap();
            let stopped = match tokio::time::timeout(
                Duration::from_secs(5),
                prepare_chain_analysis_with_io(
                    NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                    baseline.stocks.clone(),
                    None,
                    &mut io,
                ),
            )
            .await
            {
                Ok(Err(error)) => error,
                Ok(Ok(_)) => panic!("TEST_CODE Capabilities-connect unexpectedly completed Macro"),
                Err(_) => panic!("TEST_CODE Capabilities-connect prepare watchdog"),
            };
            assert_partial_macro_stop(&stopped);
            drop(io);
            let terminal = local.inspect_macro(&baseline.intent).unwrap();
            assert!(!terminal.is_complete());
            assert!(!terminal.has_unconfirmed_effect());
            assert!(terminal.attempts().is_empty());
            assert_eq!(terminal.parent_final_bytes(), baseline.final_bytes);
            assert_eq!(terminal.plan_bytes(), checkpoint.plan_bytes);
            let plan = terminal.plan();
            assert_eq!(plan.profile(), ContractProfile::ExternalV1);
            assert_eq!(plan.acquisition_authority(), Some(AUTHORITY));
            assert_eq!(plan.endpoint(), external.endpoint());
            assert_eq!(plan.started_at().get(), started_at);
            assert_eq!(plan.deadline_at().get(), started_at + 15_000_000);
            assert_eq!(plan.observed_local(), STARTED_LOCAL);
            let data_request = plan.first_source_request();
            assert_eq!(data_request.request_id(), checkpoint.data.id);
            assert_eq!(data_request.request_bytes(), checkpoint.data.bytes);
            assert_eq!(data_request.retry_policy(), (4, 1000, 60_000, 200));

            let episodes = terminal.readiness_episodes();
            assert_eq!(episodes.len(), 1);
            let episode = &episodes[0];
            assert_eq!(episode.episode_ordinal(), 1);
            assert_eq!(
                episode.initiating_source(),
                &MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Eastmoney,
                    limit: 20,
                }
            );
            assert_eq!(episode.ready_result_version(), None);
            let controls = episode.controls();
            assert_eq!(controls.len(), 2);
            let health = &controls[0];
            assert_eq!(health.kind(), ExternalControlKind::Health);
            assert_eq!(health.request_id(), checkpoint.health.request.id);
            assert_eq!(health.request_bytes(), checkpoint.health.request.bytes);
            assert_eq!(health.begin_version(), Some(checkpoint.health.begin_version));
            assert_eq!(health.result_version(), Some(checkpoint.health.result_version));
            assert_eq!(health.outcome(), Some(MacroControlOutcome::Ready));
            assert_eq!(
                health.response_bytes(),
                Some(checkpoint.health.response_bytes.as_slice())
            );
            let capabilities = &controls[1];
            assert_eq!(capabilities.kind(), ExternalControlKind::Capabilities);
            assert_eq!(capabilities.request_id(), checkpoint.capabilities.id);
            assert_eq!(capabilities.request_bytes(), checkpoint.capabilities.bytes);
            let capabilities_begin = capabilities.begin_version().unwrap();
            let capabilities_result = capabilities.result_version().unwrap();
            assert!(capabilities_begin > checkpoint.health.result_version);
            assert!(capabilities_begin > owner_b_baseline);
            assert!(capabilities_result > capabilities_begin);
            assert_eq!(capabilities.outcome(), Some(MacroControlOutcome::Rejected));
            assert_eq!(capabilities.response_bytes(), None);
            let terminal_head = local.inspect_run(&baseline.intent).unwrap().head_version();
            assert_eq!(capabilities_result + 1, terminal_head);
            assert_eq!(local.inspect_run(&baseline.intent).unwrap().lease_generation(), 4);
            assert_eq!(
                local.inspect_run(&baseline.intent).unwrap().context().canonical_bytes(),
                baseline.context
            );
            assert_eq!(clock.observation_calls.get(), 0);

            let health_raw = control_tests::raw_control_bytes(&database, &baseline.intent, 1);
            assert_eq!(health_raw, checkpoint.raw);
            control_tests::assert_response_raw(&health_raw, &checkpoint.health.response_bytes);
            let capabilities_raw =
                control_tests::raw_control_bytes(&database, &baseline.intent, 2);
            control_tests::assert_connect_unavailable_raw(&capabilities_raw);
            let receipt = control_tests::assert_connect_unavailable_source(&terminal);
            assert_eq!(
                terminal.pending_source_identities(),
                control_tests::pending_sources().as_slice()
            );
            assert_eq!(
                terminal.pending_research_queries(),
                control_tests::pending_research().as_slice()
            );
            drop(local);
            assert_eq!(business.count("chain_post_close_macro_source_finals"), 1);
            let terminal_audit = control_tests::audit_snapshot_at(&database);
            assert_eq!(terminal_audit.len(), baseline.audit.len() + 1);
            assert_eq!(&terminal_audit[..baseline.audit.len()], baseline.audit.as_slice());
            control_tests::assert_connect_unavailable_audit(
                business.connection(),
                &receipt,
                &baseline.receipt,
                "2026-09-14T07:31:03+00:00",
            );
            assert_eq!(old_fact_rows(business.connection(), &baseline.tables), baseline.facts);
            assert_eq!(parent_server.as_ref().unwrap().snapshot(), baseline.network);
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                baseline.memberships
            );
            let first_wire = external.snapshot();
            assert_eq!(first_wire.tcp_accepts, 2);
            assert_eq!(
                first_wire.health_requests,
                vec![checkpoint.health.request.bytes.clone()]
            );
            assert_eq!(first_wire.health_authorized, vec![true]);
            assert_eq!(
                first_wire.health_responses,
                vec![checkpoint.health.response_bytes.clone()]
            );
            assert!(first_wire.health_statuses.is_empty());
            assert_eq!(first_wire.capabilities_calls, 0);
            assert!(first_wire.capabilities_requests.is_empty());
            assert!(first_wire.capabilities_responses.is_empty());
            assert!(first_wire.capabilities_statuses.is_empty());
            assert_eq!(first_wire.data_calls, 0);
            assert!(first_wire.data_requests.is_empty());
            assert!(first_wire.data_responses.is_empty());
            assert!(first_wire.data_statuses.is_empty());

            drop(queries);
            drop(parent_source);
            drop(macro_source);
            external.set_reject_new_connections_for_test(false);
            external.release_health();
            external.release_capabilities();
            external.release_data();
            business.reopen();
            let reopened_macro_source = GrpcSource::from_external_macro_bundle_for_test(
                external.bundle_path().to_path_buf(),
            );
            let reopened_parent_source = GrpcSource::from_board_loopback_test_client(
                connect_parent_instance(&baseline.endpoint).await,
            );
            let reopened_queries = reopened_parent_source.connected_board_queries().await.unwrap();
            let reopened_clock = MacroClock {
                now: Cell::new(UtcMicros::try_new(started_at + 6_000_000).unwrap()),
                observation: DateTime::parse_from_rfc3339("2026-09-14T15:32:00+08:00")
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
                        "TEST_CODE_EXTERNAL_CAPABILITIES_CONNECT_OWNER_C",
                        started_at + 6_000_000,
                        started_at + 10_000_000,
                        terminal_head,
                    ),
                )
                .unwrap();
            let owner_c_baseline = local.inspect_run(&baseline.intent).unwrap().head_version();
            assert!(owner_c_baseline > terminal_head);
            let mut io = local
                .macro_preparation_io_v11(
                    lease,
                    &reopened_queries,
                    &reopened_clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &reopened_parent_source,
                    &reopened_macro_source,
                    &search_service,
                )
                .unwrap();
            let reopened_stop = match tokio::time::timeout(
                Duration::from_secs(5),
                prepare_chain_analysis_with_io(
                    NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                    baseline.stocks,
                    None,
                    &mut io,
                ),
            )
            .await
            {
                Ok(Err(error)) => error,
                Ok(Ok(_)) => {
                    panic!("TEST_CODE Capabilities-connect reopen unexpectedly completed Macro")
                }
                Err(_) => panic!("TEST_CODE Capabilities-connect reopen watchdog"),
            };
            assert_partial_macro_stop(&reopened_stop);
            drop(io);
            let reopened = local.inspect_macro(&baseline.intent).unwrap();
            control_tests::assert_same_terminal_recovery(&terminal, &reopened);
            assert_eq!(local.inspect_run(&baseline.intent).unwrap().head_version(), owner_c_baseline);
            assert_eq!(local.inspect_run(&baseline.intent).unwrap().lease_generation(), 5);
            assert_eq!(
                local.inspect_run(&baseline.intent).unwrap().context().canonical_bytes(),
                baseline.context
            );
            assert_eq!(reopened_clock.observation_calls.get(), 0);
            assert_eq!(
                reopened.pending_source_identities(),
                control_tests::pending_sources().as_slice()
            );
            assert_eq!(
                reopened.pending_research_queries(),
                control_tests::pending_research().as_slice()
            );
            drop(local);

            assert_eq!(business.count("chain_post_close_macro_source_finals"), 1);
            assert_eq!(control_tests::audit_snapshot_at(&database), terminal_audit);
            assert_eq!(
                control_tests::raw_control_bytes(&database, &baseline.intent, 1),
                health_raw
            );
            assert_eq!(
                control_tests::raw_control_bytes(&database, &baseline.intent, 2),
                capabilities_raw
            );
            assert_eq!(external.snapshot(), first_wire);
            assert_eq!(old_fact_rows(business.connection(), &baseline.tables), baseline.facts);
            assert_eq!(parent_server.as_ref().unwrap().snapshot(), baseline.network);
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                baseline.memberships
            );
            control_tests::assert_connect_unavailable_audit(
                business.connection(),
                &receipt,
                &baseline.receipt,
                "2026-09-14T07:31:03+00:00",
            );
            drop(reopened_queries);
            drop(reopened_parent_source);
            drop(reopened_macro_source);
        },
    ))
    .catch_unwind()
    .await;

    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        "TEST_CODE Capabilities-connect",
    )
    .await;
    drop(business);
    match body {
        Ok(result) => result.expect("TEST_CODE Capabilities-connect body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
