use super::*;
use crate::data_gateway::{GeneralWebResearchProvider, GlobalNewsProvider, GlobalNewsRecord};
use crate::grpc_client::client::board_loopback_fixture::spawn_macro_parent_listener;
use crate::grpc_client::client::macro_attempt::{MacroContinuation, MacroQueryIdentity};
use crate::grpc_client::client::macro_loopback_fixture::{
    connect_parent_instance, MacroLoopbackServer, NEWS_RECORDS,
};
use crate::grpc_client::pb::magic::market::v1::{Operation, QueryRequest, QueryResponse};
use crate::market_domain::SourceEvidence;
use crate::pipeline::chain_analysis::preparation::MacroObservationClock;
use crate::push_foundation::intent_store::chain_post_close::macro_codec;
use crate::search_service::SearchService;
use futures::FutureExt as _;

pub(super) fn macro_search_service(
    registered: &[GeneralWebResearchProvider],
) -> SearchService {
    SearchService::from_general_web_providers_for_test(registered)
}

struct MacroClock {
    now: Cell<UtcMicros>,
    observation: DateTime<chrono::FixedOffset>,
    observation_calls: Cell<usize>,
}

impl ConceptEffectClock for MacroClock {
    fn now(&self) -> UtcMicros {
        self.now.get()
    }
}

impl PositionObservationClock for MacroClock {
    fn cache_observation(&self) -> PositionCacheObservation {
        panic!("TEST_CODE Macro must recover the sealed position parent")
    }
}

impl DragonTigerObservationClock for MacroClock {
    fn dragon_tiger_request_observation(&self) -> DateTime<chrono::FixedOffset> {
        panic!("TEST_CODE Macro must recover the sealed DragonTiger parent")
    }
}

impl MacroObservationClock for MacroClock {
    fn macro_request_observation(&self) -> DateTime<chrono::FixedOffset> {
        self.observation_calls.set(self.observation_calls.get() + 1);
        self.observation
    }
}

fn macro_lease(owner: &str, now: i64, until: i64, head: u64) -> RunLeaseRequest {
    RunLeaseRequest::try_new(
        LeaseOwnerId::try_new(owner.to_owned()).unwrap(),
        UtcMicros::try_new(now).unwrap(),
        UtcMicros::try_new(until).unwrap(),
        Some(head),
    )
    .unwrap()
}

fn assert_partial_macro_stop(error: &anyhow::Error) {
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::Macro
        })
    ));
    let failure = error.downcast_ref::<PreparationFailure>().unwrap();
    assert_eq!(failure.stage(), PreparationStage::Macro);
    assert_eq!(
        failure.completed_stages().last(),
        Some(&PreparationStage::DragonTiger)
    );
    assert!(!failure
        .completed_stages()
        .contains(&PreparationStage::Macro));
    assert_eq!(
        failure.lhb_map()["TEST_CODE_600001"].to_bits(),
        12.5_f64.to_bits()
    );
    assert_eq!(
        failure.lhb_source().source(),
        Some("TEST_CODE_LHB_SOURCE_FIRST")
    );
}

fn assert_native_news(batch: &GatewayBatch<GlobalNewsRecord>) {
    assert!(!batch.is_verified_empty());
    let evidence = batch.evidence();
    assert_eq!(evidence.provider, ProviderId::Eastmoney);
    assert_eq!(evidence.source, "eastmoney-web");
    assert_eq!(
        evidence.source_at.as_deref(),
        Some("2026-09-14T15:30:00.123456789+08:00")
    );
    assert_eq!(evidence.observed_at, "2026-09-14T15:30:00.987654321+08:00");
    assert_eq!(evidence.batch_id, "TEST_CODE_MACRO_BATCH_FIRST");
    let record_evidence = SourceEvidence::new(
        ProviderId::Eastmoney,
        "2026-09-14T15:30:00.987654321+08:00",
        "TEST_CODE_MACRO_BATCH_FIRST",
    )
    .unwrap()
    .with_source_at("2026-09-14T15:30:00.123456789+08:00")
    .unwrap();
    let utc = |value: &str| {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&chrono::Utc)
    };
    // Handwritten native expectations, independent of the new persistence codec
    // and of fixture JSON parsing. Every current GlobalNewsRecord field is covered.
    let expected = [
        GlobalNewsRecord {
            item_id: "TEST_CODE_NEWS_一".to_owned(),
            title: "TEST_CODE 财经🌏  ".to_owned(),
            summary: Some("TEST_CODE 摘要\n第二行  ".to_owned()),
            content: None,
            publisher: "TEST_CODE 发布者甲".to_owned(),
            canonical_url: "https://example.invalid/TEST_CODE/news?x=1&y=二".to_owned(),
            published_at: utc("2026-09-14T07:29:59.123456789Z"),
            observed_at: utc("2026-09-14T07:30:00.987654321Z"),
            instruments: vec![
                "TEST_CODE_600001.SH".to_owned(),
                "TEST_CODE_000001.SZ".to_owned(),
            ],
            topics: vec!["TEST_CODE 产业".to_owned(), "TEST_CODE 政策".to_owned()],
            language: "zh-CN".to_owned(),
            evidence: record_evidence.clone(),
        },
        GlobalNewsRecord {
            item_id: "TEST_CODE_NEWS_二".to_owned(),
            title: "TEST_CODE second headline".to_owned(),
            summary: Some(String::new()),
            content: Some("TEST_CODE 正文\t末尾  ".to_owned()),
            publisher: "TEST_CODE publisher B".to_owned(),
            canonical_url: "https://example.invalid/TEST_CODE/second".to_owned(),
            published_at: utc("2026-09-14T07:29:58.987654321Z"),
            observed_at: utc("2026-09-14T07:30:00.987654321Z"),
            instruments: Vec::new(),
            topics: Vec::new(),
            language: "en".to_owned(),
            evidence: record_evidence,
        },
    ];
    assert_eq!(batch.records(), expected.as_slice());
}

#[tokio::test]
async fn single_user_local_macro_first_global_news_reopens_without_rpc_or_audit() {
    // These owners outlive the caught body, including failures while constructing
    // clients, seeding the real parent, reopening SQLite or asserting results.
    let mut fixture = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut macro_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), async {
        let (parent_endpoint, server) = tokio::time::timeout(
            Duration::from_secs(5), spawn_macro_parent_listener(DRAGON_TIGER_RECORDS.as_bytes()),
        ).await.expect("TEST_CODE parent listener deadline");
        parent_server = Some(server);
        macro_server = Some(MacroLoopbackServer::bind().await);
        let server = macro_server.as_ref().unwrap();
        let parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let queries = parent_source.connected_board_queries().await.unwrap();
        let (stocks, config, intent, v9_head) = populate_completed_v9_parent(
            &mut fixture, &queries, "TEST_CODE_RUN_MACRO_FIRST_GLOBAL_NEWS",
        ).await;
        fixture.chain_post_close().migrate_schema_v9_to_v10().unwrap();
        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(&intent, lease_request(
            "TEST_CODE_MACRO_PARENT_OWNER", 68_300_000_000, 90_000_000_000, Some(v9_head),
        )).unwrap();
        let parent_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-22T10:30:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-22T10:30:00+08:00").unwrap(),
            request_calls: Cell::new(0), cache_calls: Cell::new(0),
        };
        let mut io = local.dragon_tiger_preparation_io_v10(
            lease, &queries, &parent_clock, FixedClusterConfiguration::resolve(Some("2")),
            &parent_source,
        ).unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks.clone(), None, &mut io,
        ).await.expect_err("TEST_CODE real parent reaches Macro guard");
        assert_partial_macro_stop(&stopped);
        drop(io);
        let parent = local.inspect_dragon_tiger(&intent).unwrap();
        assert_complete_batch(parent.batch().unwrap());
        let parent_final = parent.final_bytes().unwrap().to_vec();
        let parent_receipt = parent.audit_receipt().unwrap().clone();
        let parent_run = local.inspect_run(&intent).unwrap();
        let parent_context = parent_run.context().canonical_bytes();
        let parent_head = parent_run.head_version();
        drop(local);
        let earlier_tables = table_names(fixture.connection());
        let earlier_facts = old_fact_rows(fixture.connection(), &earlier_tables);
        let audit_count = fixture.count("data_acquisition_audit");
        let old_network = parent_server.as_ref().unwrap().snapshot();
        let old_memberships = parent_server.as_ref().unwrap().membership_snapshot();
        assert_eq!(old_network.dragon_tiger_requests.len(), 1);
        let database = fixture.database();

        // RED seam: controlled v11 migration and the first durable Macro adapter.
        assert_eq!(fixture.chain_post_close().migrate_schema_v10_to_v11().unwrap().schema_version(), 11);
        let macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await, server.endpoint().to_owned(),
        );
        let started_at = micros("2026-09-14T15:30:00+08:00");
        let clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-14T15:30:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        // Explicit instance registration in original order. Local availability
        // derives from this supplied bridge, never bridge_for/global credentials.
        let registered = [GeneralWebResearchProvider::SerpApi,
            GeneralWebResearchProvider::Bocha, GeneralWebResearchProvider::Tavily];
        let search_service = macro_search_service(&registered);
        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(&intent, macro_lease(
            "TEST_CODE_MACRO_OWNER_FIRST", started_at, started_at + 2_000_000, parent_head,
        )).unwrap();
        let mut io = local.macro_preparation_io_v11(
            lease, &queries, &clock, FixedClusterConfiguration::resolve(Some("2")),
            &parent_source, &macro_source, &search_service,
        ).unwrap();
        let stopped = {
            let prepared = prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks.clone(), None, &mut io,
            );
            tokio::pin!(prepared);
            tokio::select! {
                result = &mut prepared => panic!("TEST_CODE prepare returned before gated source: {result:?}"),
                _ = server.wait_for_first_request() => {}
            }
            // A distinct reader of the same business file proves COMMIT before
            // the remote response, through the typed recovery boundary.
            let mut reader = BusinessIntentStore::open(&database).unwrap();
            let mut read_local = reader.single_user_local_chain_post_close(&config).unwrap();
            let begun = read_local.inspect_macro(&intent).unwrap();
            assert!(begun.has_unconfirmed_effect());
            assert!(!begun.is_complete());
            assert_eq!(begun.attempts().len(), 1);
            assert_eq!(begun.attempts()[0].attempt_ordinal(), 1);
            assert!(begun.attempts()[0].response_bytes().is_none());
            assert_eq!(begun.attempts()[0].request_bytes(), server.snapshot().requests[0]);
            assert!(begun.attempts()[0].begin_version() > parent_head);
            drop(read_local);
            reader.connection.close().unwrap();
            clock.now.set(UtcMicros::try_new(started_at + 1_000_000).unwrap());
            server.release_response();
            prepared.await.expect_err("TEST_CODE remaining Macro sources stay pending")
        };
        assert_partial_macro_stop(&stopped);
        drop(io);
        let recovered = local.inspect_macro(&intent).unwrap();
        assert!(!recovered.is_complete());
        assert!(!recovered.has_unconfirmed_effect());
        assert_eq!(recovered.parent_final_bytes(), parent_final);
        let plan = recovered.plan();
        assert_eq!(plan.started_at().get(), started_at);
        assert_eq!(plan.deadline_at().get(), started_at + 15_000_000);
        assert_eq!(plan.observed_local(), "2026-09-14T15:30:00+08:00");
        assert_eq!(plan.source_identities(), &[
            MacroQueryIdentity::GlobalNews { provider: GlobalNewsProvider::Eastmoney, limit: 20 },
            MacroQueryIdentity::GlobalNews { provider: GlobalNewsProvider::Cailianpress, limit: 20 },
            MacroQueryIdentity::GlobalNews { provider: GlobalNewsProvider::Jin10, limit: 20 },
            MacroQueryIdentity::GlobalNews { provider: GlobalNewsProvider::ThePaper, limit: 20 },
            MacroQueryIdentity::EconomicCalendar,
        ]);
        assert_eq!(plan.economic_intent(), (20, None));
        assert_eq!(plan.research_queries(), &[
            "2026年09月14日A股 大盘 股市 最新动态",
            "2026年09月14日国际财经 地缘政治 最新消息",
            "2026年09月14日美股 美联储 大宗商品 今日",
            "2026年09月14日中国 央行 财政 产业政策 重要新闻",
            "2026年09月14日高盛 摩根 大摩 美银 JPMorgan 中国A股 市场观点 研报",
            "2026年09月14日证券时报 第一财经 21世纪经济报道 重要财经",
        ]);
        assert_eq!(plan.research_limit(), 3);
        assert_eq!(plan.research_providers(), registered);
        assert!(plan.research_decisions().iter().all(|decision|
            decision.supports_general_web_search() && decision.is_available()));
        assert_eq!(plan.research_decisions().len(), 3);
        assert_eq!(
            plan.research_decision_provenance(),
            macro_codec::ResearchDecisionProvenance::ExplicitRegistryV2
        );
        for (index, decision) in plan.research_decisions().iter().enumerate() {
            assert_eq!(
                decision.registration_ordinal(),
                Some(u32::try_from(index + 1).unwrap())
            );
            assert_eq!(
                decision.availability_source(),
                "explicit-registry-local-semantic-search-connected"
            );
            assert_eq!(decision.local_transport_endpoint(), Some(plan.endpoint()));
            assert_eq!(decision.remote_health(), Some("Unknown"));
        }
        assert_eq!(plan.gateway_pace_ms(), 200);
        assert_eq!(plan.query_pace_ms(), 300);
        assert_eq!(recovered.pending_source_identities(), &plan.source_identities()[1..]);
        assert_eq!(recovered.pending_research_queries(), plan.research_queries());
        let source = recovered.global_news(GlobalNewsProvider::Eastmoney).unwrap();
        assert!(source.is_complete());
        assert_eq!(source.profile(), "LocalBridgeV1");
        assert_eq!(source.acquisition_authority(), None);
        assert_eq!(source.retry_policy(), (4, 1000, 60_000, 200));
        assert_native_news(source.batch().unwrap());
        let receipt = source.audit_receipt().unwrap().clone();
        let first_plan_bytes = recovered.plan_bytes().to_vec();
        let first_native_bytes = source.final_bytes().unwrap().to_vec();
        let attempts = recovered.attempts();
        assert_eq!(attempts.len(), 1);
        let result_version = attempts[0].result_version().unwrap();
        assert!(result_version > attempts[0].begin_version());
        assert_eq!(attempts[0].continuation(), Some(MacroContinuation::Terminal));
        let first_request_bytes = attempts[0].request_bytes().to_vec();
        let first_response_bytes = attempts[0].response_bytes().unwrap().to_vec();
        let first_head = local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 3);
        assert_eq!(clock.observation_calls.get(), 1);
        drop(local);

        let wire = server.snapshot();
        assert_eq!(wire.requests.len(), 1);
        assert_eq!(wire.authorized, vec![true]);
        assert_eq!(wire.responses, vec![first_response_bytes.clone()]);
        assert_eq!(wire.requests, vec![first_request_bytes.clone()]);
        assert_eq!((wire.health_calls, wire.capabilities_calls), (0, 0));
        assert!(wire.unexpected_data_calls.is_empty());
        let request = QueryRequest::decode(first_request_bytes.as_slice()).unwrap();
        let context = request.context.unwrap();
        assert_eq!(context.protocol_version, 1);
        assert!(!context.request_id.is_empty());
        assert!(request.preferred_provider.is_empty());
        assert!(!request.allow_unadmitted);
        let payload = request.payload.unwrap();
        assert_eq!(payload.schema, "news.global_news");
        assert_eq!(payload.schema_version, 1);
        assert_eq!(payload.content_type, "application/json; charset=utf-8");
        assert_eq!(payload.data, br#"{"limit":20,"provider":"Eastmoney"}"#);
        let response = QueryResponse::decode(first_response_bytes.as_slice()).unwrap();
        assert_eq!(response.request_id, context.request_id);
        assert_eq!(response.operation, Operation::GlobalNews as i32);
        assert_eq!(response.records.len(), 1);
        assert_eq!(response.records[0].data, NEWS_RECORDS.as_bytes());
        assert!(!first_plan_bytes.windows(b"TEST_CODE_MACRO_SOURCE_TOKEN".len())
            .any(|part| part == b"TEST_CODE_MACRO_SOURCE_TOKEN"));
        assert!(!first_request_bytes.windows(b"TEST_CODE_MACRO_SOURCE_TOKEN".len())
            .any(|part| part == b"TEST_CODE_MACRO_SOURCE_TOKEN"));

        let transaction = fixture.connection().unchecked_transaction().unwrap();
        let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
        let audit = verified.record();
        assert_eq!(audit.capability, "GlobalNews-Eastmoney");
        assert_eq!(audit.provider, "Eastmoney");
        assert_eq!(audit.source, "eastmoney-web");
        // Independent Ruby Digest SHA256 of the literal BR159 preimage.
        assert_eq!(audit.request_hash, "fb86badeeebfca14c04928026fe295416a2a47c6534250291c066f3a74b67b9c");
        assert_eq!(audit.source_at, Some("2026-09-14T15:30:00.123456789+08:00"));
        assert_eq!(audit.observed_at, "2026-09-14T15:30:00.987654321+08:00");
        assert_eq!(audit.batch_id, Some("TEST_CODE_MACRO_BATCH_FIRST"));
        assert_eq!(audit.outcome, "available");
        assert_eq!((audit.request_count, audit.accepted_count, audit.rejected_count), (1, 2, 0));
        assert_eq!(audit.reason_code, "accepted");
        assert!(!audit.retryable);
        transaction.commit().unwrap();
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count + 1);

        drop(queries);
        drop(parent_source);
        drop(macro_source);
        fixture.reopen(); // Closes the actual owned SQLite connection, then opens the same file.
        server.change_response_for_reopen();
        let changed_macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await, server.endpoint().to_owned(),
        );
        let changed_parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let changed_queries = changed_parent_source.connected_board_queries().await.unwrap();
        let changed_clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at + 3_000_000).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-15T15:45:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(&intent, macro_lease(
            "TEST_CODE_MACRO_OWNER_REOPENED", started_at + 3_000_000,
            started_at + 10_000_000, first_head,
        )).unwrap();
        let mut io = local.macro_preparation_io_v11(
            lease, &changed_queries, &changed_clock, FixedClusterConfiguration::resolve(Some("2")),
            &changed_parent_source, &changed_macro_source, &search_service,
        ).unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks, None, &mut io,
        ).await.expect_err("TEST_CODE recovered first source does not complete the Macro plan");
        assert_partial_macro_stop(&stopped);
        drop(io);
        let reopened = local.inspect_macro(&intent).unwrap();
        assert_eq!(reopened.plan_bytes(), first_plan_bytes);
        assert_eq!(reopened.plan().started_at().get(), started_at);
        assert_eq!(reopened.plan().deadline_at().get(), started_at + 15_000_000);
        assert_eq!(reopened.parent_final_bytes(), parent_final);
        assert!(!reopened.is_complete());
        assert!(!reopened.has_unconfirmed_effect());
        assert_eq!(reopened.pending_source_identities(), recovered.pending_source_identities());
        assert_eq!(reopened.pending_research_queries(), recovered.pending_research_queries());
        assert_eq!(reopened.attempts().len(), 1);
        assert_eq!(reopened.attempts()[0].request_bytes(), first_request_bytes);
        assert_eq!(reopened.attempts()[0].response_bytes().unwrap(), first_response_bytes);
        assert_eq!(reopened.attempts()[0].result_version(), Some(result_version));
        let source = reopened.global_news(GlobalNewsProvider::Eastmoney).unwrap();
        assert_native_news(source.batch().unwrap());
        assert_eq!(source.final_bytes().unwrap(), first_native_bytes);
        assert_eq!(source.audit_receipt(), Some(&receipt));
        assert_eq!(changed_clock.observation_calls.get(), 0);
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 4);
        assert_eq!(local.inspect_run(&intent).unwrap().context().canonical_bytes(), parent_context);
        drop(local);
        assert_eq!(server.snapshot(), wire);
        assert_eq!(parent_server.as_ref().unwrap().snapshot(), old_network);
        assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), old_memberships);
        assert_eq!(old_fact_rows(fixture.connection(), &earlier_tables), earlier_facts);
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count + 1);
        let transaction = fixture.connection().unchecked_transaction().unwrap();
        read_acquisition_in_transaction(&transaction, &receipt).unwrap();
        read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
        transaction.commit().unwrap();
        drop(changed_queries);
        drop(changed_parent_source);
        drop(changed_macro_source);
    })).catch_unwind().await;

    // The body future (and every borrowed IO/client/reader) has been dropped.
    // Join both owners even if one cleanup fails; the database root is still live.
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
    let database_cleanup = fixture.store.take().map(|store| {
        store.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        })
    });
    drop(fixture);
    // All owned tasks have terminated before any body failure is reported.
    macro_cleanup
        .expect("TEST_CODE Macro cleanup panic")
        .expect("TEST_CODE Macro cleanup");
    parent_cleanup.expect("TEST_CODE parent cleanup panic after join");
    if let Some(result) = database_cleanup {
        result.expect("TEST_CODE business connection close");
    }
    match body {
        Ok(result) => result.expect("TEST_CODE first Macro integration body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn single_user_local_macro_cancelled_after_receipt_reopens_unknown_without_rpc_or_audit() {
    // These owners outlive the caught body, including failures while constructing
    // clients, seeding the real parent, reopening SQLite or asserting results.
    let mut fixture = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut macro_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), async {
        let (parent_endpoint, server) = tokio::time::timeout(
            Duration::from_secs(5), spawn_macro_parent_listener(DRAGON_TIGER_RECORDS.as_bytes()),
        ).await.expect("TEST_CODE parent listener deadline");
        parent_server = Some(server);
        macro_server = Some(MacroLoopbackServer::bind().await);
        let server = macro_server.as_ref().unwrap();
        let parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let queries = parent_source.connected_board_queries().await.unwrap();
        let (stocks, config, intent, v9_head) = populate_completed_v9_parent(
            &mut fixture, &queries, "TEST_CODE_RUN_MACRO_CANCEL_AFTER_RECEIPT",
        ).await;
        fixture.chain_post_close().migrate_schema_v9_to_v10().unwrap();
        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(&intent, lease_request(
            "TEST_CODE_MACRO_PARENT_OWNER", 68_300_000_000, 90_000_000_000, Some(v9_head),
        )).unwrap();
        let parent_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-22T10:30:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-22T10:30:00+08:00").unwrap(),
            request_calls: Cell::new(0), cache_calls: Cell::new(0),
        };
        let mut io = local.dragon_tiger_preparation_io_v10(
            lease, &queries, &parent_clock, FixedClusterConfiguration::resolve(Some("2")),
            &parent_source,
        ).unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks.clone(), None, &mut io,
        ).await.expect_err("TEST_CODE real parent reaches Macro guard");
        assert_partial_macro_stop(&stopped);
        drop(io);
        let parent = local.inspect_dragon_tiger(&intent).unwrap();
        assert_complete_batch(parent.batch().unwrap());
        let parent_final = parent.final_bytes().unwrap().to_vec();
        let parent_receipt = parent.audit_receipt().unwrap().clone();
        let parent_run = local.inspect_run(&intent).unwrap();
        let parent_context = parent_run.context().canonical_bytes();
        let parent_head = parent_run.head_version();
        drop(local);
        let earlier_tables = table_names(fixture.connection());
        let earlier_facts = old_fact_rows(fixture.connection(), &earlier_tables);
        let audit_count = fixture.count("data_acquisition_audit");
        let old_network = parent_server.as_ref().unwrap().snapshot();
        let old_memberships = parent_server.as_ref().unwrap().membership_snapshot();
        assert_eq!(old_network.dragon_tiger_requests.len(), 1);
        let database = fixture.database();

        // The same owned store and real first-source adapter used by the positive test.
        assert_eq!(fixture.chain_post_close().migrate_schema_v10_to_v11().unwrap().schema_version(), 11);
        let macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await, server.endpoint().to_owned(),
        );
        let started_at = micros("2026-09-14T15:30:00+08:00");
        let clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-14T15:30:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        // Explicit instance registration in original order. Local availability
        // derives from this supplied bridge, never bridge_for/global credentials.
        let registered = [GeneralWebResearchProvider::SerpApi,
            GeneralWebResearchProvider::Bocha, GeneralWebResearchProvider::Tavily];
        let search_service = macro_search_service(&registered);
        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(&intent, macro_lease(
            "TEST_CODE_MACRO_CANCEL_OWNER", started_at, started_at + 2_000_000, parent_head,
        )).unwrap();
        let mut io = local.macro_preparation_io_v11(
            lease, &queries, &clock, FixedClusterConfiguration::resolve(Some("2")),
            &parent_source, &macro_source, &search_service,
        ).unwrap();
        let (first_plan_bytes, first_request_bytes, begin_version, begin_head) = {
            // The owned future lives in this lexical scope. Exiting the scope
            // drops it, not merely the Pin<&mut _> made by tokio::pin!.
            let prepared = prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks.clone(), None, &mut io,
            );
            tokio::pin!(prepared);
            tokio::select! {
                result = &mut prepared => panic!("TEST_CODE prepare returned before receipt: {result:?}"),
                _ = server.wait_for_first_request() => {}
            }
            let wire = server.snapshot();
            assert_eq!(wire.requests.len(), 1);
            assert_eq!(wire.authorized, vec![true]);
            assert!(wire.responses.is_empty());
            assert_eq!((wire.health_calls, wire.capabilities_calls), (0, 0));
            assert!(wire.unexpected_data_calls.is_empty());
            // A separate reader sees the real begin COMMIT while the remote
            // handler is gated; no response or result has been supplied.
            let mut reader = BusinessIntentStore::open(&database).unwrap();
            let mut read_local = reader.single_user_local_chain_post_close(&config).unwrap();
            let begun = read_local.inspect_macro(&intent).unwrap();
            assert!(begun.has_unconfirmed_effect());
            assert!(!begun.is_complete());
            assert_eq!(begun.attempts().len(), 1);
            assert_eq!(begun.attempts()[0].attempt_ordinal(), 1);
            assert_eq!(begun.attempts()[0].request_bytes(), wire.requests[0]);
            assert!(begun.attempts()[0].result_version().is_none());
            assert!(begun.attempts()[0].response_bytes().is_none());
            assert!(begun.attempts()[0].continuation().is_none());
            assert!(begun.global_news(GlobalNewsProvider::Eastmoney).is_none());
            assert_eq!(begun.pending_source_identities(), begun.plan().source_identities());
            assert_eq!(begun.pending_source_identities().len(), 5);
            assert_eq!(begun.pending_research_queries().len(), 6);
            let saved = (
                begun.plan_bytes().to_vec(),
                begun.attempts()[0].request_bytes().to_vec(),
                begun.attempts()[0].begin_version(),
                read_local.inspect_run(&intent).unwrap().head_version(),
            );
            assert!(saved.2 > parent_head);
            drop(read_local);
            reader.connection.close().unwrap();
            // Deliberately no release_response and no prepared.await.
            saved
        };
        assert_eq!(clock.observation_calls.get(), 1);
        let stopped_same_io = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks.clone(), None, &mut io,
        ).await.expect_err("TEST_CODE cancelled IO must retain armed-effect classification");
        assert!(matches!(
            stopped_same_io.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::ResultUnconfirmed { intent_id }) if intent_id == intent.as_str()
        ));
        drop(io);
        let cancelled = local.inspect_macro(&intent).unwrap();
        assert!(cancelled.has_unconfirmed_effect());
        assert!(!cancelled.is_complete());
        assert_eq!(cancelled.plan_bytes(), first_plan_bytes);
        assert_eq!(cancelled.plan().started_at().get(), started_at);
        assert_eq!(cancelled.plan().deadline_at().get(), started_at + 15_000_000);
        assert_eq!(cancelled.parent_final_bytes(), parent_final);
        assert_eq!(cancelled.attempts().len(), 1);
        assert_eq!(cancelled.attempts()[0].begin_version(), begin_version);
        assert_eq!(cancelled.attempts()[0].request_bytes(), first_request_bytes);
        assert!(cancelled.attempts()[0].result_version().is_none());
        assert!(cancelled.attempts()[0].response_bytes().is_none());
        assert!(cancelled.attempts()[0].continuation().is_none());
        assert!(cancelled.global_news(GlobalNewsProvider::Eastmoney).is_none());
        assert_eq!(local.inspect_run(&intent).unwrap().head_version(), begin_head);
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 3);
        assert_eq!(local.inspect_run(&intent).unwrap().context().canonical_bytes(), parent_context);
        drop(local);
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count);
        assert_eq!(old_fact_rows(fixture.connection(), &earlier_tables), earlier_facts);
        let cancelled_wire = server.snapshot();
        assert_eq!(cancelled_wire.requests, vec![first_request_bytes.clone()]);
        assert_eq!(cancelled_wire.authorized, vec![true]);
        assert!(cancelled_wire.responses.is_empty());
        assert_eq!((cancelled_wire.health_calls, cancelled_wire.capabilities_calls), (0, 0));
        assert!(cancelled_wire.unexpected_data_calls.is_empty());

        drop(queries);
        drop(parent_source);
        drop(macro_source);
        fixture.reopen(); // Actual close/reopen, not another handle to the original connection.
        // If replay is attempted it returns promptly with different data.
        // The original cancelled server handler may or may not finish after
        // this gate opens; its response is not evidence of a second request.
        server.change_response_for_reopen();
        let changed_macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await, server.endpoint().to_owned(),
        );
        let changed_parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let changed_queries = changed_parent_source.connected_board_queries().await.unwrap();
        let changed_clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at + 3_000_000).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-15T15:45:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(&intent, macro_lease(
            "TEST_CODE_MACRO_CANCEL_REOPENED", started_at + 3_000_000,
            started_at + 10_000_000, begin_head,
        )).unwrap();
        let resumed_head = local.inspect_run(&intent).unwrap().head_version();
        assert!(resumed_head > begin_head); // Lease reacquisition legitimately advances head.
        let mut io = local.macro_preparation_io_v11(
            lease, &changed_queries, &changed_clock, FixedClusterConfiguration::resolve(Some("2")),
            &changed_parent_source, &changed_macro_source, &search_service,
        ).unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks, None, &mut io,
        ).await.expect_err("TEST_CODE unknown received effect cannot be replayed");
        assert!(matches!(
            stopped.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::IncompleteOnReopen { intent_id }) if intent_id == intent.as_str()
        ));
        assert!(matches!(
            stopped.downcast_ref::<ChainPostCloseError>(),
            Some(ChainPostCloseError::IncompleteEffect { intent_id }) if intent_id == intent.as_str()
        ));
        let failure = stopped.downcast_ref::<PreparationFailure>().unwrap();
        assert_eq!(failure.stage(), PreparationStage::Macro);
        assert_eq!(failure.completed_stages().last(), Some(&PreparationStage::DragonTiger));
        assert!(!failure.completed_stages().contains(&PreparationStage::Macro));
        drop(io);
        let reopened = local.inspect_macro(&intent).unwrap();
        assert!(reopened.has_unconfirmed_effect());
        assert!(!reopened.is_complete());
        assert_eq!(reopened.plan_bytes(), first_plan_bytes);
        assert_eq!(reopened.plan().started_at().get(), started_at);
        assert_eq!(reopened.plan().deadline_at().get(), started_at + 15_000_000);
        assert_eq!(reopened.parent_final_bytes(), parent_final);
        assert_eq!(reopened.pending_source_identities(), cancelled.pending_source_identities());
        assert_eq!(reopened.pending_research_queries(), cancelled.pending_research_queries());
        assert_eq!(reopened.attempts().len(), 1);
        assert_eq!(reopened.attempts()[0].attempt_ordinal(), 1);
        assert_eq!(reopened.attempts()[0].begin_version(), begin_version);
        assert_eq!(reopened.attempts()[0].request_bytes(), first_request_bytes);
        assert!(reopened.attempts()[0].result_version().is_none());
        assert!(reopened.attempts()[0].response_bytes().is_none());
        assert!(reopened.attempts()[0].continuation().is_none());
        assert!(reopened.global_news(GlobalNewsProvider::Eastmoney).is_none());
        assert_eq!(changed_clock.observation_calls.get(), 0);
        assert_eq!(local.inspect_run(&intent).unwrap().head_version(), resumed_head);
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 4);
        assert_eq!(local.inspect_run(&intent).unwrap().context().canonical_bytes(), parent_context);
        drop(local);
        let reopened_wire = server.snapshot();
        assert_eq!(reopened_wire.requests, cancelled_wire.requests);
        assert_eq!(reopened_wire.requests.len(), 1);
        assert_eq!(reopened_wire.authorized, vec![true]);
        assert_eq!((reopened_wire.health_calls, reopened_wire.capabilities_calls), (0, 0));
        assert!(reopened_wire.unexpected_data_calls.is_empty());
        assert_eq!(parent_server.as_ref().unwrap().snapshot(), old_network);
        assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), old_memberships);
        assert_eq!(old_fact_rows(fixture.connection(), &earlier_tables), earlier_facts);
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count);
        let transaction = fixture.connection().unchecked_transaction().unwrap();
        read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
        transaction.commit().unwrap();
        drop(changed_queries);
        drop(changed_parent_source);
        drop(changed_macro_source);
    })).catch_unwind().await;

    // The body future (and every borrowed IO/client/reader) has been dropped.
    // Join both owners even if one cleanup fails; the database root is still live.
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
    let database_cleanup = fixture.store.take().map(|store| {
        store.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        })
    });
    drop(fixture);
    // All owned tasks have terminated before any body failure is reported.
    macro_cleanup
        .expect("TEST_CODE Macro cleanup panic")
        .expect("TEST_CODE Macro cleanup");
    parent_cleanup.expect("TEST_CODE parent cleanup panic after join");
    if let Some(result) = database_cleanup {
        result.expect("TEST_CODE business connection close");
    }
    match body {
        Ok(result) => result.expect("TEST_CODE cancelled Macro integration body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn single_user_local_macro_result_commit_failure_reopens_unknown_without_rpc_or_audit() {
    // These owners outlive the caught body, including failures while constructing
    // clients, seeding the real parent, reopening SQLite or asserting results.
    let mut fixture = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut macro_server = None;
    let mut fault_reader: Option<Connection> = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), async {
        let (parent_endpoint, server) = tokio::time::timeout(
            Duration::from_secs(5), spawn_macro_parent_listener(DRAGON_TIGER_RECORDS.as_bytes()),
        ).await.expect("TEST_CODE parent listener deadline");
        parent_server = Some(server);
        macro_server = Some(MacroLoopbackServer::bind().await);
        let server = macro_server.as_ref().unwrap();
        let parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let queries = parent_source.connected_board_queries().await.unwrap();
        let (stocks, config, intent, v9_head) = populate_completed_v9_parent(
            &mut fixture, &queries, "TEST_CODE_RUN_MACRO_RESULT_COMMIT",
        ).await;
        fixture.chain_post_close().migrate_schema_v9_to_v10().unwrap();
        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(&intent, lease_request(
            "TEST_CODE_MACRO_PARENT_OWNER", 68_300_000_000, 90_000_000_000, Some(v9_head),
        )).unwrap();
        let parent_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-22T10:30:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-22T10:30:00+08:00").unwrap(),
            request_calls: Cell::new(0), cache_calls: Cell::new(0),
        };
        let mut io = local.dragon_tiger_preparation_io_v10(
            lease, &queries, &parent_clock, FixedClusterConfiguration::resolve(Some("2")),
            &parent_source,
        ).unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks.clone(), None, &mut io,
        ).await.expect_err("TEST_CODE real parent reaches Macro guard");
        assert_partial_macro_stop(&stopped);
        drop(io);
        let parent = local.inspect_dragon_tiger(&intent).unwrap();
        assert_complete_batch(parent.batch().unwrap());
        let parent_final = parent.final_bytes().unwrap().to_vec();
        let parent_receipt = parent.audit_receipt().unwrap().clone();
        let parent_run = local.inspect_run(&intent).unwrap();
        let parent_context = parent_run.context().canonical_bytes();
        let parent_head = parent_run.head_version();
        drop(local);
        let earlier_tables = table_names(fixture.connection());
        let earlier_facts = old_fact_rows(fixture.connection(), &earlier_tables);
        let audit_count = fixture.count("data_acquisition_audit");
        let audit_tables = vec![
            "data_acquisition_audit".to_owned(),
            "data_acquisition_audit_chain".to_owned(),
        ];
        let audit_facts = old_fact_rows(fixture.connection(), &audit_tables);
        let old_network = parent_server.as_ref().unwrap().snapshot();
        let old_memberships = parent_server.as_ref().unwrap().membership_snapshot();
        assert_eq!(old_network.dragon_tiger_requests.len(), 1);
        let database = fixture.database();

        // The same owned store and real first-source adapter used by the positive test.
        assert_eq!(fixture.chain_post_close().migrate_schema_v10_to_v11().unwrap().schema_version(), 11);
        let journal: String = fixture.connection().query_row(
            "PRAGMA journal_mode=DELETE", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(journal.to_ascii_lowercase(), "delete");
        let writer_busy_ms: i64 = fixture.connection().query_row(
            "PRAGMA busy_timeout", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(writer_busy_ms, 250); // Preserve the store's existing bounded wait.
        fault_reader = Some(Connection::open_with_flags(
            &database, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ).unwrap());
        fault_reader.as_ref().unwrap().busy_timeout(Duration::from_millis(250)).unwrap();
        let macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await, server.endpoint().to_owned(),
        );
        let started_at = micros("2026-09-14T15:30:00+08:00");
        let clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-14T15:30:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        // Explicit instance registration in original order. Local availability
        // derives from this supplied bridge, never bridge_for/global credentials.
        let registered = [GeneralWebResearchProvider::SerpApi,
            GeneralWebResearchProvider::Bocha, GeneralWebResearchProvider::Tavily];
        let search_service = macro_search_service(&registered);
        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(&intent, macro_lease(
            "TEST_CODE_MACRO_COMMIT_OWNER", started_at, started_at + 2_000_000, parent_head,
        )).unwrap();
        let mut io = local.macro_preparation_io_v11(
            lease, &queries, &clock, FixedClusterConfiguration::resolve(Some("2")),
            &parent_source, &macro_source, &search_service,
        ).unwrap();
        let ((first_plan_bytes, first_request_bytes, begin_version, begin_head), commit_error) = {
            let prepared = prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks.clone(), None, &mut io,
            );
            tokio::pin!(prepared);
            tokio::select! {
                result = &mut prepared => panic!("TEST_CODE prepare returned before gated COMMIT test: {result:?}"),
                _ = server.wait_for_first_request() => {}
            }
            let before_response = server.snapshot();
            assert_eq!(before_response.requests.len(), 1);
            assert_eq!(before_response.authorized, vec![true]);
            assert!(before_response.responses.is_empty());
            assert_eq!((before_response.health_calls, before_response.capabilities_calls), (0, 0));
            assert!(before_response.unexpected_data_calls.is_empty());
            let mut reader = BusinessIntentStore::open(&database).unwrap();
            let mut read_local = reader.single_user_local_chain_post_close(&config).unwrap();
            let begun = read_local.inspect_macro(&intent).unwrap();
            assert!(begun.has_unconfirmed_effect());
            assert_eq!(begun.attempts().len(), 1);
            assert_eq!(begun.attempts()[0].request_bytes(), before_response.requests[0]);
            assert!(begun.attempts()[0].result_version().is_none());
            assert!(begun.attempts()[0].response_bytes().is_none());
            assert!(begun.global_news(GlobalNewsProvider::Eastmoney).is_none());
            let saved = (
                begun.plan_bytes().to_vec(),
                begun.attempts()[0].request_bytes().to_vec(),
                begun.attempts()[0].begin_version(),
                read_local.inspect_run(&intent).unwrap().head_version(),
            );
            assert!(saved.2 > parent_head);
            drop(read_local);
            reader.connection.close().unwrap();

            // This pre-opened same-file reader takes a SHARED lock only AFTER
            // the real begin COMMIT is visible. DELETE mode allows writer
            // mutation but denies its EXCLUSIVE lock at the result COMMIT.
            let reader = fault_reader.as_ref().unwrap();
            reader.execute_batch("BEGIN DEFERRED;").unwrap();
            let locked_head: u64 = reader.query_row(
                "SELECT head_version FROM chain_post_close_runs WHERE intent_id=?1",
                [intent.as_str()], |row| row.get(0),
            ).unwrap();
            assert_eq!(locked_head, saved.3);
            assert_eq!(old_fact_rows(reader, &audit_tables), audit_facts);
            clock.now.set(UtcMicros::try_new(started_at + 1_000_000).unwrap());
            server.release_response();
            let error = tokio::time::timeout(Duration::from_secs(5), &mut prepared)
                .await.expect("TEST_CODE bounded actual Macro result COMMIT")
                .expect_err("TEST_CODE result COMMIT must fail while reader holds SHARED lock");
            (saved, error)
        }; // The actual preparation future is dropped before reader cleanup.
        assert!(matches!(
            commit_error.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::ResultUnconfirmed { intent_id }) if intent_id == intent.as_str()
        ));
        assert!(matches!(
            commit_error.downcast_ref::<ChainPostCloseError>(),
            Some(ChainPostCloseError::StorageFailed { operation: "macro result commit" })
        ));
        let failure = commit_error.downcast_ref::<PreparationFailure>().unwrap();
        assert_eq!(failure.stage(), PreparationStage::Macro);
        assert_eq!(failure.completed_stages().last(), Some(&PreparationStage::DragonTiger));
        assert!(!failure.completed_stages().contains(&PreparationStage::Macro));
        assert_eq!(clock.observation_calls.get(), 1);
        let completed_wire = server.snapshot();
        assert_eq!(completed_wire.requests, vec![first_request_bytes.clone()]);
        assert_eq!(completed_wire.authorized, vec![true]);
        assert_eq!(completed_wire.responses.len(), 1);
        let response = QueryResponse::decode(completed_wire.responses[0].as_slice()).unwrap();
        let request = QueryRequest::decode(first_request_bytes.as_slice()).unwrap();
        assert_eq!(response.request_id, request.context.unwrap().request_id);
        assert_eq!(response.operation, Operation::GlobalNews as i32);
        assert_eq!(response.records.len(), 1);
        assert_eq!(response.records[0].schema, "news.global_news");
        assert_eq!(response.records[0].data, NEWS_RECORDS.as_bytes());
        assert_eq!((completed_wire.health_calls, completed_wire.capabilities_calls), (0, 0));
        assert!(completed_wire.unexpected_data_calls.is_empty());
        let stopped_same_io = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks.clone(), None, &mut io,
        ).await.expect_err("TEST_CODE failed COMMIT IO must stay stopped");
        assert!(matches!(
            stopped_same_io.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::ResultUnconfirmed { intent_id }) if intent_id == intent.as_str()
        ));
        assert_eq!(server.snapshot(), completed_wire);
        drop(io);

        // Release the snapshot, then close it before obtaining fresh evidence.
        // No assertion from the old locked snapshot alone proves rollback.
        let reader = fault_reader.take().unwrap();
        reader.execute_batch("ROLLBACK;").unwrap();
        reader.close().unwrap();
        let mut fresh = BusinessIntentStore::open(&database).unwrap();
        assert_eq!(old_fact_rows(&fresh.connection, &audit_tables), audit_facts);
        let mut read_local = fresh.single_user_local_chain_post_close(&config).unwrap();
        let unknown = read_local.inspect_macro(&intent).unwrap();
        assert!(unknown.has_unconfirmed_effect());
        assert!(!unknown.is_complete());
        assert_eq!(unknown.plan_bytes(), first_plan_bytes);
        assert_eq!(unknown.plan().started_at().get(), started_at);
        assert_eq!(unknown.plan().deadline_at().get(), started_at + 15_000_000);
        assert_eq!(unknown.parent_final_bytes(), parent_final);
        assert_eq!(unknown.attempts().len(), 1);
        assert_eq!(unknown.attempts()[0].attempt_ordinal(), 1);
        assert_eq!(unknown.attempts()[0].begin_version(), begin_version);
        assert_eq!(unknown.attempts()[0].request_bytes(), first_request_bytes);
        assert!(unknown.attempts()[0].result_version().is_none());
        assert!(unknown.attempts()[0].response_bytes().is_none());
        assert!(unknown.attempts()[0].continuation().is_none());
        assert!(unknown.global_news(GlobalNewsProvider::Eastmoney).is_none());
        assert_eq!(unknown.pending_source_identities(), unknown.plan().source_identities());
        assert_eq!(unknown.pending_source_identities().len(), 5);
        assert_eq!(unknown.pending_research_queries().len(), 6);
        assert_eq!(read_local.inspect_run(&intent).unwrap().head_version(), begin_head);
        assert_eq!(read_local.inspect_run(&intent).unwrap().context().canonical_bytes(), parent_context);
        drop(read_local);
        let transaction = fresh.connection.unchecked_transaction().unwrap();
        read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
        transaction.commit().unwrap();
        fresh.connection.close().unwrap();
        assert_eq!(local.inspect_run(&intent).unwrap().head_version(), begin_head);
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 3);
        drop(local);
        assert_eq!(old_fact_rows(fixture.connection(), &earlier_tables), earlier_facts);
        assert_eq!(old_fact_rows(fixture.connection(), &audit_tables), audit_facts);
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count);

        drop(queries);
        drop(parent_source);
        drop(macro_source);
        fixture.reopen();
        server.change_response_for_reopen();
        let changed_macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await, server.endpoint().to_owned(),
        );
        let changed_parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let changed_queries = changed_parent_source.connected_board_queries().await.unwrap();
        let changed_clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at + 3_000_000).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-15T15:45:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(&intent, macro_lease(
            "TEST_CODE_MACRO_COMMIT_REOPENED", started_at + 3_000_000,
            started_at + 10_000_000, begin_head,
        )).unwrap();
        let resumed_head = local.inspect_run(&intent).unwrap().head_version();
        assert!(resumed_head > begin_head);
        let mut io = local.macro_preparation_io_v11(
            lease, &changed_queries, &changed_clock, FixedClusterConfiguration::resolve(Some("2")),
            &changed_parent_source, &changed_macro_source, &search_service,
        ).unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks, None, &mut io,
        ).await.expect_err("TEST_CODE result COMMIT failure must reopen Unknown");
        assert!(matches!(
            stopped.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::IncompleteOnReopen { intent_id }) if intent_id == intent.as_str()
        ));
        assert!(matches!(
            stopped.downcast_ref::<ChainPostCloseError>(),
            Some(ChainPostCloseError::IncompleteEffect { intent_id }) if intent_id == intent.as_str()
        ));
        let failure = stopped.downcast_ref::<PreparationFailure>().unwrap();
        assert_eq!(failure.stage(), PreparationStage::Macro);
        assert_eq!(failure.completed_stages().last(), Some(&PreparationStage::DragonTiger));
        assert!(!failure.completed_stages().contains(&PreparationStage::Macro));
        drop(io);
        let reopened = local.inspect_macro(&intent).unwrap();
        assert!(reopened.has_unconfirmed_effect());
        assert!(!reopened.is_complete());
        assert_eq!(reopened.plan_bytes(), first_plan_bytes);
        assert_eq!(reopened.plan().started_at().get(), started_at);
        assert_eq!(reopened.plan().deadline_at().get(), started_at + 15_000_000);
        assert_eq!(reopened.parent_final_bytes(), parent_final);
        assert_eq!(reopened.pending_source_identities(), unknown.pending_source_identities());
        assert_eq!(reopened.pending_research_queries(), unknown.pending_research_queries());
        assert_eq!(reopened.attempts().len(), 1);
        assert_eq!(reopened.attempts()[0].begin_version(), begin_version);
        assert_eq!(reopened.attempts()[0].request_bytes(), first_request_bytes);
        assert!(reopened.attempts()[0].result_version().is_none());
        assert!(reopened.attempts()[0].response_bytes().is_none());
        assert!(reopened.attempts()[0].continuation().is_none());
        assert!(reopened.global_news(GlobalNewsProvider::Eastmoney).is_none());
        assert_eq!(changed_clock.observation_calls.get(), 0);
        assert_eq!(local.inspect_run(&intent).unwrap().head_version(), resumed_head);
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 4);
        assert_eq!(local.inspect_run(&intent).unwrap().context().canonical_bytes(), parent_context);
        drop(local);
        assert_eq!(server.snapshot(), completed_wire);
        assert_eq!(parent_server.as_ref().unwrap().snapshot(), old_network);
        assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), old_memberships);
        assert_eq!(old_fact_rows(fixture.connection(), &earlier_tables), earlier_facts);
        assert_eq!(old_fact_rows(fixture.connection(), &audit_tables), audit_facts);
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count);
        let transaction = fixture.connection().unchecked_transaction().unwrap();
        read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
        transaction.commit().unwrap();
        drop(changed_queries);
        drop(changed_parent_source);
        drop(changed_macro_source);
    })).catch_unwind().await;

    // The body and all in-flight futures are gone. Release/close any fault
    // reader retained by a panic before either server join or temp-root drop.
    let fault_cleanup = fault_reader.take().map(|reader| {
        let rollback = if reader.is_autocommit() {
            Ok(())
        } else {
            reader.execute_batch("ROLLBACK;")
        };
        let close = reader.close().map_err(|(connection, error)| {
            drop(connection);
            error
        });
        (rollback, close)
    });
    // The body future (and every borrowed IO/client/reader) has been dropped.
    // Join both owners even if one cleanup fails; the database root is still live.
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
    let database_cleanup = fixture.store.take().map(|store| {
        store.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        })
    });
    drop(fixture);
    // All owned tasks have terminated before any body failure is reported.
    if let Some((rollback, close)) = fault_cleanup {
        rollback.expect("TEST_CODE fault reader rollback after joins");
        close.expect("TEST_CODE fault reader close after joins");
    }
    macro_cleanup
        .expect("TEST_CODE Macro cleanup panic")
        .expect("TEST_CODE Macro cleanup");
    parent_cleanup.expect("TEST_CODE parent cleanup panic after join");
    if let Some(result) = database_cleanup {
        result.expect("TEST_CODE business connection close");
    }
    match body {
        Ok(result) => result.expect("TEST_CODE result COMMIT Macro integration body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

use crate::pipeline::chain_analysis::preparation::ChainPreparationIo;
use std::collections::{HashMap, HashSet};

// Only the begin-COMMIT test uses this borrowed decorator. Every business
// method reaches the real adapter; the fault starts at its Macro budget entry,
// after earlier recovery stages have completed their own transactions.
struct MacroBeginCommitFaultIo<'a, I: ChainPreparationIo> {
    inner: &'a mut I,
    reader: &'a Connection,
    expected_intent: &'a str,
    expected_head: u64,
    injections: usize,
}

#[async_trait::async_trait(?Send)]
impl<I: ChainPreparationIo> ChainPreparationIo for MacroBeginCommitFaultIo<'_, I> {
    fn validate_fixed_input(
        &mut self,
        business_date: NaiveDate,
        limit_ups: &[TopStock],
        macro_news: &Option<String>,
    ) -> anyhow::Result<()> {
        self.inner
            .validate_fixed_input(business_date, limit_ups, macro_news)
    }

    async fn concepts(&mut self, codes: &[String]) -> anyhow::Result<HashMap<String, Vec<String>>> {
        self.inner.concepts(codes).await
    }

    async fn position_concepts(
        &mut self,
        codes: &[String],
    ) -> anyhow::Result<HashMap<String, Vec<String>>> {
        self.inner.position_concepts(codes).await
    }

    fn before_cluster_configuration(
        &mut self,
        concepts: &HashMap<String, Vec<String>>,
    ) -> anyhow::Result<()> {
        self.inner.before_cluster_configuration(concepts)
    }

    fn min_cluster_size(&mut self) -> usize {
        self.inner.min_cluster_size()
    }

    fn cluster_material(
        &mut self,
        stocks: &[TopStock],
        concepts: &HashMap<String, Vec<String>>,
    ) -> anyhow::Result<(
        usize,
        Vec<crate::pipeline::chain_analysis::ChainCluster>,
        Vec<TopStock>,
    )> {
        self.inner.cluster_material(stocks, concepts)
    }

    async fn persist_clusters(
        &mut self,
        date: NaiveDate,
        rows: &[(String, Vec<String>, i32)],
    ) -> anyhow::Result<HashMap<String, i64>> {
        self.inner.persist_clusters(date, rows).await
    }

    async fn board_codes(
        &mut self,
    ) -> anyhow::Result<(
        HashMap<String, String>,
        Vec<crate::data_gateway::BatchEvidence>,
    )> {
        self.inner.board_codes().await
    }

    async fn candidates(
        &mut self,
        board: &str,
        excluded: &HashSet<String>,
    ) -> anyhow::Result<GatewayBatch<TopStock>> {
        self.inner.candidates(board, excluded).await
    }

    fn resolve_board_code(
        &mut self,
        cluster_ordinal: usize,
        cluster: &crate::pipeline::chain_analysis::ChainCluster,
        board_map: &HashMap<String, String>,
    ) -> anyhow::Result<String> {
        self.inner
            .resolve_board_code(cluster_ordinal, cluster, board_map)
    }

    async fn positions(
        &mut self,
    ) -> anyhow::Result<Vec<crate::pipeline::chain_analysis::preparation::PositionInput>> {
        self.inner.positions().await
    }

    async fn lhb(
        &mut self,
    ) -> anyhow::Result<(
        HashMap<String, f64>,
        crate::pipeline::chain_analysis::preparation::SourceObservation,
    )> {
        self.inner.lhb().await
    }

    async fn macro_search(&mut self) -> anyhow::Result<String> {
        self.inner.macro_search().await
    }

    async fn macro_search_with_budget(
        &mut self,
    ) -> std::result::Result<anyhow::Result<String>, tokio::time::error::Elapsed> {
        assert_eq!(
            self.injections, 0,
            "TEST_CODE inject Macro read lock only once"
        );
        assert!(self.reader.is_autocommit());
        self.reader.execute_batch("BEGIN DEFERRED;").unwrap();
        let locked_head: u64 = self
            .reader
            .query_row(
                "SELECT head_version FROM chain_post_close_runs WHERE intent_id=?1",
                [self.expected_intent],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(locked_head, self.expected_head);
        self.injections += 1;
        self.inner.macro_search_with_budget().await
    }

    fn before_models_search_and_report(&mut self) -> anyhow::Result<()> {
        self.inner.before_models_search_and_report()
    }

    fn model_available(&mut self) -> bool {
        self.inner.model_available()
    }

    async fn model(
        &self,
        prompt: &str,
        system: &str,
        mode: crate::analyzer::AgentMode,
    ) -> anyhow::Result<String> {
        self.inner.model(prompt, system, mode).await
    }

    fn search_available(&mut self) -> bool {
        self.inner.search_available()
    }

    async fn search_topic(
        &mut self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<crate::search_service::SearchResult>> {
        self.inner.search_topic(query, limit).await
    }

    fn local_now(&mut self) -> DateTime<chrono::FixedOffset> {
        self.inner.local_now()
    }
}

#[tokio::test]
async fn single_user_local_macro_begin_commit_failure_keeps_plan_and_can_resume_without_replay() {
    // These owners outlive the caught body, including failures while constructing
    // clients, seeding the real parent, reopening SQLite or asserting results.
    let mut fixture = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut macro_server = None;
    let mut fault_reader: Option<Connection> = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), async {
        let (parent_endpoint, server) = tokio::time::timeout(
            Duration::from_secs(5),
            spawn_macro_parent_listener(DRAGON_TIGER_RECORDS.as_bytes()),
        )
        .await
        .expect("TEST_CODE parent listener deadline");
        parent_server = Some(server);
        macro_server = Some(MacroLoopbackServer::bind().await);
        let server = macro_server.as_ref().unwrap();
        let parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let queries = parent_source.connected_board_queries().await.unwrap();
        let (stocks, config, intent, v9_head) = populate_completed_v9_parent(
            &mut fixture,
            &queries,
            "TEST_CODE_RUN_MACRO_BEGIN_COMMIT",
        )
        .await;
        fixture
            .chain_post_close()
            .migrate_schema_v9_to_v10()
            .unwrap();
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                lease_request(
                    "TEST_CODE_MACRO_PARENT_OWNER",
                    68_300_000_000,
                    90_000_000_000,
                    Some(v9_head),
                ),
            )
            .unwrap();
        let parent_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-22T10:30:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-22T10:30:00+08:00").unwrap(),
            request_calls: Cell::new(0),
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .dragon_tiger_preparation_io_v10(
                lease,
                &queries,
                &parent_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &parent_source,
            )
            .unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE real parent reaches Macro guard");
        assert_partial_macro_stop(&stopped);
        drop(io);
        let parent = local.inspect_dragon_tiger(&intent).unwrap();
        assert_complete_batch(parent.batch().unwrap());
        let parent_final = parent.final_bytes().unwrap().to_vec();
        let parent_receipt = parent.audit_receipt().unwrap().clone();
        let parent_run = local.inspect_run(&intent).unwrap();
        let parent_context = parent_run.context().canonical_bytes();
        let parent_head = parent_run.head_version();
        drop(local);
        let earlier_tables = table_names(fixture.connection());
        let earlier_facts = old_fact_rows(fixture.connection(), &earlier_tables);
        let audit_count = fixture.count("data_acquisition_audit");
        let audit_tables = vec![
            "data_acquisition_audit".to_owned(),
            "data_acquisition_audit_chain".to_owned(),
        ];
        let audit_facts = old_fact_rows(fixture.connection(), &audit_tables);
        let old_network = parent_server.as_ref().unwrap().snapshot();
        let old_memberships = parent_server.as_ref().unwrap().membership_snapshot();
        assert_eq!(old_network.dragon_tiger_requests.len(), 1);
        let database = fixture.database();

        // The same owned store and real first-source adapter used by the positive test.
        assert_eq!(
            fixture
                .chain_post_close()
                .migrate_schema_v10_to_v11()
                .unwrap()
                .schema_version(),
            11
        );
        let journal: String = fixture
            .connection()
            .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_ascii_lowercase(), "delete");
        let writer_busy_ms: i64 = fixture
            .connection()
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(writer_busy_ms, 250); // Preserve the store's existing bounded wait.
        let macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await,
            server.endpoint().to_owned(),
        );
        let started_at = micros("2026-09-14T15:30:00+08:00");
        let clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-14T15:30:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        // Explicit instance registration in original order. Local availability
        // derives from this supplied bridge, never bridge_for/global credentials.
        let registered = [
            GeneralWebResearchProvider::SerpApi,
            GeneralWebResearchProvider::Bocha,
            GeneralWebResearchProvider::Tavily,
        ];
        let search_service = macro_search_service(&registered);
        let web = search_service.macro_web_snapshot(&macro_source).unwrap();
        let connected = macro_source.connected_macro_queries().unwrap();
        let authorized = connected
            .session(MacroQueryIdentity::GlobalNews {
                provider: GlobalNewsProvider::Eastmoney,
                limit: 20,
            })
            .unwrap()
            .authorize_next()
            .unwrap();
        let original_request = authorized.request_bytes();
        let original_request_id = authorized.request_id().to_owned();
        let original_policy = authorized.retry_policy();
        assert_eq!(original_policy, (4, 1000, 60_000, 200));
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                macro_lease(
                    "TEST_CODE_MACRO_PLAN_OWNER",
                    started_at,
                    started_at + 2_000_000,
                    parent_head,
                ),
            )
            .unwrap();
        // Confirm the actual typed plan with a real authorized native request,
        // without calling execute or fabricating any persisted SQL fact.
        let planned_lease = local
            .plan_macro(
                lease,
                clock.now(),
                clock.macro_request_observation(),
                connected.endpoint(),
                &authorized,
                &web,
                clock.now(),
            )
            .unwrap();
        let plan_head = planned_lease.head_version();
        let planned = local.inspect_macro(&intent).unwrap();
        let original_plan = planned.plan_bytes().to_vec();
        assert!(!planned.is_complete());
        assert!(!planned.has_unconfirmed_effect());
        assert!(planned.attempts().is_empty());
        assert!(planned.global_news(GlobalNewsProvider::Eastmoney).is_none());
        assert_eq!(planned.plan().started_at().get(), started_at);
        assert_eq!(planned.plan().deadline_at().get(), started_at + 15_000_000);
        assert_eq!(planned.parent_final_bytes(), parent_final);
        assert_eq!(
            planned.pending_source_identities(),
            planned.plan().source_identities()
        );
        assert_eq!(planned.pending_source_identities().len(), 5);
        assert_eq!(planned.pending_research_queries().len(), 6);
        assert_eq!(
            local.inspect_run(&intent).unwrap().head_version(),
            plan_head
        );
        assert_eq!(clock.observation_calls.get(), 1);
        drop(planned_lease);
        drop(local);
        assert_eq!(
            old_fact_rows(fixture.connection(), &audit_tables),
            audit_facts
        );
        assert_eq!(
            old_fact_rows(fixture.connection(), &earlier_tables),
            earlier_facts
        );
        let no_rpc = server.snapshot();
        assert!(no_rpc.requests.is_empty());
        assert!(no_rpc.responses.is_empty());
        assert!(no_rpc.authorized.is_empty());
        assert_eq!((no_rpc.health_calls, no_rpc.capabilities_calls), (0, 0));
        assert!(no_rpc.unexpected_data_calls.is_empty());
        drop(authorized);
        drop(connected);
        drop(queries);
        drop(parent_source);
        drop(macro_source);
        fixture.reopen(); // First reopen: the plan committed but no begin ever ran.

        let fault_macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await,
            server.endpoint().to_owned(),
        );
        let fault_parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let fault_queries = fault_parent_source.connected_board_queries().await.unwrap();
        let fault_clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at + 3_000_000).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-15T15:45:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        fault_reader = Some(
            Connection::open_with_flags(
                &database,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .unwrap(),
        );
        fault_reader
            .as_ref()
            .unwrap()
            .busy_timeout(Duration::from_millis(250))
            .unwrap();
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                macro_lease(
                    "TEST_CODE_MACRO_BEGIN_FAULT_OWNER",
                    started_at + 3_000_000,
                    started_at + 4_000_000,
                    plan_head,
                ),
            )
            .unwrap();
        let fault_head = local.inspect_run(&intent).unwrap().head_version();
        assert!(fault_head > plan_head);
        let mut io = local
            .macro_preparation_io_v11(
                lease,
                &fault_queries,
                &fault_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &fault_parent_source,
                &fault_macro_source,
                &search_service,
            )
            .unwrap();
        // Earlier stages still execute their real recovery transactions. The
        // decorator takes the SHARED lock only at the real Macro budget entry.
        let reader = fault_reader.as_ref().unwrap();
        let mut fault_io = MacroBeginCommitFaultIo {
            inner: &mut io,
            reader,
            expected_intent: intent.as_str(),
            expected_head: fault_head,
            injections: 0,
        };
        let error = {
            let prepared = prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                stocks.clone(),
                None,
                &mut fault_io,
            );
            tokio::pin!(prepared);
            tokio::time::timeout(Duration::from_secs(5), &mut prepared)
                .await
                .expect("TEST_CODE bounded actual Macro begin COMMIT")
                .expect_err("TEST_CODE SHARED reader must block begin COMMIT")
        };
        assert_eq!(fault_io.injections, 1);
        assert_eq!(old_fact_rows(reader, &audit_tables), audit_facts);
        assert!(matches!(
            error.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::AuthorityRejected { intent_id }) if intent_id == intent.as_str()
        ));
        assert!(
            matches!(
                error.downcast_ref::<ChainPostCloseError>(),
                Some(ChainPostCloseError::StorageFailed {
                    operation: "macro begin commit"
                })
            ),
            "TEST_CODE begin COMMIT expected; cause={:?}; stage={:?}",
            error.downcast_ref::<ChainPostCloseError>(),
            error
                .downcast_ref::<PreparationFailure>()
                .map(|failure| failure.stage()),
        );
        let failure = error.downcast_ref::<PreparationFailure>().unwrap();
        assert_eq!(failure.stage(), PreparationStage::Macro);
        assert_eq!(
            failure.completed_stages().last(),
            Some(&PreparationStage::DragonTiger)
        );
        assert!(!failure
            .completed_stages()
            .contains(&PreparationStage::Macro));
        assert_eq!(fault_clock.observation_calls.get(), 0);
        assert_eq!(server.snapshot(), no_rpc);
        let same_io = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut fault_io,
        )
        .await
        .expect_err("TEST_CODE consumed begin-failure lease cannot retry in same IO");
        assert!(matches!(
            same_io.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::AuthorityRejected { intent_id }) if intent_id == intent.as_str()
        ));
        assert_eq!(server.snapshot(), no_rpc);
        assert_eq!(fault_io.injections, 1);
        drop(fault_io);
        drop(io);
        let reader = fault_reader.take().unwrap();
        reader.execute_batch("ROLLBACK;").unwrap();
        reader.close().unwrap();

        // Fresh snapshot, not the locked reader's old view, proves rollback.
        let mut fresh = BusinessIntentStore::open(&database).unwrap();
        assert_eq!(old_fact_rows(&fresh.connection, &audit_tables), audit_facts);
        assert_eq!(
            old_fact_rows(&fresh.connection, &earlier_tables),
            earlier_facts
        );
        let mut read_local = fresh.single_user_local_chain_post_close(&config).unwrap();
        let rolled_back = read_local.inspect_macro(&intent).unwrap();
        assert!(!rolled_back.has_unconfirmed_effect());
        assert!(!rolled_back.is_complete());
        assert!(rolled_back.attempts().is_empty());
        assert!(rolled_back
            .global_news(GlobalNewsProvider::Eastmoney)
            .is_none());
        assert_eq!(rolled_back.plan_bytes(), original_plan);
        assert_eq!(rolled_back.plan().started_at().get(), started_at);
        assert_eq!(
            rolled_back.plan().deadline_at().get(),
            started_at + 15_000_000
        );
        assert_eq!(rolled_back.parent_final_bytes(), parent_final);
        assert_eq!(
            rolled_back.pending_source_identities(),
            planned.pending_source_identities()
        );
        assert_eq!(
            rolled_back.pending_research_queries(),
            planned.pending_research_queries()
        );
        assert_eq!(
            read_local.inspect_run(&intent).unwrap().head_version(),
            fault_head
        );
        assert_eq!(
            read_local
                .inspect_run(&intent)
                .unwrap()
                .context()
                .canonical_bytes(),
            parent_context
        );
        drop(read_local);
        let transaction = fresh.connection.unchecked_transaction().unwrap();
        read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
        transaction.commit().unwrap();
        fresh.connection.close().unwrap();
        assert_eq!(
            local.inspect_run(&intent).unwrap().head_version(),
            fault_head
        );
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 4);
        drop(local);
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count);
        assert_eq!(
            old_fact_rows(fixture.connection(), &audit_tables),
            audit_facts
        );
        drop(fault_queries);
        drop(fault_parent_source);
        drop(fault_macro_source);
        fixture.reopen(); // Second reopen: fresh owner only after the short lease expires.

        let resumed_macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await,
            server.endpoint().to_owned(),
        );
        let resumed_parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let resumed_queries = resumed_parent_source
            .connected_board_queries()
            .await
            .unwrap();
        let resumed_clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at + 5_000_000).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-16T16:15:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                macro_lease(
                    "TEST_CODE_MACRO_BEGIN_RECOVERED_OWNER",
                    started_at + 5_000_000,
                    started_at + 14_000_000,
                    fault_head,
                ),
            )
            .unwrap();
        let resumed_head = local.inspect_run(&intent).unwrap().head_version();
        assert!(resumed_head > fault_head);
        let mut io = local
            .macro_preparation_io_v11(
                lease,
                &resumed_queries,
                &resumed_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &resumed_parent_source,
                &resumed_macro_source,
                &search_service,
            )
            .unwrap();
        // No prior request was sent. This permits the first actual response,
        // with the original fixture payload, only after its new begin commits.
        server.release_response();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE only first source is migrated");
        assert_partial_macro_stop(&stopped);
        drop(io);
        let recovered = local.inspect_macro(&intent).unwrap();
        assert!(!recovered.has_unconfirmed_effect());
        assert!(!recovered.is_complete());
        assert_eq!(recovered.plan_bytes(), original_plan);
        assert_eq!(recovered.plan().started_at().get(), started_at);
        assert_eq!(
            recovered.plan().deadline_at().get(),
            started_at + 15_000_000
        );
        assert_eq!(recovered.parent_final_bytes(), parent_final);
        assert_eq!(recovered.attempts().len(), 1);
        let attempt = &recovered.attempts()[0];
        assert_eq!(attempt.attempt_ordinal(), 1);
        assert_eq!(attempt.request_bytes(), original_request);
        assert!(attempt.begin_version() > resumed_head);
        assert!(attempt.result_version().unwrap() > attempt.begin_version());
        assert_eq!(attempt.continuation(), Some(MacroContinuation::Terminal));
        assert_eq!(
            recovered.pending_source_identities(),
            &recovered.plan().source_identities()[1..]
        );
        assert_eq!(
            recovered.pending_research_queries(),
            planned.pending_research_queries()
        );
        let source = recovered
            .global_news(GlobalNewsProvider::Eastmoney)
            .unwrap();
        assert!(source.is_complete());
        assert_eq!(source.retry_policy(), original_policy);
        assert_eq!(source.profile(), "LocalBridgeV1");
        assert_eq!(source.acquisition_authority(), None);
        assert_native_news(source.batch().unwrap());
        assert!(!source.final_bytes().unwrap().is_empty());
        let receipt = source.audit_receipt().unwrap().clone();
        assert_eq!(resumed_clock.observation_calls.get(), 0);
        assert_eq!(
            local.inspect_run(&intent).unwrap().head_version(),
            resumed_head + 3
        );
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 5);
        assert_eq!(
            local
                .inspect_run(&intent)
                .unwrap()
                .context()
                .canonical_bytes(),
            parent_context
        );
        drop(local);
        let wire = server.snapshot();
        assert_eq!(wire.requests, vec![original_request.clone()]);
        assert_eq!(wire.authorized, vec![true]);
        assert_eq!(wire.responses.len(), 1);
        assert_eq!(attempt.response_bytes().unwrap(), wire.responses[0]);
        let request = QueryRequest::decode(wire.requests[0].as_slice()).unwrap();
        assert_eq!(request.context.unwrap().request_id, original_request_id);
        let response = QueryResponse::decode(wire.responses[0].as_slice()).unwrap();
        assert_eq!(response.request_id, original_request_id);
        assert_eq!(response.operation, Operation::GlobalNews as i32);
        assert_eq!(response.records.len(), 1);
        assert_eq!(response.records[0].schema, "news.global_news");
        assert_eq!(response.records[0].data, NEWS_RECORDS.as_bytes());
        assert_eq!((wire.health_calls, wire.capabilities_calls), (0, 0));
        assert!(wire.unexpected_data_calls.is_empty());
        assert_eq!(parent_server.as_ref().unwrap().snapshot(), old_network);
        assert_eq!(
            parent_server.as_ref().unwrap().membership_snapshot(),
            old_memberships
        );
        assert_eq!(
            old_fact_rows(fixture.connection(), &earlier_tables),
            earlier_facts
        );
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count + 1);
        let after_audits = old_fact_rows(fixture.connection(), &audit_tables);
        for (table, original_rows) in &audit_facts {
            let rows = &after_audits[table];
            assert_eq!(rows.len(), original_rows.len() + 1);
            assert_eq!(&rows[..original_rows.len()], original_rows.as_slice());
        }
        let transaction = fixture.connection().unchecked_transaction().unwrap();
        let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
        let audit = verified.record();
        assert_eq!(audit.capability, "GlobalNews-Eastmoney");
        assert_eq!(audit.provider, "Eastmoney");
        assert_eq!(audit.source, "eastmoney-web");
        assert_eq!(
            audit.request_hash,
            "fb86badeeebfca14c04928026fe295416a2a47c6534250291c066f3a74b67b9c"
        );
        assert_eq!(audit.outcome, "available");
        assert_eq!(
            (
                audit.request_count,
                audit.accepted_count,
                audit.rejected_count
            ),
            (1, 2, 0)
        );
        read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
        transaction.commit().unwrap();
        drop(resumed_queries);
        drop(resumed_parent_source);
        drop(resumed_macro_source);
    }))
    .catch_unwind()
    .await;

    // The body and all in-flight futures are gone. Release/close any fault
    // reader retained by a panic before either server join or temp-root drop.
    let fault_cleanup = fault_reader.take().map(|reader| {
        let rollback = if reader.is_autocommit() {
            Ok(())
        } else {
            reader.execute_batch("ROLLBACK;")
        };
        let close = reader.close().map_err(|(connection, error)| {
            drop(connection);
            error
        });
        (rollback, close)
    });
    // The body future (and every borrowed IO/client/reader) has been dropped.
    // Join both owners even if one cleanup fails; the database root is still live.
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
    let database_cleanup = fixture.store.take().map(|store| {
        store.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        })
    });
    drop(fixture);
    // All owned tasks have terminated before any body failure is reported.
    if let Some((rollback, close)) = fault_cleanup {
        rollback.expect("TEST_CODE fault reader rollback after joins");
        close.expect("TEST_CODE fault reader close after joins");
    }
    macro_cleanup
        .expect("TEST_CODE Macro cleanup panic")
        .expect("TEST_CODE Macro cleanup");
    parent_cleanup.expect("TEST_CODE parent cleanup panic after join");
    if let Some(result) = database_cleanup {
        result.expect("TEST_CODE business connection close");
    }
    match body {
        Ok(result) => result.expect("TEST_CODE begin COMMIT Macro integration body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn single_user_local_macro_verified_empty_reopens_without_rpc_or_audit() {
    let mut fixture = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut macro_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), async {
        let (parent_endpoint, server) = tokio::time::timeout(
            Duration::from_secs(5),
            spawn_macro_parent_listener(DRAGON_TIGER_RECORDS.as_bytes()),
        )
        .await
        .expect("TEST_CODE parent listener deadline");
        parent_server = Some(server);
        macro_server = Some(MacroLoopbackServer::bind().await);
        let server = macro_server.as_ref().unwrap();
        server.respond_with_verified_empty();
        let parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let queries = parent_source.connected_board_queries().await.unwrap();
        let (stocks, config, intent, v9_head) = populate_completed_v9_parent(
            &mut fixture,
            &queries,
            "TEST_CODE_RUN_MACRO_VERIFIED_EMPTY",
        )
        .await;
        fixture
            .chain_post_close()
            .migrate_schema_v9_to_v10()
            .unwrap();
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                lease_request(
                    "TEST_CODE_MACRO_EMPTY_PARENT_OWNER",
                    68_300_000_000,
                    90_000_000_000,
                    Some(v9_head),
                ),
            )
            .unwrap();
        let parent_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-22T10:30:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-22T10:30:00+08:00")
                .unwrap(),
            request_calls: Cell::new(0),
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .dragon_tiger_preparation_io_v10(
                lease,
                &queries,
                &parent_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &parent_source,
            )
            .unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE real parent reaches Macro guard");
        assert_partial_macro_stop(&stopped);
        drop(io);
        let parent = local.inspect_dragon_tiger(&intent).unwrap();
        assert_complete_batch(parent.batch().unwrap());
        let parent_final = parent.final_bytes().unwrap().to_vec();
        let parent_receipt = parent.audit_receipt().unwrap().clone();
        let parent_run = local.inspect_run(&intent).unwrap();
        let parent_context = parent_run.context().canonical_bytes();
        let parent_head = parent_run.head_version();
        drop(local);
        let earlier_tables = table_names(fixture.connection());
        let earlier_facts = old_fact_rows(fixture.connection(), &earlier_tables);
        let audit_count = fixture.count("data_acquisition_audit");
        let audit_tables = vec![
            "data_acquisition_audit".to_owned(),
            "data_acquisition_audit_chain".to_owned(),
        ];
        let audit_facts = old_fact_rows(fixture.connection(), &audit_tables);
        let old_network = parent_server.as_ref().unwrap().snapshot();
        let old_memberships = parent_server.as_ref().unwrap().membership_snapshot();
        assert_eq!(old_network.dragon_tiger_requests.len(), 1);
        let database = fixture.database();

        assert_eq!(
            fixture
                .chain_post_close()
                .migrate_schema_v10_to_v11()
                .unwrap()
                .schema_version(),
            11
        );
        let macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await,
            server.endpoint().to_owned(),
        );
        let started_at = micros("2026-09-14T15:30:00+08:00");
        let clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-14T15:30:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let registered = [
            GeneralWebResearchProvider::SerpApi,
            GeneralWebResearchProvider::Bocha,
            GeneralWebResearchProvider::Tavily,
        ];
        let search_service = macro_search_service(&registered);
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                macro_lease(
                    "TEST_CODE_MACRO_EMPTY_OWNER",
                    started_at,
                    started_at + 2_000_000,
                    parent_head,
                ),
            )
            .unwrap();
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
        let stopped = {
            let prepared = prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                stocks.clone(),
                None,
                &mut io,
            );
            tokio::pin!(prepared);
            tokio::select! {
                result = &mut prepared => panic!("TEST_CODE prepare returned before gated empty source: {result:?}"),
                _ = server.wait_for_first_request() => {}
            }
            let mut reader = BusinessIntentStore::open(&database).unwrap();
            let mut read_local = reader
                .single_user_local_chain_post_close(&config)
                .unwrap();
            let begun = read_local.inspect_macro(&intent).unwrap();
            assert!(begun.has_unconfirmed_effect());
            assert!(!begun.is_complete());
            assert_eq!(begun.attempts().len(), 1);
            assert_eq!(begun.attempts()[0].attempt_ordinal(), 1);
            assert!(begun.attempts()[0].response_bytes().is_none());
            assert_eq!(
                begun.attempts()[0].request_bytes(),
                server.snapshot().requests[0]
            );
            assert!(begun.attempts()[0].begin_version() > parent_head);
            drop(read_local);
            reader.connection.close().unwrap();
            clock
                .now
                .set(UtcMicros::try_new(started_at + 1_000_000).unwrap());
            server.release_response();
            prepared
                .await
                .expect_err("TEST_CODE remaining Macro sources stay pending")
        };
        assert_partial_macro_stop(&stopped);
        drop(io);
        let recovered = local.inspect_macro(&intent).unwrap();
        assert!(!recovered.is_complete());
        assert!(!recovered.has_unconfirmed_effect());
        assert_eq!(recovered.parent_final_bytes(), parent_final);
        let plan = recovered.plan();
        assert_eq!(plan.started_at().get(), started_at);
        assert_eq!(plan.deadline_at().get(), started_at + 15_000_000);
        assert_eq!(plan.observed_local(), "2026-09-14T15:30:00+08:00");
        assert_eq!(
            plan.source_identities(),
            &[
                MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Eastmoney,
                    limit: 20,
                },
                MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Cailianpress,
                    limit: 20,
                },
                MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Jin10,
                    limit: 20,
                },
                MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::ThePaper,
                    limit: 20,
                },
                MacroQueryIdentity::EconomicCalendar,
            ]
        );
        assert_eq!(plan.economic_intent(), (20, None));
        assert_eq!(
            plan.research_queries(),
            &[
                "2026年09月14日A股 大盘 股市 最新动态",
                "2026年09月14日国际财经 地缘政治 最新消息",
                "2026年09月14日美股 美联储 大宗商品 今日",
                "2026年09月14日中国 央行 财政 产业政策 重要新闻",
                "2026年09月14日高盛 摩根 大摩 美银 JPMorgan 中国A股 市场观点 研报",
                "2026年09月14日证券时报 第一财经 21世纪经济报道 重要财经",
            ]
        );
        assert_eq!(plan.research_limit(), 3);
        assert_eq!(plan.research_providers(), registered);
        assert!(plan
            .research_decisions()
            .iter()
            .all(|decision| decision.supports_general_web_search() && decision.is_available()));
        assert_eq!(plan.research_decisions().len(), 3);
        assert_eq!(plan.gateway_pace_ms(), 200);
        assert_eq!(plan.query_pace_ms(), 300);
        assert_eq!(
            recovered.pending_source_identities(),
            &plan.source_identities()[1..]
        );
        assert_eq!(recovered.pending_source_identities().len(), 4);
        assert_eq!(recovered.pending_research_queries(), plan.research_queries());
        assert_eq!(recovered.pending_research_queries().len(), 6);
        let source = recovered
            .global_news(GlobalNewsProvider::Eastmoney)
            .unwrap();
        assert!(source.is_complete());
        assert_eq!(source.profile(), "LocalBridgeV1");
        assert_eq!(source.acquisition_authority(), None);
        assert_eq!(source.retry_policy(), (4, 1000, 60_000, 200));
        let batch = source.batch().unwrap();
        assert!(batch.is_verified_empty());
        assert!(batch.records().is_empty());
        let evidence = batch.evidence();
        assert_eq!(evidence.provider, ProviderId::Eastmoney);
        assert_eq!(evidence.source, "eastmoney-web");
        assert_eq!(
            evidence.source_at.as_deref(),
            Some("2026-09-14T15:30:00.123456789+08:00")
        );
        assert_eq!(
            evidence.observed_at,
            "2026-09-14T15:30:00.987654321+08:00"
        );
        assert_eq!(evidence.batch_id, "TEST_CODE_MACRO_BATCH_FIRST");
        assert!(source.error().is_none());
        let receipt = source.audit_receipt().unwrap().clone();
        assert_eq!(receipt.previous_outcome, None);
        assert_eq!(receipt.current_outcome, "verified_empty");
        let first_plan_bytes = recovered.plan_bytes().to_vec();
        let first_native_bytes = source.final_bytes().unwrap().to_vec();
        assert!(!first_native_bytes.is_empty());
        let native: serde_json::Value = serde_json::from_slice(&first_native_bytes).unwrap();
        assert_eq!(
            native,
            serde_json::json!({
                "version": 1,
                "kind": "VerifiedEmpty",
                "evidence": {
                    "provider": "Eastmoney",
                    "source": "eastmoney-web",
                    "source_at": "2026-09-14T15:30:00.123456789+08:00",
                    "observed_at": "2026-09-14T15:30:00.987654321+08:00",
                    "batch_id": "TEST_CODE_MACRO_BATCH_FIRST"
                },
                "records": []
            })
        );
        let attempts = recovered.attempts();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].attempt_ordinal(), 1);
        let begin_version = attempts[0].begin_version();
        let result_version = attempts[0].result_version().unwrap();
        assert!(result_version > begin_version);
        assert_eq!(
            attempts[0].continuation(),
            Some(MacroContinuation::Terminal)
        );
        let first_request_bytes = attempts[0].request_bytes().to_vec();
        let first_response_bytes = attempts[0].response_bytes().unwrap().to_vec();
        let first_head = local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(first_head, result_version + 1);
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 3);
        assert_eq!(clock.observation_calls.get(), 1);
        drop(local);

        let wire = server.snapshot();
        assert_eq!(wire.requests, vec![first_request_bytes.clone()]);
        assert_eq!(wire.authorized, vec![true]);
        assert_eq!(wire.responses, vec![first_response_bytes.clone()]);
        assert_eq!((wire.health_calls, wire.capabilities_calls), (0, 0));
        assert!(wire.unexpected_data_calls.is_empty());
        let request = QueryRequest::decode(first_request_bytes.as_slice()).unwrap();
        let context = request.context.unwrap();
        assert_eq!(context.protocol_version, 1);
        assert!(!context.request_id.is_empty());
        assert!(request.preferred_provider.is_empty());
        assert!(!request.allow_unadmitted);
        let payload = request.payload.unwrap();
        assert_eq!(payload.schema, "news.global_news");
        assert_eq!(payload.schema_version, 1);
        assert_eq!(payload.content_type, "application/json; charset=utf-8");
        assert_eq!(payload.data, br#"{"limit":20,"provider":"Eastmoney"}"#);
        let response = QueryResponse::decode(first_response_bytes.as_slice()).unwrap();
        assert_eq!(response.request_id, context.request_id);
        assert_eq!(response.operation, Operation::GlobalNews as i32);
        assert_eq!(
            response.admission,
            crate::grpc_client::pb::magic::market::v1::AdmissionState::Admitted as i32
        );
        assert_eq!(response.selected_provider, "Eastmoney");
        assert_eq!(response.batch_id, "TEST_CODE_MACRO_BATCH_FIRST");
        assert!(response.complete);
        assert_eq!(
            response.observed_at,
            "2026-09-14T15:30:00.987654321+08:00"
        );
        assert_eq!(
            response.source_at,
            "2026-09-14T15:30:00.123456789+08:00"
        );
        assert_eq!(response.source, "eastmoney-web");
        assert!(response.diagnostic_blocker.is_empty());
        assert_eq!(response.records.len(), 1);
        assert_eq!(response.records[0].schema, "news.global_news");
        assert_eq!(response.records[0].schema_version, 1);
        assert_eq!(
            response.records[0].content_type,
            "application/json; charset=utf-8"
        );
        assert_eq!(response.records[0].data, b"[]");

        let transaction = fixture.connection().unchecked_transaction().unwrap();
        let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
        assert_eq!(verified.receipt(), &receipt);
        let audit = verified.record();
        assert_eq!(audit.capability, "GlobalNews-Eastmoney");
        assert_eq!(audit.provider, "Eastmoney");
        assert_eq!(audit.source, "eastmoney-web");
        assert_eq!(
            audit.request_hash,
            "fb86badeeebfca14c04928026fe295416a2a47c6534250291c066f3a74b67b9c"
        );
        assert_eq!(
            audit.source_at,
            Some("2026-09-14T15:30:00.123456789+08:00")
        );
        assert_eq!(
            audit.observed_at,
            "2026-09-14T15:30:00.987654321+08:00"
        );
        assert_eq!(audit.batch_id, Some("TEST_CODE_MACRO_BATCH_FIRST"));
        assert_eq!(audit.outcome, "verified_empty");
        assert_eq!(
            (
                audit.request_count,
                audit.accepted_count,
                audit.rejected_count
            ),
            (1, 0, 0)
        );
        assert_eq!(audit.reason_code, "verified_empty");
        assert!(!audit.retryable);
        read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
        transaction.commit().unwrap();
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count + 1);
        let completed_audit_facts = old_fact_rows(fixture.connection(), &audit_tables);
        for (table, original_rows) in &audit_facts {
            let rows = &completed_audit_facts[table];
            assert_eq!(rows.len(), original_rows.len() + 1);
            assert_eq!(&rows[..original_rows.len()], original_rows.as_slice());
        }

        drop(queries);
        drop(parent_source);
        drop(macro_source);
        fixture.reopen();
        server.change_response_for_reopen();
        let changed_macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await,
            server.endpoint().to_owned(),
        );
        let changed_parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let changed_queries = changed_parent_source.connected_board_queries().await.unwrap();
        let changed_clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at + 3_000_000).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-15T15:45:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                macro_lease(
                    "TEST_CODE_MACRO_EMPTY_REOPENED_OWNER",
                    started_at + 3_000_000,
                    started_at + 10_000_000,
                    first_head,
                ),
            )
            .unwrap();
        let reopened_head = local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(reopened_head, first_head + 1);
        let mut io = local
            .macro_preparation_io_v11(
                lease,
                &changed_queries,
                &changed_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &changed_parent_source,
                &changed_macro_source,
                &search_service,
            )
            .unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE recovered empty source leaves the Macro plan pending");
        assert_partial_macro_stop(&stopped);
        drop(io);
        let reopened = local.inspect_macro(&intent).unwrap();
        assert_eq!(reopened.plan_bytes(), first_plan_bytes);
        assert_eq!(reopened.plan().started_at().get(), started_at);
        assert_eq!(reopened.plan().deadline_at().get(), started_at + 15_000_000);
        assert_eq!(reopened.parent_final_bytes(), parent_final);
        assert!(!reopened.is_complete());
        assert!(!reopened.has_unconfirmed_effect());
        assert_eq!(reopened.pending_source_identities().len(), 4);
        assert_eq!(reopened.pending_research_queries().len(), 6);
        assert_eq!(reopened.attempts().len(), 1);
        let reopened_attempt = &reopened.attempts()[0];
        assert_eq!(reopened_attempt.attempt_ordinal(), 1);
        assert_eq!(reopened_attempt.begin_version(), begin_version);
        assert_eq!(reopened_attempt.request_bytes(), first_request_bytes);
        assert_eq!(
            reopened_attempt.response_bytes().unwrap(),
            first_response_bytes
        );
        assert_eq!(reopened_attempt.result_version(), Some(result_version));
        assert_eq!(
            reopened_attempt.continuation(),
            Some(MacroContinuation::Terminal)
        );
        let reopened_request = QueryRequest::decode(reopened_attempt.request_bytes()).unwrap();
        assert_eq!(reopened_request.context.unwrap().request_id, context.request_id);
        let source = reopened
            .global_news(GlobalNewsProvider::Eastmoney)
            .unwrap();
        assert_eq!(source.profile(), "LocalBridgeV1");
        assert_eq!(source.acquisition_authority(), None);
        assert_eq!(source.retry_policy(), (4, 1000, 60_000, 200));
        let batch = source.batch().unwrap();
        assert!(batch.is_verified_empty());
        assert!(batch.records().is_empty());
        assert_eq!(batch.evidence().provider, ProviderId::Eastmoney);
        assert_eq!(batch.evidence().source, "eastmoney-web");
        assert_eq!(
            batch.evidence().source_at.as_deref(),
            Some("2026-09-14T15:30:00.123456789+08:00")
        );
        assert_eq!(
            batch.evidence().observed_at,
            "2026-09-14T15:30:00.987654321+08:00"
        );
        assert_eq!(
            batch.evidence().batch_id,
            "TEST_CODE_MACRO_BATCH_FIRST"
        );
        assert_eq!(source.final_bytes().unwrap(), first_native_bytes);
        assert_eq!(source.audit_receipt(), Some(&receipt));
        assert_eq!(changed_clock.observation_calls.get(), 0);
        assert_eq!(local.inspect_run(&intent).unwrap().head_version(), reopened_head);
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 4);
        assert_eq!(
            local
                .inspect_run(&intent)
                .unwrap()
                .context()
                .canonical_bytes(),
            parent_context
        );
        drop(local);
        assert_eq!(server.snapshot(), wire);
        assert_eq!(parent_server.as_ref().unwrap().snapshot(), old_network);
        assert_eq!(
            parent_server.as_ref().unwrap().membership_snapshot(),
            old_memberships
        );
        assert_eq!(
            old_fact_rows(fixture.connection(), &earlier_tables),
            earlier_facts
        );
        assert_eq!(
            old_fact_rows(fixture.connection(), &audit_tables),
            completed_audit_facts
        );
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count + 1);
        let transaction = fixture.connection().unchecked_transaction().unwrap();
        let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
        assert_eq!(verified.receipt(), &receipt);
        read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
        transaction.commit().unwrap();
        drop(changed_queries);
        drop(changed_parent_source);
        drop(changed_macro_source);
    }))
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
    let database_cleanup = fixture.store.take().map(|store| {
        store.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        })
    });
    drop(fixture);
    macro_cleanup
        .expect("TEST_CODE Macro cleanup panic")
        .expect("TEST_CODE Macro cleanup");
    parent_cleanup.expect("TEST_CODE parent cleanup panic after join");
    if let Some(result) = database_cleanup {
        result.expect("TEST_CODE business connection close");
    }
    match body {
        Ok(result) => result.expect("TEST_CODE verified-empty Macro integration body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn single_user_local_macro_terminal_status_reopens_without_rpc_or_audit() {
    let mut fixture = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut macro_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), async {
        let (parent_endpoint, server) = tokio::time::timeout(
            Duration::from_secs(5),
            spawn_macro_parent_listener(DRAGON_TIGER_RECORDS.as_bytes()),
        )
        .await
        .expect("TEST_CODE parent listener deadline");
        parent_server = Some(server);
        macro_server = Some(MacroLoopbackServer::bind().await);
        let server = macro_server.as_ref().unwrap();
        server.respond_with_terminal_status();
        let parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let queries = parent_source.connected_board_queries().await.unwrap();
        let (stocks, config, intent, v9_head) = populate_completed_v9_parent(
            &mut fixture,
            &queries,
            "TEST_CODE_RUN_MACRO_TERMINAL_STATUS",
        )
        .await;
        fixture
            .chain_post_close()
            .migrate_schema_v9_to_v10()
            .unwrap();
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                lease_request(
                    "TEST_CODE_MACRO_STATUS_PARENT_OWNER",
                    68_300_000_000,
                    90_000_000_000,
                    Some(v9_head),
                ),
            )
            .unwrap();
        let parent_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-22T10:30:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-22T10:30:00+08:00")
                .unwrap(),
            request_calls: Cell::new(0),
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .dragon_tiger_preparation_io_v10(
                lease,
                &queries,
                &parent_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &parent_source,
            )
            .unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE real parent reaches Macro guard");
        assert_partial_macro_stop(&stopped);
        drop(io);
        let parent = local.inspect_dragon_tiger(&intent).unwrap();
        assert_complete_batch(parent.batch().unwrap());
        let parent_final = parent.final_bytes().unwrap().to_vec();
        let parent_receipt = parent.audit_receipt().unwrap().clone();
        let parent_run = local.inspect_run(&intent).unwrap();
        let parent_context = parent_run.context().canonical_bytes();
        let parent_head = parent_run.head_version();
        drop(local);
        let earlier_tables = table_names(fixture.connection());
        let earlier_facts = old_fact_rows(fixture.connection(), &earlier_tables);
        let audit_count = fixture.count("data_acquisition_audit");
        let audit_tables = vec![
            "data_acquisition_audit".to_owned(),
            "data_acquisition_audit_chain".to_owned(),
        ];
        let audit_facts = old_fact_rows(fixture.connection(), &audit_tables);
        let old_network = parent_server.as_ref().unwrap().snapshot();
        let old_memberships = parent_server.as_ref().unwrap().membership_snapshot();
        assert_eq!(old_network.dragon_tiger_requests.len(), 1);
        let database = fixture.database();

        assert_eq!(
            fixture
                .chain_post_close()
                .migrate_schema_v10_to_v11()
                .unwrap()
                .schema_version(),
            11
        );
        let macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await,
            server.endpoint().to_owned(),
        );
        let started_at = micros("2026-09-14T15:30:00+08:00");
        let clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-14T15:30:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let registered = [
            GeneralWebResearchProvider::SerpApi,
            GeneralWebResearchProvider::Bocha,
            GeneralWebResearchProvider::Tavily,
        ];
        let search_service = macro_search_service(&registered);
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                macro_lease(
                    "TEST_CODE_MACRO_STATUS_OWNER",
                    started_at,
                    started_at + 2_000_000,
                    parent_head,
                ),
            )
            .unwrap();
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
        let stopped = {
            let prepared = prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                stocks.clone(),
                None,
                &mut io,
            );
            tokio::pin!(prepared);
            tokio::select! {
                result = &mut prepared => panic!("TEST_CODE prepare returned before gated status: {result:?}"),
                _ = server.wait_for_first_request() => {}
            }
            let mut reader = BusinessIntentStore::open(&database).unwrap();
            let mut read_local = reader
                .single_user_local_chain_post_close(&config)
                .unwrap();
            let begun = read_local.inspect_macro(&intent).unwrap();
            assert!(begun.has_unconfirmed_effect());
            assert!(!begun.is_complete());
            assert_eq!(begun.attempts().len(), 1);
            assert_eq!(begun.attempts()[0].attempt_ordinal(), 1);
            assert!(begun.attempts()[0].result_material().is_none());
            assert!(begun.attempts()[0].response_bytes().is_none());
            assert_eq!(begun.attempts()[0].continuation(), None);
            assert_eq!(
                begun.attempts()[0].request_bytes(),
                server.snapshot().requests[0]
            );
            assert!(begun.attempts()[0].begin_version() > parent_head);
            drop(read_local);
            reader.connection.close().unwrap();
            clock
                .now
                .set(UtcMicros::try_new(started_at + 1_000_000).unwrap());
            server.release_response();
            prepared
                .await
                .expect_err("TEST_CODE terminal first source leaves Macro pending")
        };
        assert_partial_macro_stop(&stopped);
        drop(io);
        let recovered = local.inspect_macro(&intent).unwrap();
        assert!(!recovered.is_complete());
        assert!(!recovered.has_unconfirmed_effect());
        assert_eq!(recovered.parent_final_bytes(), parent_final);
        let plan = recovered.plan();
        assert_eq!(plan.started_at().get(), started_at);
        assert_eq!(plan.deadline_at().get(), started_at + 15_000_000);
        assert_eq!(plan.observed_local(), "2026-09-14T15:30:00+08:00");
        assert_eq!(
            plan.source_identities(),
            &[
                MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Eastmoney,
                    limit: 20,
                },
                MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Cailianpress,
                    limit: 20,
                },
                MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::Jin10,
                    limit: 20,
                },
                MacroQueryIdentity::GlobalNews {
                    provider: GlobalNewsProvider::ThePaper,
                    limit: 20,
                },
                MacroQueryIdentity::EconomicCalendar,
            ]
        );
        assert_eq!(plan.economic_intent(), (20, None));
        assert_eq!(
            plan.research_queries(),
            &[
                "2026年09月14日A股 大盘 股市 最新动态",
                "2026年09月14日国际财经 地缘政治 最新消息",
                "2026年09月14日美股 美联储 大宗商品 今日",
                "2026年09月14日中国 央行 财政 产业政策 重要新闻",
                "2026年09月14日高盛 摩根 大摩 美银 JPMorgan 中国A股 市场观点 研报",
                "2026年09月14日证券时报 第一财经 21世纪经济报道 重要财经",
            ]
        );
        assert_eq!(plan.research_limit(), 3);
        assert_eq!(plan.research_providers(), registered);
        assert!(plan
            .research_decisions()
            .iter()
            .all(|decision| decision.supports_general_web_search() && decision.is_available()));
        assert_eq!(plan.research_decisions().len(), 3);
        assert_eq!(plan.gateway_pace_ms(), 200);
        assert_eq!(plan.query_pace_ms(), 300);
        assert_eq!(
            recovered.pending_source_identities(),
            &plan.source_identities()[1..]
        );
        assert_eq!(recovered.pending_source_identities().len(), 4);
        assert_eq!(recovered.pending_research_queries(), plan.research_queries());
        assert_eq!(recovered.pending_research_queries().len(), 6);
        let source = recovered
            .global_news(GlobalNewsProvider::Eastmoney)
            .unwrap();
        assert!(source.is_complete());
        assert_eq!(source.profile(), "LocalBridgeV1");
        assert_eq!(source.acquisition_authority(), None);
        assert_eq!(source.retry_policy(), (4, 1000, 60_000, 200));
        assert!(source.batch().is_none());
        let error = source.error().unwrap();
        assert_eq!(error.capability(), "GrpcBridge");
        assert_eq!(error.provider(), Some(ProviderId::Eastmoney));
        assert_eq!(error.audit_outcome(), "unavailable");
        assert_eq!(error.reason_code(), "no_verified_batch");
        assert!(!error.retryable());
        assert_eq!(
            error.message(),
            "gRPC GlobalNews 查询失败: 服务不可用 (指数退避, 重新检查 health/capabilities)"
        );
        let first_error = error.clone();
        let receipt = source.audit_receipt().unwrap().clone();
        assert_eq!(receipt.previous_outcome, None);
        assert_eq!(receipt.current_outcome, "unavailable");
        let first_plan_bytes = recovered.plan_bytes().to_vec();
        let first_native_bytes = source.final_bytes().unwrap().to_vec();
        assert!(!first_native_bytes.is_empty());
        let native: serde_json::Value = serde_json::from_slice(&first_native_bytes).unwrap();
        assert_eq!(
            native,
            serde_json::json!({
                "version": 1,
                "kind": "Error",
                "error": {
                    "capability": "GrpcBridge",
                    "provider": "Eastmoney",
                    "audit_outcome": "unavailable",
                    "reason_code": "no_verified_batch",
                    "retryable": false,
                    "message": "gRPC GlobalNews 查询失败: 服务不可用 (指数退避, 重新检查 health/capabilities)"
                }
            })
        );
        let attempts = recovered.attempts();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].attempt_ordinal(), 1);
        assert!(attempts[0].response_bytes().is_none());
        let result_material = attempts[0].result_material().unwrap();
        assert_eq!(
            result_material.diagnostic,
            Some("[redacted-unclassified-status]")
        );
        assert_eq!(
            result_material.retry_decision,
            crate::grpc_client::retry::RetryDecision::NoRetry
        );
        assert_eq!(result_material.continuation, MacroContinuation::Terminal);
        let (first_status_code, first_status_details, first_status_trailer) =
            match result_material.wire {
                crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecoveredWire::Status {
                    code,
                    details,
                    trailer:
                        crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecoveredTrailer::Bytes(
                            trailer,
                        ),
                } => (code, details.to_vec(), trailer.to_vec()),
                _ => panic!("TEST_CODE expected recovered terminal Status material"),
            };
        assert_eq!(first_status_code, tonic::Code::Unavailable as i32);
        assert_eq!(attempts[0].continuation(), Some(MacroContinuation::Terminal));
        let begin_version = attempts[0].begin_version();
        let result_version = attempts[0].result_version().unwrap();
        assert!(result_version > begin_version);
        let first_request_bytes = attempts[0].request_bytes().to_vec();
        let first_head = local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(first_head, result_version + 1);
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 3);
        assert_eq!(clock.observation_calls.get(), 1);
        drop(local);

        let wire = server.snapshot();
        assert_eq!(wire.requests, vec![first_request_bytes.clone()]);
        assert_eq!(wire.authorized, vec![true]);
        assert!(wire.responses.is_empty());
        assert_eq!(wire.statuses.len(), 1);
        assert_eq!(wire.statuses[0].code, tonic::Code::Unavailable as i32);
        assert_eq!(wire.statuses[0].details, first_status_details);
        assert_eq!(wire.statuses[0].trailer, first_status_trailer);
        assert_eq!((wire.health_calls, wire.capabilities_calls), (0, 0));
        assert!(wire.unexpected_data_calls.is_empty());
        let request = QueryRequest::decode(first_request_bytes.as_slice()).unwrap();
        let context = request.context.unwrap();
        assert_eq!(context.protocol_version, 1);
        assert!(!context.request_id.is_empty());
        assert!(request.preferred_provider.is_empty());
        assert!(!request.allow_unadmitted);
        let payload = request.payload.unwrap();
        assert_eq!(payload.schema, "news.global_news");
        assert_eq!(payload.schema_version, 1);
        assert_eq!(payload.content_type, "application/json; charset=utf-8");
        assert_eq!(payload.data, br#"{"limit":20,"provider":"Eastmoney"}"#);
        let standard = crate::grpc_client::pb::magic::market::v1::ErrorDetail::decode(
            first_status_details.as_slice(),
        )
        .unwrap();
        let trailer = crate::grpc_client::pb::magic::market::v1::ErrorDetail::decode(
            first_status_trailer.as_slice(),
        )
        .unwrap();
        assert_eq!(standard, trailer);
        assert_eq!(standard.request_id, context.request_id);
        assert_eq!(standard.operation, Operation::GlobalNews as i32);
        assert_eq!(standard.provider, "Eastmoney");
        assert_eq!(standard.reason_code, "no_verified_batch");
        assert!(!standard.retryable);

        let transaction = fixture.connection().unchecked_transaction().unwrap();
        let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
        assert_eq!(verified.receipt(), &receipt);
        let audit = verified.record();
        assert_eq!(audit.capability, "GlobalNews-Eastmoney");
        assert_eq!(audit.provider, "Eastmoney");
        assert_eq!(audit.source, "review-data-gateway");
        assert_eq!(
            audit.request_hash,
            "fb86badeeebfca14c04928026fe295416a2a47c6534250291c066f3a74b67b9c"
        );
        assert_eq!(audit.source_at, None);
        assert_eq!(audit.observed_at, "2026-09-14T07:30:01+00:00");
        assert_eq!(audit.batch_id, None);
        assert_eq!(audit.outcome, "unavailable");
        assert_eq!(
            (
                audit.request_count,
                audit.accepted_count,
                audit.rejected_count
            ),
            (1, 0, 1)
        );
        assert_eq!(audit.reason_code, "no_verified_batch");
        assert!(!audit.retryable);
        read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
        transaction.commit().unwrap();
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count + 1);
        let completed_audit_facts = old_fact_rows(fixture.connection(), &audit_tables);
        for (table, original_rows) in &audit_facts {
            let rows = &completed_audit_facts[table];
            assert_eq!(rows.len(), original_rows.len() + 1);
            assert_eq!(&rows[..original_rows.len()], original_rows.as_slice());
        }

        drop(queries);
        drop(parent_source);
        drop(macro_source);
        fixture.reopen();
        server.change_response_for_reopen();
        let changed_macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await,
            server.endpoint().to_owned(),
        );
        let changed_parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let changed_queries = changed_parent_source.connected_board_queries().await.unwrap();
        let changed_clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at + 3_000_000).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-15T15:45:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                macro_lease(
                    "TEST_CODE_MACRO_STATUS_REOPENED_OWNER",
                    started_at + 3_000_000,
                    started_at + 10_000_000,
                    first_head,
                ),
            )
            .unwrap();
        let reopened_head = local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(reopened_head, first_head + 1);
        let mut io = local
            .macro_preparation_io_v11(
                lease,
                &changed_queries,
                &changed_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &changed_parent_source,
                &changed_macro_source,
                &search_service,
            )
            .unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE recovered terminal source leaves Macro pending");
        assert_partial_macro_stop(&stopped);
        drop(io);
        let reopened = local.inspect_macro(&intent).unwrap();
        assert_eq!(reopened.plan_bytes(), first_plan_bytes);
        assert_eq!(reopened.plan().started_at().get(), started_at);
        assert_eq!(reopened.plan().deadline_at().get(), started_at + 15_000_000);
        assert_eq!(reopened.parent_final_bytes(), parent_final);
        assert!(!reopened.is_complete());
        assert!(!reopened.has_unconfirmed_effect());
        assert_eq!(reopened.pending_source_identities().len(), 4);
        assert_eq!(reopened.pending_research_queries().len(), 6);
        assert_eq!(reopened.attempts().len(), 1);
        let reopened_attempt = &reopened.attempts()[0];
        assert_eq!(reopened_attempt.attempt_ordinal(), 1);
        assert_eq!(reopened_attempt.begin_version(), begin_version);
        assert_eq!(reopened_attempt.request_bytes(), first_request_bytes);
        assert!(reopened_attempt.response_bytes().is_none());
        assert_eq!(reopened_attempt.result_version(), Some(result_version));
        assert_eq!(
            reopened_attempt.continuation(),
            Some(MacroContinuation::Terminal)
        );
        let reopened_result = reopened_attempt.result_material().unwrap();
        assert_eq!(
            reopened_result.diagnostic,
            Some("[redacted-unclassified-status]")
        );
        assert_eq!(
            reopened_result.retry_decision,
            crate::grpc_client::retry::RetryDecision::NoRetry
        );
        assert_eq!(reopened_result.continuation, MacroContinuation::Terminal);
        match reopened_result.wire {
            crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecoveredWire::Status {
                code,
                details,
                trailer:
                    crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecoveredTrailer::Bytes(
                        trailer,
                    ),
            } => {
                assert_eq!(code, first_status_code);
                assert_eq!(details, first_status_details);
                assert_eq!(trailer, first_status_trailer);
            }
            _ => panic!("TEST_CODE expected reopened terminal Status material"),
        }
        let reopened_request = QueryRequest::decode(reopened_attempt.request_bytes()).unwrap();
        assert_eq!(reopened_request.context.unwrap().request_id, context.request_id);
        let source = reopened
            .global_news(GlobalNewsProvider::Eastmoney)
            .unwrap();
        assert!(source.batch().is_none());
        let reopened_error = source.error().unwrap();
        assert_eq!(reopened_error.capability(), first_error.capability());
        assert_eq!(reopened_error.provider(), first_error.provider());
        assert_eq!(reopened_error.audit_outcome(), first_error.audit_outcome());
        assert_eq!(reopened_error.reason_code(), first_error.reason_code());
        assert_eq!(reopened_error.retryable(), first_error.retryable());
        assert_eq!(reopened_error.message(), first_error.message());
        assert_eq!(source.profile(), "LocalBridgeV1");
        assert_eq!(source.acquisition_authority(), None);
        assert_eq!(source.retry_policy(), (4, 1000, 60_000, 200));
        assert_eq!(source.final_bytes().unwrap(), first_native_bytes);
        assert_eq!(source.audit_receipt(), Some(&receipt));
        assert_eq!(changed_clock.observation_calls.get(), 0);
        assert_eq!(local.inspect_run(&intent).unwrap().head_version(), reopened_head);
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 4);
        assert_eq!(
            local
                .inspect_run(&intent)
                .unwrap()
                .context()
                .canonical_bytes(),
            parent_context
        );
        drop(local);
        assert_eq!(server.snapshot(), wire);
        assert_eq!(parent_server.as_ref().unwrap().snapshot(), old_network);
        assert_eq!(
            parent_server.as_ref().unwrap().membership_snapshot(),
            old_memberships
        );
        assert_eq!(
            old_fact_rows(fixture.connection(), &earlier_tables),
            earlier_facts
        );
        assert_eq!(
            old_fact_rows(fixture.connection(), &audit_tables),
            completed_audit_facts
        );
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count + 1);
        let transaction = fixture.connection().unchecked_transaction().unwrap();
        let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
        assert_eq!(verified.receipt(), &receipt);
        read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
        transaction.commit().unwrap();
        drop(changed_queries);
        drop(changed_parent_source);
        drop(changed_macro_source);
    }))
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
    let database_cleanup = fixture.store.take().map(|store| {
        store.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        })
    });
    drop(fixture);
    macro_cleanup
        .expect("TEST_CODE Macro cleanup panic")
        .expect("TEST_CODE Macro cleanup");
    parent_cleanup.expect("TEST_CODE parent cleanup panic after join");
    if let Some(result) = database_cleanup {
        result.expect("TEST_CODE business connection close");
    }
    match body {
        Ok(result) => result.expect("TEST_CODE terminal-status Macro integration body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn single_user_local_macro_confirmed_retry_reopens_after_remaining_backoff() {
    let mut fixture = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut macro_server = None;
    let mut retry_reader: Option<BusinessIntentStore> = None;
    let mut time_paused = false;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), async {
        let (parent_endpoint, server) = tokio::time::timeout(
            Duration::from_secs(5),
            spawn_macro_parent_listener(DRAGON_TIGER_RECORDS.as_bytes()),
        )
        .await
        .expect("TEST_CODE parent listener deadline");
        parent_server = Some(server);
        macro_server = Some(MacroLoopbackServer::bind().await);
        let server = macro_server.as_ref().unwrap();
        server.respond_with_retry_then_success();
        let parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let queries = parent_source.connected_board_queries().await.unwrap();
        let (stocks, config, intent, v9_head) = populate_completed_v9_parent(
            &mut fixture,
            &queries,
            "TEST_CODE_RUN_MACRO_CONFIRMED_RETRY",
        )
        .await;
        fixture
            .chain_post_close()
            .migrate_schema_v9_to_v10()
            .unwrap();
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                lease_request(
                    "TEST_CODE_MACRO_RETRY_PARENT_OWNER",
                    68_300_000_000,
                    90_000_000_000,
                    Some(v9_head),
                ),
            )
            .unwrap();
        let parent_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-22T10:30:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-22T10:30:00+08:00")
                .unwrap(),
            request_calls: Cell::new(0),
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .dragon_tiger_preparation_io_v10(
                lease,
                &queries,
                &parent_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &parent_source,
            )
            .unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE real parent reaches Macro guard");
        assert_partial_macro_stop(&stopped);
        drop(io);
        let parent = local.inspect_dragon_tiger(&intent).unwrap();
        assert_complete_batch(parent.batch().unwrap());
        let parent_final = parent.final_bytes().unwrap().to_vec();
        let parent_receipt = parent.audit_receipt().unwrap().clone();
        let parent_run = local.inspect_run(&intent).unwrap();
        let parent_context = parent_run.context().canonical_bytes();
        let parent_head = parent_run.head_version();
        drop(local);
        let earlier_tables = table_names(fixture.connection());
        let earlier_facts = old_fact_rows(fixture.connection(), &earlier_tables);
        let audit_count = fixture.count("data_acquisition_audit");
        let audit_tables = vec![
            "data_acquisition_audit".to_owned(),
            "data_acquisition_audit_chain".to_owned(),
        ];
        let audit_facts = old_fact_rows(fixture.connection(), &audit_tables);
        let old_network = parent_server.as_ref().unwrap().snapshot();
        let old_memberships = parent_server.as_ref().unwrap().membership_snapshot();
        assert_eq!(old_network.dragon_tiger_requests.len(), 1);
        let database = fixture.database();

        assert_eq!(
            fixture
                .chain_post_close()
                .migrate_schema_v10_to_v11()
                .unwrap()
                .schema_version(),
            11
        );
        let macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await,
            server.endpoint().to_owned(),
        );
        let started_at = micros("2026-09-14T15:30:00+08:00");
        let clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-14T15:30:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let registered = [
            GeneralWebResearchProvider::SerpApi,
            GeneralWebResearchProvider::Bocha,
            GeneralWebResearchProvider::Tavily,
        ];
        let search_service = macro_search_service(&registered);
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                macro_lease(
                    "TEST_CODE_MACRO_RETRY_OWNER",
                    started_at,
                    started_at + 750_000,
                    parent_head,
                ),
            )
            .unwrap();
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

        tokio::time::pause();
        time_paused = true;
        let mut prepared = Box::pin(prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        ));
        let first_request_deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(5);
        while server.snapshot().requests.is_empty() {
            assert!(
                std::time::Instant::now() < first_request_deadline,
                "TEST_CODE timed out waiting for first Retry request"
            );
            tokio::select! {
                biased;
                result = &mut prepared => panic!("TEST_CODE prepare returned before first Retry request: {result:?}"),
                _ = tokio::task::yield_now() => {}
            }
        }
        retry_reader = Some(BusinessIntentStore::open(&database).unwrap());
        let mut read_local = retry_reader
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let begun = read_local.inspect_macro(&intent).unwrap();
        assert!(begun.has_unconfirmed_effect());
        assert_eq!(begun.attempts().len(), 1);
        assert_eq!(begun.attempts()[0].attempt_ordinal(), 1);
        assert!(begun.attempts()[0].result_material().is_none());
        assert_eq!(begun.attempts()[0].request_bytes(), server.snapshot().requests[0]);
        assert!(begun.attempts()[0].begin_version() > parent_head);
        clock
            .now
            .set(UtcMicros::try_new(started_at + 500_000).unwrap());
        server.release_response();

        let confirmation_deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(5);
        let confirmed = loop {
            assert!(
                std::time::Instant::now() < confirmation_deadline,
                "TEST_CODE timed out waiting for confirmed Macro Retry"
            );
            tokio::select! {
                biased;
                result = &mut prepared => panic!("TEST_CODE prepare returned before Retry cancellation: {result:?}"),
                _ = tokio::task::yield_now() => {}
            }
            let recovery = read_local
                .inspect_macro(&intent)
                .expect("TEST_CODE inspect confirmed Macro Retry");
            if recovery.attempts().len() == 1
                && recovery.attempts()[0].result_version().is_some()
            {
                break recovery;
            }
        };
        assert!(!confirmed.has_unconfirmed_effect());
        assert!(!confirmed.is_complete());
        assert!(confirmed
            .global_news(GlobalNewsProvider::Eastmoney)
            .is_none());
        assert_eq!(confirmed.pending_source_identities().len(), 5);
        assert_eq!(confirmed.pending_research_queries().len(), 6);
        let first_attempt = &confirmed.attempts()[0];
        assert_eq!(first_attempt.attempt_ordinal(), 1);
        assert!(first_attempt.response_bytes().is_none());
        assert_eq!(
            first_attempt.continuation(),
            Some(MacroContinuation::Retry { backoff_ms: 1000 })
        );
        assert_eq!(
            first_attempt.retry_not_before,
            Some(started_at + 1_500_000)
        );
        let first_result_version = first_attempt.result_version().unwrap();
        let first_begin_version = first_attempt.begin_version();
        assert!(first_result_version > first_begin_version);
        let first_material = first_attempt.result_material().unwrap();
        assert_eq!(
            first_material.diagnostic,
            Some("[redacted-unclassified-status]")
        );
        assert_eq!(
            first_material.retry_decision,
            crate::grpc_client::retry::RetryDecision::RetryBackoff
        );
        assert_eq!(
            first_material.continuation,
            MacroContinuation::Retry { backoff_ms: 1000 }
        );
        let (first_status_code, first_status_details, first_status_trailer) =
            match first_material.wire {
                crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecoveredWire::Status {
                    code,
                    details,
                    trailer:
                        crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecoveredTrailer::Bytes(
                            trailer,
                        ),
                } => (code, details.to_vec(), trailer.to_vec()),
                _ => panic!("TEST_CODE confirmed Retry must retain Status material"),
            };
        assert_eq!(first_status_code, tonic::Code::Unavailable as i32);
        let first_plan_bytes = confirmed.plan_bytes().to_vec();
        assert_eq!(confirmed.plan().started_at().get(), started_at);
        assert_eq!(confirmed.plan().deadline_at().get(), started_at + 15_000_000);
        assert_eq!(confirmed.plan().observed_local(), "2026-09-14T15:30:00+08:00");
        assert_eq!(confirmed.parent_final_bytes(), parent_final);
        let first_request_bytes = first_attempt.request_bytes().to_vec();
        let first_wire = server.snapshot();
        assert_eq!(first_wire.requests, vec![first_request_bytes.clone()]);
        assert_eq!(first_wire.authorized, vec![true]);
        assert!(first_wire.responses.is_empty());
        assert_eq!(first_wire.statuses.len(), 1);
        assert_eq!(first_wire.statuses[0].code, first_status_code);
        assert_eq!(first_wire.statuses[0].details, first_status_details);
        assert_eq!(first_wire.statuses[0].trailer, first_status_trailer);
        assert_eq!((first_wire.health_calls, first_wire.capabilities_calls), (0, 0));
        assert!(first_wire.unexpected_data_calls.is_empty());
        let request = QueryRequest::decode(first_request_bytes.as_slice()).unwrap();
        let original_request_id = request.context.as_ref().unwrap().request_id.clone();
        assert!(!original_request_id.is_empty());
        let standard = crate::grpc_client::pb::magic::market::v1::ErrorDetail::decode(
            first_status_details.as_slice(),
        )
        .unwrap();
        let trailer = crate::grpc_client::pb::magic::market::v1::ErrorDetail::decode(
            first_status_trailer.as_slice(),
        )
        .unwrap();
        assert_eq!(standard, trailer);
        assert_eq!(standard.request_id, original_request_id);
        assert_eq!(standard.operation, Operation::GlobalNews as i32);
        assert_eq!(standard.provider, "Eastmoney");
        assert_eq!(standard.reason_code, "no_verified_batch");
        assert!(standard.retryable);
        assert_eq!(clock.observation_calls.get(), 1);

        drop(prepared);
        drop(io);
        drop(read_local);
        let reader = retry_reader.take().unwrap();
        reader.connection.close().unwrap();
        let recovered = local.inspect_macro(&intent).unwrap();
        assert_eq!(recovered.plan_bytes(), first_plan_bytes);
        assert_eq!(recovered.attempts().len(), 1);
        assert!(!recovered.has_unconfirmed_effect());
        let first_retry_head = local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(first_retry_head, first_result_version);
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 3);
        drop(local);
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count);
        assert_eq!(old_fact_rows(fixture.connection(), &audit_tables), audit_facts);

        tokio::time::resume();
        time_paused = false;
        drop(queries);
        drop(parent_source);
        drop(macro_source);
        fixture.reopen();

        let changed_macro_source = GrpcSource::from_macro_loopback_test_client(
            server
                .connect_with_retry_policy_for_test((1, 7, 9, 0))
                .await,
            server.endpoint().to_owned(),
        );
        let current = changed_macro_source.connected_macro_queries().unwrap();
        let current_probe = current
            .session(MacroQueryIdentity::GlobalNews {
                provider: GlobalNewsProvider::Eastmoney,
                limit: 20,
            })
            .unwrap()
            .authorize_next()
            .unwrap();
        assert_eq!(current_probe.retry_policy(), (1, 7, 9, 0));
        drop(current_probe);
        drop(current);
        assert_eq!(server.snapshot(), first_wire);
        let changed_parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let changed_queries = changed_parent_source.connected_board_queries().await.unwrap();
        let changed_clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at + 1_000_000).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-15T15:45:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                macro_lease(
                    "TEST_CODE_MACRO_RETRY_REOPENED_OWNER",
                    started_at + 1_000_000,
                    started_at + 10_000_000,
                    first_retry_head,
                ),
            )
            .unwrap();
        let reopened_head = local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(reopened_head, first_retry_head + 1);
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 4);
        let mut io = local
            .macro_preparation_io_v11(
                lease,
                &changed_queries,
                &changed_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &changed_parent_source,
                &changed_macro_source,
                &search_service,
            )
            .unwrap();
        retry_reader = Some(BusinessIntentStore::open(&database).unwrap());
        let mut read_local = retry_reader
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        server.release_response();
        tokio::time::pause();
        time_paused = true;
        let mut prepared = Box::pin(prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut io,
        ));
        tokio::select! {
            biased;
            result = &mut prepared => panic!("TEST_CODE recovered Retry returned before remaining backoff: {result:?}"),
            _ = tokio::task::yield_now() => {}
        }
        let before_due = read_local.inspect_macro(&intent).unwrap();
        assert!(!before_due.has_unconfirmed_effect());
        assert_eq!(before_due.attempts().len(), 1);
        assert_eq!(
            before_due.attempts()[0].retry_not_before,
            Some(started_at + 1_500_000)
        );
        assert_eq!(before_due.plan_bytes(), first_plan_bytes);
        assert_eq!(read_local.inspect_run(&intent).unwrap().head_version(), reopened_head);
        assert_eq!(server.snapshot(), first_wire);

        changed_clock
            .now
            .set(UtcMicros::try_new(started_at + 1_499_000).unwrap());
        tokio::time::advance(std::time::Duration::from_millis(499)).await;
        let early_watchdog =
            std::time::Instant::now() + std::time::Duration::from_millis(100);
        while std::time::Instant::now() < early_watchdog {
            tokio::select! {
                biased;
                result = &mut prepared => panic!("TEST_CODE Retry resumed before saved due: {result:?}"),
                _ = tokio::task::yield_now() => {}
            }
        }
        assert_eq!(server.snapshot(), first_wire);
        let still_waiting = read_local.inspect_macro(&intent).unwrap();
        assert_eq!(still_waiting.attempts().len(), 1);
        assert!(!still_waiting.has_unconfirmed_effect());
        assert!(still_waiting
            .global_news(GlobalNewsProvider::Eastmoney)
            .is_none());
        assert_eq!(read_local.inspect_run(&intent).unwrap().head_version(), reopened_head);

        changed_clock
            .now
            .set(UtcMicros::try_new(started_at + 1_500_000).unwrap());
        tokio::time::advance(std::time::Duration::from_millis(1)).await;
        let mut completed = tokio::select! {
            biased;
            result = &mut prepared => Some(result),
            _ = tokio::task::yield_now() => None,
        };
        if completed.is_none() {
            changed_clock
                .now
                .set(UtcMicros::try_new(started_at + 1_501_000).unwrap());
            tokio::time::advance(std::time::Duration::from_millis(1)).await;
        }
        let completion_deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(5);
        while completed.is_none() {
            assert!(
                std::time::Instant::now() < completion_deadline,
                "TEST_CODE timed out completing recovered Macro Retry"
            );
            completed = tokio::select! {
                biased;
                result = &mut prepared => Some(result),
                _ = tokio::task::yield_now() => None,
            };
        }
        let stopped = completed
            .unwrap()
            .expect_err("TEST_CODE second Macro success still leaves plan pending");
        assert_partial_macro_stop(&stopped);
        drop(prepared);
        drop(io);
        drop(read_local);
        let reader = retry_reader.take().unwrap();
        reader.connection.close().unwrap();
        tokio::time::resume();
        time_paused = false;

        let recovered = local.inspect_macro(&intent).unwrap();
        assert!(!recovered.is_complete());
        assert!(!recovered.has_unconfirmed_effect());
        assert_eq!(recovered.plan_bytes(), first_plan_bytes);
        assert_eq!(recovered.plan().started_at().get(), started_at);
        assert_eq!(recovered.plan().deadline_at().get(), started_at + 15_000_000);
        assert_eq!(recovered.parent_final_bytes(), parent_final);
        assert_eq!(recovered.pending_source_identities().len(), 4);
        assert_eq!(recovered.pending_research_queries().len(), 6);
        assert_eq!(recovered.attempts().len(), 2);
        let first = &recovered.attempts()[0];
        assert_eq!(first.attempt_ordinal(), 1);
        assert_eq!(first.begin_version(), first_begin_version);
        assert_eq!(first.result_version(), Some(first_result_version));
        assert_eq!(first.request_bytes(), first_request_bytes);
        assert_eq!(first.retry_not_before, Some(started_at + 1_500_000));
        assert_eq!(
            first.continuation(),
            Some(MacroContinuation::Retry { backoff_ms: 1000 })
        );
        let first_recovered_material = first.result_material().unwrap();
        assert_eq!(
            first_recovered_material.retry_decision,
            crate::grpc_client::retry::RetryDecision::RetryBackoff
        );
        match first_recovered_material.wire {
            crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecoveredWire::Status {
                code,
                details,
                trailer:
                    crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecoveredTrailer::Bytes(
                        trailer,
                    ),
            } => {
                assert_eq!(code, first_status_code);
                assert_eq!(details, first_status_details);
                assert_eq!(trailer, first_status_trailer);
            }
            _ => panic!("TEST_CODE original Retry Status changed after success"),
        }
        let second = &recovered.attempts()[1];
        assert_eq!(second.attempt_ordinal(), 2);
        assert!(second.begin_version() > first_result_version);
        assert!(second.result_version().unwrap() > second.begin_version());
        assert_eq!(second.request_bytes(), first_request_bytes);
        assert_eq!(second.retry_not_before, None);
        assert_eq!(second.continuation(), Some(MacroContinuation::Terminal));
        let second_material = second.result_material().unwrap();
        assert_eq!(second_material.diagnostic, None);
        assert_eq!(
            second_material.retry_decision,
            crate::grpc_client::retry::RetryDecision::NoRetry
        );
        assert_eq!(second_material.continuation, MacroContinuation::Terminal);
        let second_response = match second_material.wire {
            crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecoveredWire::Response(
                response,
            ) => response,
            _ => panic!("TEST_CODE second attempt must recover original Response bytes"),
        };
        assert_eq!(second.response_bytes(), Some(second_response));
        let source = recovered
            .global_news(GlobalNewsProvider::Eastmoney)
            .unwrap();
        assert_eq!(source.retry_policy(), (4, 1000, 60_000, 200));
        assert_eq!(source.profile(), "LocalBridgeV1");
        assert_eq!(source.acquisition_authority(), None);
        assert_native_news(source.batch().unwrap());
        let native = source.final_bytes().unwrap().to_vec();
        assert!(!native.is_empty());
        let receipt = source.audit_receipt().unwrap().clone();
        assert_eq!(receipt.previous_outcome, None);
        assert_eq!(receipt.current_outcome, "available");
        assert_eq!(changed_clock.observation_calls.get(), 0);
        assert_eq!(
            local.inspect_run(&intent).unwrap().head_version(),
            reopened_head + 3
        );
        assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 4);
        assert_eq!(
            local
                .inspect_run(&intent)
                .unwrap()
                .context()
                .canonical_bytes(),
            parent_context
        );
        drop(local);

        let final_wire = server.snapshot();
        assert_eq!(final_wire.requests.len(), 2);
        assert_eq!(final_wire.requests[0], first_request_bytes);
        assert_eq!(final_wire.requests[1], first_request_bytes);
        assert_eq!(final_wire.authorized, vec![true, true]);
        assert_eq!(final_wire.statuses, first_wire.statuses);
        assert_eq!(final_wire.responses.len(), 1);
        assert_eq!(second_response, final_wire.responses[0]);
        assert_eq!((final_wire.health_calls, final_wire.capabilities_calls), (0, 0));
        assert!(final_wire.unexpected_data_calls.is_empty());
        let second_request = QueryRequest::decode(final_wire.requests[1].as_slice()).unwrap();
        assert_eq!(second_request.context.unwrap().request_id, original_request_id);
        let response = QueryResponse::decode(final_wire.responses[0].as_slice()).unwrap();
        assert_eq!(response.request_id, original_request_id);
        assert_eq!(response.operation, Operation::GlobalNews as i32);
        assert_eq!(response.records.len(), 1);
        assert_eq!(response.records[0].schema, "news.global_news");
        assert_eq!(response.records[0].data, NEWS_RECORDS.as_bytes());
        assert_eq!(parent_server.as_ref().unwrap().snapshot(), old_network);
        assert_eq!(
            parent_server.as_ref().unwrap().membership_snapshot(),
            old_memberships
        );
        assert_eq!(
            old_fact_rows(fixture.connection(), &earlier_tables),
            earlier_facts
        );
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count + 1);
        let final_audit_facts = old_fact_rows(fixture.connection(), &audit_tables);
        for (table, original_rows) in &audit_facts {
            let rows = &final_audit_facts[table];
            assert_eq!(rows.len(), original_rows.len() + 1);
            assert_eq!(&rows[..original_rows.len()], original_rows.as_slice());
        }
        let transaction = fixture.connection().unchecked_transaction().unwrap();
        let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
        assert_eq!(verified.receipt(), &receipt);
        let audit = verified.record();
        assert_eq!(audit.capability, "GlobalNews-Eastmoney");
        assert_eq!(audit.provider, "Eastmoney");
        assert_eq!(audit.source, "eastmoney-web");
        assert_eq!(
            audit.request_hash,
            "fb86badeeebfca14c04928026fe295416a2a47c6534250291c066f3a74b67b9c"
        );
        assert_eq!(
            audit.source_at,
            Some("2026-09-14T15:30:00.123456789+08:00")
        );
        assert_eq!(
            audit.observed_at,
            "2026-09-14T15:30:00.987654321+08:00"
        );
        assert_eq!(audit.batch_id, Some("TEST_CODE_MACRO_BATCH_FIRST"));
        assert_eq!(audit.outcome, "available");
        assert_eq!(
            (
                audit.request_count,
                audit.accepted_count,
                audit.rejected_count
            ),
            (1, 2, 0)
        );
        assert_eq!(audit.reason_code, "accepted");
        assert!(!audit.retryable);
        read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
        transaction.commit().unwrap();
        drop(changed_queries);
        drop(changed_parent_source);
        drop(changed_macro_source);
    }))
    .catch_unwind()
    .await;

    let time_cleanup = if time_paused {
        std::panic::catch_unwind(tokio::time::resume)
    } else {
        Ok(())
    };
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
    let reader_cleanup = retry_reader.take().map(|reader| {
        reader.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        })
    });
    let database_cleanup = fixture.store.take().map(|store| {
        store.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        })
    });
    drop(fixture);
    time_cleanup.expect("TEST_CODE resume paused time after caught Retry body");
    macro_cleanup
        .expect("TEST_CODE Macro cleanup panic")
        .expect("TEST_CODE Macro cleanup");
    parent_cleanup.expect("TEST_CODE parent cleanup panic after join");
    if let Some(result) = reader_cleanup {
        result.expect("TEST_CODE retry reader close after joins");
    }
    if let Some(result) = database_cleanup {
        result.expect("TEST_CODE business connection close");
    }
    match body {
        Ok(result) => result.expect("TEST_CODE confirmed-Retry Macro integration body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn single_user_local_macro_response_after_deadline_reopens_unknown_without_rpc_or_audit() {
    let mut fixture = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut macro_server = None;
    let mut deadline_reader: Option<BusinessIntentStore> = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), async {
        let (parent_endpoint, server) = tokio::time::timeout(
            Duration::from_secs(5),
            spawn_macro_parent_listener(DRAGON_TIGER_RECORDS.as_bytes()),
        )
        .await
        .expect("TEST_CODE parent listener deadline");
        parent_server = Some(server);
        macro_server = Some(MacroLoopbackServer::bind().await);
        let server = macro_server.as_ref().unwrap();
        let parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let queries = parent_source.connected_board_queries().await.unwrap();
        let (stocks, config, intent, v9_head) = populate_completed_v9_parent(
            &mut fixture,
            &queries,
            "TEST_CODE_RUN_MACRO_RESPONSE_AFTER_DEADLINE",
        )
        .await;
        fixture
            .chain_post_close()
            .migrate_schema_v9_to_v10()
            .unwrap();
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                lease_request(
                    "TEST_CODE_MACRO_DEADLINE_PARENT_OWNER",
                    68_300_000_000,
                    90_000_000_000,
                    Some(v9_head),
                ),
            )
            .unwrap();
        let parent_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-22T10:30:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-22T10:30:00+08:00")
                .unwrap(),
            request_calls: Cell::new(0),
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .dragon_tiger_preparation_io_v10(
                lease,
                &queries,
                &parent_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &parent_source,
            )
            .unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE real parent reaches Macro guard");
        assert_partial_macro_stop(&stopped);
        drop(io);
        let parent = local.inspect_dragon_tiger(&intent).unwrap();
        assert_complete_batch(parent.batch().unwrap());
        let parent_final = parent.final_bytes().unwrap().to_vec();
        let parent_receipt = parent.audit_receipt().unwrap().clone();
        let parent_run = local.inspect_run(&intent).unwrap();
        let parent_context = parent_run.context().canonical_bytes();
        let parent_head = parent_run.head_version();
        drop(local);
        let earlier_tables = table_names(fixture.connection());
        let earlier_facts = old_fact_rows(fixture.connection(), &earlier_tables);
        let audit_count = fixture.count("data_acquisition_audit");
        let audit_tables = vec![
            "data_acquisition_audit".to_owned(),
            "data_acquisition_audit_chain".to_owned(),
        ];
        let audit_facts = old_fact_rows(fixture.connection(), &audit_tables);
        let old_network = parent_server.as_ref().unwrap().snapshot();
        let old_memberships = parent_server.as_ref().unwrap().membership_snapshot();
        assert_eq!(old_network.dragon_tiger_requests.len(), 1);
        let database = fixture.database();

        assert_eq!(
            fixture
                .chain_post_close()
                .migrate_schema_v10_to_v11()
                .unwrap()
                .schema_version(),
            11
        );
        let macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await,
            server.endpoint().to_owned(),
        );
        let started_at = micros("2026-09-14T15:30:00+08:00");
        let deadline_at = started_at + 15_000_000;
        let original_lease_until = started_at + 30_000_000;
        let clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-14T15:30:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let registered = [
            GeneralWebResearchProvider::SerpApi,
            GeneralWebResearchProvider::Bocha,
            GeneralWebResearchProvider::Tavily,
        ];
        let search_service = macro_search_service(&registered);
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                macro_lease(
                    "TEST_CODE_MACRO_DEADLINE_OWNER",
                    started_at,
                    original_lease_until,
                    parent_head,
                ),
            )
            .unwrap();
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

        let monotonic_started = tokio::time::Instant::now();
        let mut prepared = Box::pin(prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        ));
        tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! {
                result = &mut prepared => panic!("TEST_CODE prepare returned before deadline request receipt: {result:?}"),
                _ = server.wait_for_first_request() => {}
            }
        })
        .await
        .expect("TEST_CODE first deadline request receipt timeout");
        let before_response = server.snapshot();
        assert_eq!(before_response.requests.len(), 1);
        assert_eq!(before_response.authorized, vec![true]);
        assert!(before_response.responses.is_empty());
        assert!(before_response.statuses.is_empty());
        assert_eq!(
            (
                before_response.health_calls,
                before_response.capabilities_calls
            ),
            (0, 0)
        );
        assert!(before_response.unexpected_data_calls.is_empty());

        deadline_reader = Some(BusinessIntentStore::open(&database).unwrap());
        let mut read_local = deadline_reader
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let begun = read_local.inspect_macro(&intent).unwrap();
        assert!(begun.has_unconfirmed_effect());
        assert!(!begun.is_complete());
        assert_eq!(begun.plan().started_at().get(), started_at);
        assert_eq!(begun.plan().deadline_at().get(), deadline_at);
        assert_eq!(begun.parent_final_bytes(), parent_final);
        assert_eq!(begun.attempts().len(), 1);
        let first_attempt = &begun.attempts()[0];
        assert_eq!(first_attempt.attempt_ordinal(), 1);
        assert_eq!(first_attempt.request_bytes(), before_response.requests[0]);
        assert!(first_attempt.result_version().is_none());
        assert!(first_attempt.result_material().is_none());
        assert!(first_attempt.response_bytes().is_none());
        assert!(first_attempt.continuation().is_none());
        assert!(begun.global_news(GlobalNewsProvider::Eastmoney).is_none());
        assert_eq!(
            begun.pending_source_identities(),
            begun.plan().source_identities()
        );
        assert_eq!(begun.pending_source_identities().len(), 5);
        assert_eq!(begun.pending_research_queries().len(), 6);
        let first_plan_bytes = begun.plan_bytes().to_vec();
        let first_request_bytes = first_attempt.request_bytes().to_vec();
        let begin_version = first_attempt.begin_version();
        let begin_head = read_local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(begin_head, begin_version);
        assert!(begin_version > parent_head);
        assert_eq!(
            read_local.inspect_run(&intent).unwrap().lease_generation(),
            3
        );
        drop(read_local);
        let reader = deadline_reader.take().unwrap();
        reader.connection.close().unwrap();

        assert!(deadline_at < original_lease_until);
        clock
            .now
            .set(UtcMicros::try_new(deadline_at).unwrap());
        server.release_response();
        let stopped = match tokio::time::timeout(Duration::from_secs(5), &mut prepared).await {
            Ok(Err(error)) => error,
            Ok(Ok(_)) => panic!("TEST_CODE expired Macro response unexpectedly completed prepare"),
            Err(_) => panic!("TEST_CODE expired Macro response did not return within five seconds"),
        };
        let monotonic_elapsed = monotonic_started.elapsed();
        assert!(
            monotonic_elapsed < Duration::from_secs(15),
            "TEST_CODE deadline classification was not isolated from monotonic timeout: elapsed={monotonic_elapsed:?}"
        );
        let completed_wire = server.snapshot();
        assert_eq!(completed_wire.requests, vec![first_request_bytes.clone()]);
        assert_eq!(completed_wire.authorized, vec![true]);
        assert_eq!(completed_wire.responses.len(), 1);
        assert!(completed_wire.statuses.is_empty());
        assert_eq!(
            (
                completed_wire.health_calls,
                completed_wire.capabilities_calls
            ),
            (0, 0)
        );
        assert!(completed_wire.unexpected_data_calls.is_empty());
        let request = QueryRequest::decode(first_request_bytes.as_slice()).unwrap();
        let request_id = request.context.as_ref().unwrap().request_id.clone();
        assert!(!request_id.is_empty());
        let response = QueryResponse::decode(completed_wire.responses[0].as_slice()).unwrap();
        assert_eq!(response.request_id, request_id);
        assert_eq!(response.operation, Operation::GlobalNews as i32);
        assert_eq!(response.records.len(), 1);
        assert_eq!(response.records[0].schema, "news.global_news");
        assert_eq!(response.records[0].data, NEWS_RECORDS.as_bytes());

        let actual_stop = stopped.downcast_ref::<PreparationStop>();
        let actual_stage = stopped
            .downcast_ref::<PreparationFailure>()
            .map(PreparationFailure::stage);
        assert!(
            matches!(
                actual_stop,
                Some(PreparationStop::ResultUnconfirmed { intent_id })
                    if intent_id == intent.as_str()
            ),
            "TEST_CODE expected ResultUnconfirmed after expired response; actual_stop={actual_stop:?}, actual_stage={actual_stage:?}"
        );
        let failure = stopped.downcast_ref::<PreparationFailure>().unwrap();
        assert_eq!(failure.stage(), PreparationStage::Macro);
        assert_eq!(
            failure.completed_stages().last(),
            Some(&PreparationStage::DragonTiger)
        );
        assert!(!failure
            .completed_stages()
            .contains(&PreparationStage::Macro));
        assert!(failure.failed_stage_may_have_effects());

        drop(prepared);
        let stopped_same_io = match tokio::time::timeout(
            Duration::from_secs(5),
            prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                stocks.clone(),
                None,
                &mut io,
            ),
        )
        .await
        {
            Ok(Err(error)) => error,
            Ok(Ok(_)) => panic!("TEST_CODE expired-response IO unexpectedly resumed"),
            Err(_) => panic!("TEST_CODE expired-response IO did not remain stopped"),
        };
        let same_io_stop = stopped_same_io.downcast_ref::<PreparationStop>();
        let same_io_stage = stopped_same_io
            .downcast_ref::<PreparationFailure>()
            .map(PreparationFailure::stage);
        assert!(
            matches!(
                same_io_stop,
                Some(PreparationStop::ResultUnconfirmed { intent_id })
                    if intent_id == intent.as_str()
            ),
            "TEST_CODE expected stopped IO ResultUnconfirmed; actual_stop={same_io_stop:?}, actual_stage={same_io_stage:?}"
        );
        assert_eq!(server.snapshot(), completed_wire);
        drop(io);

        let unknown = local.inspect_macro(&intent).unwrap();
        assert!(unknown.has_unconfirmed_effect());
        assert!(!unknown.is_complete());
        assert_eq!(unknown.plan_bytes(), first_plan_bytes);
        assert_eq!(unknown.plan().started_at().get(), started_at);
        assert_eq!(unknown.plan().deadline_at().get(), deadline_at);
        assert_eq!(unknown.parent_final_bytes(), parent_final);
        assert_eq!(unknown.attempts().len(), 1);
        let unknown_attempt = &unknown.attempts()[0];
        assert_eq!(unknown_attempt.attempt_ordinal(), 1);
        assert_eq!(unknown_attempt.begin_version(), begin_version);
        assert_eq!(unknown_attempt.request_bytes(), first_request_bytes);
        assert!(unknown_attempt.result_version().is_none());
        assert!(unknown_attempt.result_material().is_none());
        assert!(unknown_attempt.response_bytes().is_none());
        assert!(unknown_attempt.continuation().is_none());
        assert!(unknown.global_news(GlobalNewsProvider::Eastmoney).is_none());
        assert_eq!(
            unknown.pending_source_identities(),
            begun.pending_source_identities()
        );
        assert_eq!(
            unknown.pending_research_queries(),
            begun.pending_research_queries()
        );
        assert_eq!(local.inspect_run(&intent).unwrap().head_version(), begin_head);
        assert_eq!(
            local.inspect_run(&intent).unwrap().lease_generation(),
            3
        );
        assert_eq!(
            local
                .inspect_run(&intent)
                .unwrap()
                .context()
                .canonical_bytes(),
            parent_context
        );
        assert_eq!(clock.observation_calls.get(), 1);
        drop(local);
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count);
        assert_eq!(old_fact_rows(fixture.connection(), &audit_tables), audit_facts);
        assert_eq!(
            old_fact_rows(fixture.connection(), &earlier_tables),
            earlier_facts
        );
        let transaction = fixture.connection().unchecked_transaction().unwrap();
        read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
        transaction.commit().unwrap();

        drop(queries);
        drop(parent_source);
        drop(macro_source);
        fixture.reopen();
        server.change_response_for_reopen();
        let changed_macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await,
            server.endpoint().to_owned(),
        );
        let changed_parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let changed_queries = changed_parent_source.connected_board_queries().await.unwrap();
        let reopened_at = started_at + 31_000_000;
        assert!(reopened_at > original_lease_until);
        let changed_clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(reopened_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-15T15:45:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                macro_lease(
                    "TEST_CODE_MACRO_DEADLINE_REOPENED_OWNER",
                    reopened_at,
                    started_at + 60_000_000,
                    begin_head,
                ),
            )
            .unwrap();
        let resumed_head = local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(resumed_head, begin_head + 1);
        assert_eq!(
            local.inspect_run(&intent).unwrap().lease_generation(),
            4
        );
        let mut io = local
            .macro_preparation_io_v11(
                lease,
                &changed_queries,
                &changed_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &changed_parent_source,
                &changed_macro_source,
                &search_service,
            )
            .unwrap();
        let stopped = match tokio::time::timeout(
            Duration::from_secs(5),
            prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                stocks,
                None,
                &mut io,
            ),
        )
        .await
        {
            Ok(Err(error)) => error,
            Ok(Ok(_)) => panic!("TEST_CODE expired Unknown unexpectedly completed after reopen"),
            Err(_) => panic!("TEST_CODE expired Unknown did not stop after reopen"),
        };
        let reopened_stop = stopped.downcast_ref::<PreparationStop>();
        let reopened_stage = stopped
            .downcast_ref::<PreparationFailure>()
            .map(PreparationFailure::stage);
        assert!(
            matches!(
                reopened_stop,
                Some(PreparationStop::IncompleteOnReopen { intent_id })
                    if intent_id == intent.as_str()
            ),
            "TEST_CODE expected IncompleteOnReopen before expired-plan handling; actual_stop={reopened_stop:?}, actual_stage={reopened_stage:?}"
        );
        assert!(matches!(
            stopped.downcast_ref::<ChainPostCloseError>(),
            Some(ChainPostCloseError::IncompleteEffect { intent_id })
                if intent_id == intent.as_str()
        ));
        let failure = stopped.downcast_ref::<PreparationFailure>().unwrap();
        assert_eq!(failure.stage(), PreparationStage::Macro);
        assert_eq!(
            failure.completed_stages().last(),
            Some(&PreparationStage::DragonTiger)
        );
        assert!(!failure
            .completed_stages()
            .contains(&PreparationStage::Macro));
        drop(io);

        let reopened = local.inspect_macro(&intent).unwrap();
        assert!(reopened.has_unconfirmed_effect());
        assert!(!reopened.is_complete());
        assert_eq!(reopened.plan_bytes(), first_plan_bytes);
        assert_eq!(reopened.plan().started_at().get(), started_at);
        assert_eq!(reopened.plan().deadline_at().get(), deadline_at);
        assert_eq!(reopened.parent_final_bytes(), parent_final);
        assert_eq!(reopened.pending_source_identities(), unknown.pending_source_identities());
        assert_eq!(
            reopened.pending_research_queries(),
            unknown.pending_research_queries()
        );
        assert_eq!(reopened.attempts().len(), 1);
        let reopened_attempt = &reopened.attempts()[0];
        assert_eq!(reopened_attempt.attempt_ordinal(), 1);
        assert_eq!(reopened_attempt.begin_version(), begin_version);
        assert_eq!(reopened_attempt.request_bytes(), first_request_bytes);
        assert!(reopened_attempt.result_version().is_none());
        assert!(reopened_attempt.result_material().is_none());
        assert!(reopened_attempt.response_bytes().is_none());
        assert!(reopened_attempt.continuation().is_none());
        assert!(reopened.global_news(GlobalNewsProvider::Eastmoney).is_none());
        assert_eq!(changed_clock.observation_calls.get(), 0);
        assert_eq!(local.inspect_run(&intent).unwrap().head_version(), resumed_head);
        assert_eq!(
            local.inspect_run(&intent).unwrap().lease_generation(),
            4
        );
        assert_eq!(
            local
                .inspect_run(&intent)
                .unwrap()
                .context()
                .canonical_bytes(),
            parent_context
        );
        drop(local);

        assert_eq!(server.snapshot(), completed_wire);
        assert_eq!(server.snapshot().requests.len(), 1);
        assert_eq!(server.snapshot().responses.len(), 1);
        assert!(server.snapshot().statuses.is_empty());
        assert_eq!(parent_server.as_ref().unwrap().snapshot(), old_network);
        assert_eq!(
            parent_server.as_ref().unwrap().membership_snapshot(),
            old_memberships
        );
        assert_eq!(
            old_fact_rows(fixture.connection(), &earlier_tables),
            earlier_facts
        );
        assert_eq!(old_fact_rows(fixture.connection(), &audit_tables), audit_facts);
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count);
        let transaction = fixture.connection().unchecked_transaction().unwrap();
        read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
        transaction.commit().unwrap();
        drop(changed_queries);
        drop(changed_parent_source);
        drop(changed_macro_source);
    }))
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
    let reader_cleanup = deadline_reader.take().map(|reader| {
        reader.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        })
    });
    let database_cleanup = fixture.store.take().map(|store| {
        store.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        })
    });
    drop(fixture);
    macro_cleanup
        .expect("TEST_CODE Macro cleanup panic")
        .expect("TEST_CODE Macro cleanup");
    parent_cleanup.expect("TEST_CODE parent cleanup panic after join");
    if let Some(result) = reader_cleanup {
        result.expect("TEST_CODE deadline reader close after joins");
    }
    if let Some(result) = database_cleanup {
        result.expect("TEST_CODE business connection close");
    }
    match body {
        Ok(result) => result.expect("TEST_CODE deadline-return Macro integration body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

use crate::grpc_client::errors::GrpcError as MacroAuthGrpcError;

#[tokio::test]
async fn single_user_local_macro_invalid_instance_auth_leaves_no_plan_before_valid_reopen() {
    let mut fixture = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut macro_server = None;
    let mut auth_reader: Option<BusinessIntentStore> = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), async {
        let (parent_endpoint, server) = tokio::time::timeout(
            Duration::from_secs(5),
            spawn_macro_parent_listener(DRAGON_TIGER_RECORDS.as_bytes()),
        )
        .await
        .expect("TEST_CODE parent listener deadline");
        parent_server = Some(server);
        macro_server = Some(MacroLoopbackServer::bind().await);
        let server = macro_server.as_ref().unwrap();
        let parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let queries = parent_source.connected_board_queries().await.unwrap();
        let (stocks, config, intent, v9_head) = populate_completed_v9_parent(
            &mut fixture,
            &queries,
            "TEST_CODE_RUN_MACRO_INVALID_INSTANCE_AUTH",
        )
        .await;
        fixture
            .chain_post_close()
            .migrate_schema_v9_to_v10()
            .unwrap();
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                lease_request(
                    "TEST_CODE_MACRO_AUTH_PARENT_OWNER",
                    68_300_000_000,
                    90_000_000_000,
                    Some(v9_head),
                ),
            )
            .unwrap();
        let parent_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-22T10:30:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-22T10:30:00+08:00")
                .unwrap(),
            request_calls: Cell::new(0),
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .dragon_tiger_preparation_io_v10(
                lease,
                &queries,
                &parent_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &parent_source,
            )
            .unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE real parent reaches Macro guard");
        assert_partial_macro_stop(&stopped);
        drop(io);
        let parent = local.inspect_dragon_tiger(&intent).unwrap();
        assert_complete_batch(parent.batch().unwrap());
        let parent_final = parent.final_bytes().unwrap().to_vec();
        let parent_receipt = parent.audit_receipt().unwrap().clone();
        let parent_run = local.inspect_run(&intent).unwrap();
        let parent_context = parent_run.context().canonical_bytes();
        let parent_head = parent_run.head_version();
        drop(local);
        let earlier_tables = table_names(fixture.connection());
        let earlier_facts = old_fact_rows(fixture.connection(), &earlier_tables);
        let audit_count = fixture.count("data_acquisition_audit");
        let audit_tables = vec![
            "data_acquisition_audit".to_owned(),
            "data_acquisition_audit_chain".to_owned(),
        ];
        let audit_facts = old_fact_rows(fixture.connection(), &audit_tables);
        let old_network = parent_server.as_ref().unwrap().snapshot();
        let old_memberships = parent_server.as_ref().unwrap().membership_snapshot();
        assert_eq!(old_network.dragon_tiger_requests.len(), 1);

        let database = fixture.database();
        assert_eq!(
            fixture
                .chain_post_close()
                .migrate_schema_v10_to_v11()
                .unwrap()
                .schema_version(),
            11
        );
        let macro_tables = crate::push_foundation::intent_store::chain_post_close::macro_stage::TABLES
            .iter()
            .map(|table| (*table).to_owned())
            .collect::<Vec<_>>();
        let empty_macro_facts = old_fact_rows(fixture.connection(), &macro_tables);
        assert!(empty_macro_facts.values().all(|rows| rows.is_empty()));

        let invalid_macro_source = GrpcSource::from_macro_loopback_test_client(
            server
                .connect_with_invalid_instance_bearer_for_test()
                .await,
            server.endpoint().to_owned(),
        );
        let started_at = micros("2026-09-14T15:30:00+08:00");
        let invalid_lease_until = started_at + 2_000_000;
        let invalid_clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-14T15:30:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let registered = [
            GeneralWebResearchProvider::SerpApi,
            GeneralWebResearchProvider::Bocha,
            GeneralWebResearchProvider::Tavily,
        ];
        let search_service = macro_search_service(&registered);
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                macro_lease(
                    "TEST_CODE_MACRO_INVALID_AUTH_OWNER",
                    started_at,
                    invalid_lease_until,
                    parent_head,
                ),
            )
            .unwrap();
        let invalid_resume_head = local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(invalid_resume_head, parent_head + 1);
        assert_eq!(
            local.inspect_run(&intent).unwrap().lease_generation(),
            3
        );
        let mut io = local
            .macro_preparation_io_v11(
                lease,
                &queries,
                &invalid_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &parent_source,
                &invalid_macro_source,
                &search_service,
            )
            .unwrap();
        let rejected = match tokio::time::timeout(
            Duration::from_secs(5),
            prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                stocks.clone(),
                None,
                &mut io,
            ),
        )
        .await
        {
            Ok(Err(error)) => error,
            Ok(Ok(_)) => panic!("TEST_CODE invalid instance auth unexpectedly completed prepare"),
            Err(_) => panic!("TEST_CODE invalid instance auth did not reject within five seconds"),
        };
        let actual_stop = rejected.downcast_ref::<PreparationStop>();
        let actual_stage = rejected
            .downcast_ref::<PreparationFailure>()
            .map(PreparationFailure::stage);
        assert!(
            matches!(
                actual_stop,
                Some(PreparationStop::AuthorityRejected { intent_id })
                    if intent_id == intent.as_str()
            ),
            "TEST_CODE expected local auth AuthorityRejected; actual_stop={actual_stop:?}, actual_stage={actual_stage:?}"
        );
        let auth_error = rejected
            .downcast_ref::<MacroAuthGrpcError>()
            .expect("TEST_CODE local auth rejection retains typed gRPC error");
        assert!(matches!(
            auth_error,
            MacroAuthGrpcError::Unauthenticated { .. }
        ));
        let details = auth_error.details();
        assert_eq!(details.code, "unauthenticated");
        assert_eq!(details.request_id, None);
        assert_eq!(details.method, None);
        assert_eq!(details.provider, None);
        assert_eq!(details.reason_code, None);
        assert_eq!(details.retryable, None);
        assert_eq!(details.admission, None);
        assert_eq!(details.evidence_code, None);
        assert_eq!(details.evidence_field, None);
        assert_eq!(details.record_index, None);
        assert_eq!(auth_error.safe_diagnostic(), None);
        let failure = rejected.downcast_ref::<PreparationFailure>().unwrap();
        assert_eq!(failure.stage(), PreparationStage::Macro);
        assert_eq!(
            failure.completed_stages().last(),
            Some(&PreparationStage::DragonTiger)
        );
        assert!(!failure
            .completed_stages()
            .contains(&PreparationStage::Macro));
        assert!(failure.failed_stage_may_have_effects());
        assert_eq!(invalid_clock.observation_calls.get(), 1);
        let rejected_wire = server.snapshot();
        assert!(rejected_wire.requests.is_empty());
        assert!(rejected_wire.authorized.is_empty());
        assert!(rejected_wire.responses.is_empty());
        assert!(rejected_wire.statuses.is_empty());
        assert_eq!(
            (
                rejected_wire.health_calls,
                rejected_wire.capabilities_calls
            ),
            (0, 0)
        );
        assert!(rejected_wire.unexpected_data_calls.is_empty());
        drop(io);

        let missing = match local.inspect_macro(&intent) {
            Ok(_) => panic!("TEST_CODE local auth rejection unexpectedly persisted Macro plan"),
            Err(error) => error,
        };
        assert_eq!(missing, ChainPostCloseError::MacroNotStarted);
        assert_eq!(
            local.inspect_run(&intent).unwrap().head_version(),
            invalid_resume_head
        );
        assert_eq!(
            local.inspect_run(&intent).unwrap().lease_generation(),
            3
        );
        assert_eq!(
            local
                .inspect_run(&intent)
                .unwrap()
                .context()
                .canonical_bytes(),
            parent_context
        );
        drop(local);
        assert_eq!(
            old_fact_rows(fixture.connection(), &macro_tables),
            empty_macro_facts
        );
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count);
        assert_eq!(old_fact_rows(fixture.connection(), &audit_tables), audit_facts);
        assert_eq!(
            old_fact_rows(fixture.connection(), &earlier_tables),
            earlier_facts
        );
        assert_eq!(parent_server.as_ref().unwrap().snapshot(), old_network);
        assert_eq!(
            parent_server.as_ref().unwrap().membership_snapshot(),
            old_memberships
        );
        let transaction = fixture.connection().unchecked_transaction().unwrap();
        read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
        transaction.commit().unwrap();

        drop(queries);
        drop(parent_source);
        drop(invalid_macro_source);
        fixture.reopen();

        let valid_macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await,
            server.endpoint().to_owned(),
        );
        let valid_parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await,
        );
        let valid_queries = valid_parent_source.connected_board_queries().await.unwrap();
        let reopened_at = started_at + 3_000_000;
        assert!(reopened_at > invalid_lease_until);
        let valid_clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(reopened_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-15T15:45:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                macro_lease(
                    "TEST_CODE_MACRO_VALID_AUTH_REOPENED_OWNER",
                    reopened_at,
                    started_at + 30_000_000,
                    invalid_resume_head,
                ),
            )
            .unwrap();
        let valid_resume_head = local.inspect_run(&intent).unwrap().head_version();
        assert_eq!(valid_resume_head, invalid_resume_head + 1);
        assert_eq!(
            local.inspect_run(&intent).unwrap().lease_generation(),
            4
        );
        assert_eq!(
            local
                .inspect_run(&intent)
                .unwrap()
                .context()
                .canonical_bytes(),
            parent_context
        );
        let still_missing = match local.inspect_macro(&intent) {
            Ok(_) => panic!("TEST_CODE valid reopen found a plan from rejected authorization"),
            Err(error) => error,
        };
        assert_eq!(still_missing, ChainPostCloseError::MacroNotStarted);
        let mut io = local
            .macro_preparation_io_v11(
                lease,
                &valid_queries,
                &valid_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &valid_parent_source,
                &valid_macro_source,
                &search_service,
            )
            .unwrap();
        let mut prepared = Box::pin(prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut io,
        ));
        tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! {
                result = &mut prepared => panic!("TEST_CODE valid auth prepare returned before first persisted request: {result:?}"),
                _ = server.wait_for_first_request() => {}
            }
        })
        .await
        .expect("TEST_CODE valid auth first request timeout");
        let valid_begin_wire = server.snapshot();
        assert_eq!(valid_begin_wire.requests.len(), 1);
        assert_eq!(valid_begin_wire.authorized, vec![true]);
        assert!(valid_begin_wire.responses.is_empty());
        assert!(valid_begin_wire.statuses.is_empty());
        assert_eq!(
            (
                valid_begin_wire.health_calls,
                valid_begin_wire.capabilities_calls
            ),
            (0, 0)
        );
        assert!(valid_begin_wire.unexpected_data_calls.is_empty());

        auth_reader = Some(BusinessIntentStore::open(&database).unwrap());
        let mut read_local = auth_reader
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let begun = read_local.inspect_macro(&intent).unwrap();
        assert!(begun.has_unconfirmed_effect());
        assert!(!begun.is_complete());
        assert_eq!(begun.plan().started_at().get(), reopened_at);
        assert_eq!(
            begun.plan().deadline_at().get(),
            reopened_at + 15_000_000
        );
        assert_eq!(begun.plan().observed_local(), "2026-09-15T15:45:00+08:00");
        assert_eq!(begun.parent_final_bytes(), parent_final);
        assert_eq!(begun.attempts().len(), 1);
        let begun_attempt = &begun.attempts()[0];
        assert_eq!(begun_attempt.attempt_ordinal(), 1);
        assert_eq!(begun_attempt.request_bytes(), valid_begin_wire.requests[0]);
        assert!(begun_attempt.result_version().is_none());
        assert!(begun_attempt.result_material().is_none());
        assert!(begun_attempt.response_bytes().is_none());
        assert!(begun_attempt.continuation().is_none());
        assert!(begun.global_news(GlobalNewsProvider::Eastmoney).is_none());
        assert_eq!(
            begun.pending_source_identities(),
            begun.plan().source_identities()
        );
        assert_eq!(begun.pending_source_identities().len(), 5);
        assert_eq!(begun.pending_research_queries().len(), 6);
        let valid_plan_bytes = begun.plan_bytes().to_vec();
        let valid_request_bytes = begun_attempt.request_bytes().to_vec();
        let valid_begin_version = begun_attempt.begin_version();
        assert_eq!(
            read_local.inspect_run(&intent).unwrap().head_version(),
            valid_begin_version
        );
        assert!(valid_begin_version > valid_resume_head);
        assert_eq!(
            read_local.inspect_run(&intent).unwrap().lease_generation(),
            4
        );
        drop(read_local);
        let reader = auth_reader.take().unwrap();
        reader.connection.close().unwrap();

        valid_clock
            .now
            .set(UtcMicros::try_new(reopened_at + 1_000_000).unwrap());
        server.release_response();
        let stopped = match tokio::time::timeout(Duration::from_secs(5), &mut prepared).await {
            Ok(Err(error)) => error,
            Ok(Ok(_)) => panic!("TEST_CODE valid auth unexpectedly completed all Macro sources"),
            Err(_) => panic!("TEST_CODE valid auth response did not return within five seconds"),
        };
        assert_partial_macro_stop(&stopped);
        drop(prepared);
        drop(io);

        let recovered = local.inspect_macro(&intent).unwrap();
        assert!(!recovered.has_unconfirmed_effect());
        assert!(!recovered.is_complete());
        assert_eq!(recovered.plan_bytes(), valid_plan_bytes);
        assert_eq!(recovered.plan().started_at().get(), reopened_at);
        assert_eq!(
            recovered.plan().deadline_at().get(),
            reopened_at + 15_000_000
        );
        assert_eq!(recovered.parent_final_bytes(), parent_final);
        assert_eq!(recovered.pending_source_identities().len(), 4);
        assert_eq!(recovered.pending_research_queries().len(), 6);
        assert_eq!(recovered.attempts().len(), 1);
        let attempt = &recovered.attempts()[0];
        assert_eq!(attempt.attempt_ordinal(), 1);
        assert_eq!(attempt.begin_version(), valid_begin_version);
        assert!(attempt.result_version().unwrap() > attempt.begin_version());
        assert_eq!(attempt.request_bytes(), valid_request_bytes);
        assert_eq!(attempt.continuation(), Some(MacroContinuation::Terminal));
        let valid_response_bytes = attempt.response_bytes().unwrap().to_vec();
        let source = recovered
            .global_news(GlobalNewsProvider::Eastmoney)
            .unwrap();
        assert_eq!(source.retry_policy(), (4, 1000, 60_000, 200));
        assert_eq!(source.profile(), "LocalBridgeV1");
        assert_eq!(source.acquisition_authority(), None);
        assert_native_news(source.batch().unwrap());
        let native = source.final_bytes().unwrap().to_vec();
        assert!(!native.is_empty());
        let receipt = source.audit_receipt().unwrap().clone();
        assert_eq!(receipt.previous_outcome, None);
        assert_eq!(receipt.current_outcome, "available");
        assert_eq!(valid_clock.observation_calls.get(), 1);
        assert_eq!(
            local.inspect_run(&intent).unwrap().head_version(),
            attempt.result_version().unwrap() + 1
        );
        assert_eq!(
            local.inspect_run(&intent).unwrap().lease_generation(),
            4
        );
        assert_eq!(
            local
                .inspect_run(&intent)
                .unwrap()
                .context()
                .canonical_bytes(),
            parent_context
        );
        drop(local);

        let final_wire = server.snapshot();
        assert_eq!(final_wire.requests, vec![valid_request_bytes.clone()]);
        assert_eq!(final_wire.authorized, vec![true]);
        assert_eq!(final_wire.responses, vec![valid_response_bytes.clone()]);
        assert!(final_wire.statuses.is_empty());
        assert_eq!(
            (final_wire.health_calls, final_wire.capabilities_calls),
            (0, 0)
        );
        assert!(final_wire.unexpected_data_calls.is_empty());
        let request = QueryRequest::decode(valid_request_bytes.as_slice()).unwrap();
        let context = request.context.unwrap();
        assert_eq!(context.protocol_version, 1);
        let request_id = context.request_id;
        assert!(!request_id.is_empty());
        assert!(request.preferred_provider.is_empty());
        assert!(!request.allow_unadmitted);
        let payload = request.payload.unwrap();
        assert_eq!(payload.schema, "news.global_news");
        assert_eq!(payload.schema_version, 1);
        assert_eq!(payload.content_type, "application/json; charset=utf-8");
        assert_eq!(payload.data, br#"{"limit":20,"provider":"Eastmoney"}"#);
        let response = QueryResponse::decode(valid_response_bytes.as_slice()).unwrap();
        assert_eq!(response.request_id, request_id);
        assert_eq!(response.operation, Operation::GlobalNews as i32);
        assert_eq!(response.records.len(), 1);
        assert_eq!(response.records[0].schema, "news.global_news");
        assert_eq!(response.records[0].data, NEWS_RECORDS.as_bytes());
        assert_eq!(parent_server.as_ref().unwrap().snapshot(), old_network);
        assert_eq!(
            parent_server.as_ref().unwrap().membership_snapshot(),
            old_memberships
        );
        assert_eq!(
            old_fact_rows(fixture.connection(), &earlier_tables),
            earlier_facts
        );
        let final_macro_facts = old_fact_rows(fixture.connection(), &macro_tables);
        let expected_macro_rows = [
            ("chain_post_close_macro_plans", 1),
            ("chain_post_close_macro_request_plans", 1),
            ("chain_post_close_macro_readiness_episode_plans", 0),
            ("chain_post_close_macro_control_attempt_begins", 0),
            ("chain_post_close_macro_control_attempt_results", 0),
            ("chain_post_close_macro_attempt_begins", 1),
            ("chain_post_close_macro_attempt_results", 1),
            ("chain_post_close_macro_source_finals", 1),
        ];
        assert_eq!(final_macro_facts.len(), expected_macro_rows.len());
        for (table, expected_rows) in expected_macro_rows {
            assert_eq!(
                final_macro_facts
                    .get(table)
                    .unwrap_or_else(|| panic!("TEST_CODE missing Local Macro table {table}"))
                    .len(),
                expected_rows,
                "TEST_CODE unexpected Local Macro row count for {table}"
            );
        }
        assert_eq!(fixture.count("data_acquisition_audit"), audit_count + 1);
        let final_audit_facts = old_fact_rows(fixture.connection(), &audit_tables);
        for (table, original_rows) in &audit_facts {
            let rows = &final_audit_facts[table];
            assert_eq!(rows.len(), original_rows.len() + 1);
            assert_eq!(&rows[..original_rows.len()], original_rows.as_slice());
        }
        let transaction = fixture.connection().unchecked_transaction().unwrap();
        let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
        assert_eq!(verified.receipt(), &receipt);
        let audit = verified.record();
        assert_eq!(audit.capability, "GlobalNews-Eastmoney");
        assert_eq!(audit.provider, "Eastmoney");
        assert_eq!(audit.source, "eastmoney-web");
        assert_eq!(
            audit.request_hash,
            "fb86badeeebfca14c04928026fe295416a2a47c6534250291c066f3a74b67b9c"
        );
        assert_eq!(
            audit.source_at,
            Some("2026-09-14T15:30:00.123456789+08:00")
        );
        assert_eq!(
            audit.observed_at,
            "2026-09-14T15:30:00.987654321+08:00"
        );
        assert_eq!(audit.batch_id, Some("TEST_CODE_MACRO_BATCH_FIRST"));
        assert_eq!(audit.outcome, "available");
        assert_eq!(
            (
                audit.request_count,
                audit.accepted_count,
                audit.rejected_count
            ),
            (1, 2, 0)
        );
        assert_eq!(audit.reason_code, "accepted");
        assert!(!audit.retryable);
        read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
        transaction.commit().unwrap();
        drop(valid_queries);
        drop(valid_parent_source);
        drop(valid_macro_source);
    }))
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
    let reader_cleanup = auth_reader.take().map(|reader| {
        reader.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        })
    });
    let database_cleanup = fixture.store.take().map(|store| {
        store.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        })
    });
    drop(fixture);
    macro_cleanup
        .expect("TEST_CODE Macro cleanup panic")
        .expect("TEST_CODE Macro cleanup");
    parent_cleanup.expect("TEST_CODE parent cleanup panic after join");
    if let Some(result) = reader_cleanup {
        result.expect("TEST_CODE auth reader close after joins");
    }
    if let Some(result) = database_cleanup {
        result.expect("TEST_CODE business connection close");
    }
    match body {
        Ok(result) => result.expect("TEST_CODE invalid-auth Macro integration body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[path = "chain_post_close_macro_control_tests.rs"]
mod control_tests;
#[path = "chain_post_close_macro_full_tests.rs"]
mod full_tests;
#[path = "chain_post_close_macro_control_recovery_tests.rs"]
mod control_recovery_tests;
#[path = "chain_post_close_macro_control_connect_tests.rs"]
mod control_connect_tests;
#[path = "chain_post_close_macro_control_caps_connect_tests.rs"]
mod control_caps_connect_tests;
#[path = "chain_post_close_macro_control_unknown_commit_tests.rs"]
mod control_unknown_commit_tests;
#[path = "chain_post_close_macro_pre_effect_refusal_tests.rs"]
mod pre_effect_refusal_tests;
#[path = "chain_post_close_macro_route_refusal_tests.rs"]
mod route_refusal_tests;
#[path = "chain_post_close_macro_external_retry_tests.rs"]
mod external_retry_tests;
#[path = "chain_post_close_macro_corruption_tests.rs"]
mod corruption_tests;
#[path = "chain_post_close_schema_v11_migration_tests.rs"]
mod v11_migration_tests;
#[path = "chain_post_close_schema_v12_migration_tests.rs"]
mod v12_migration_tests;
#[path = "chain_post_close_models_tests.rs"]
mod models_tests;

#[tokio::test]
async fn single_user_external_macro_confirmed_controls_reopen_and_execute_only_original_data() {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(120),
        control_tests::establish_confirmed_external_first_source(
            &mut business,
            &mut parent_server,
            &mut external_server,
            "TEST_CODE_RUN_EXTERNAL_MACRO_CONFIRMED_CONTROLS",
        ),
    ))
    .catch_unwind()
    .await;

    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        "External same-store",
    )
    .await;
    drop(business);
    match body {
        Ok(result) => {
            result.expect("TEST_CODE External same-store body deadline");
        }
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn single_user_external_macro_health_not_ready_recovers_without_capabilities_or_data() {
    control_tests::run_control_rejection(control_tests::RejectionCase::HealthNotReady).await;
}
