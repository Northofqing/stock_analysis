use super::*;
use crate::grpc_client::client::macro_full_loopback_fixture::{
    Call, Lane, MacroFullLoopbackServer, QUERIES,
};
use crate::search_service::macro_news::{runner::QueryKey, NativeOutcome};
use crate::push_foundation::intent_store::chain_post_close::macro_native::ExpiryBasis;

// Frozen from the pre-Task3 public wrapper's documented formatting, worked by
// hand from the fixture facts. Both actual prepare and the native StageFinal
// read-only Interface must equal these independent bytes before and after reopen.
// Do not replace this literal with a call to the renderer under test.
const EXPECTED_MACRO: &str = concat!(
    "## 📡 今日宏观 / 市场背景（2026年09月14日）\n\n",
    "### 📰 东方财富财经要闻\n",
    "_source=eastmoney-web source_at=2026-09-14T15:30:00.123456789+08:00 batch_id=TEST_CODE_FULL_Eastmoney_\n",
    "- **TEST_CODE Eastmoney 财经🌏** \u{0060}2026-09-14T07:29:59.123456789+00:00\u{0060}（TEST_CODE 发布者）\n",
    "  TEST_CODE 摘要  \n\n",
    "### 🧭 财联社电报\n",
    "_source=cls-v1 source_at=2026-09-14T15:30:00.123456789+08:00 batch_id=TEST_CODE_FULL_Cailianpress_\n",
    "- **TEST_CODE Cailianpress 财经🌏** \u{0060}2026-09-14T07:29:59.123456789+00:00\u{0060}（TEST_CODE 发布者）\n",
    "  TEST_CODE 摘要  \n\n",
    "### 📣 金十快讯\n",
    "_source=jin10-flash-v1 source_at=2026-09-14T15:30:00.123456789+08:00 batch_id=TEST_CODE_FULL_Jin10_\n",
    "- **TEST_CODE Jin10 财经🌏** \u{0060}2026-09-14T07:29:59.123456789+00:00\u{0060}（TEST_CODE 发布者）\n",
    "  TEST_CODE 摘要  \n\n",
    "### 🌐 澎湃财经\n",
    "_source=thepaper-finance-v1 source_at=2026-09-14T15:30:00.123456789+08:00 batch_id=TEST_CODE_FULL_ThePaper_\n",
    "- **TEST_CODE ThePaper 财经🌏** \u{0060}2026-09-14T07:29:59.123456789+00:00\u{0060}（TEST_CODE 发布者）\n",
    "  TEST_CODE 摘要  \n\n",
    "### 📊 最新经济数据发布（金十）\n",
    "_source=jin10-flash-v1 source_at=2026-09-14T15:30:00.123456789+08:00 batch_id=TEST_CODE_FULL_E_\n",
    "- \u{0060}2026-09-14T07:30:00.987654321+00:00\u{0060} ★★★ **[TEST_CODE 中国]** TEST_CODE 实际发布🌏\n",
    "  周期 2026-08 | 前值 -0.0000000000000001 | 公布 3.141592653589793 | 修正  | 单位 % | 影响 TEST_CODE 原始影响  \n\n",
    "### 🔎 通用网页研究发现（ResearchOnly；不得作为金融事实）\n### 🇨🇳 A股市场动态\n",
    "- **TEST_CODE 网页维度1** 2026-09-14T15:29:00.123456789+08:00  \n  TEST_CODE 研究摘要🌏  \n\n",
    "### 🔎 通用网页研究发现（ResearchOnly；不得作为金融事实）\n### 🌍 国际财经 / 地缘政治\n",
    "- **TEST_CODE 网页维度2** 2026-09-14T15:29:00.123456789+08:00  \n  TEST_CODE 研究摘要🌏  \n\n",
    "### 🔎 通用网页研究发现（ResearchOnly；不得作为金融事实）\n### 🇺🇸 美股 / 大宗商品\n",
    "- **TEST_CODE 网页维度3** 2026-09-14T15:29:00.123456789+08:00  \n  TEST_CODE 研究摘要🌏  \n\n",
    "### 🔎 通用网页研究发现（ResearchOnly；不得作为金融事实）\n### 📋 宏观政策\n",
    "- **TEST_CODE 网页维度4** 2026-09-14T15:29:00.123456789+08:00  \n  TEST_CODE 研究摘要🌏  \n\n",
    "### 🔎 通用网页研究发现（ResearchOnly；不得作为金融事实）\n### 🏦 投行观点（高盛/摩根/美银）\n",
    "- **TEST_CODE 网页维度5** 2026-09-14T15:29:00.123456789+08:00  \n  TEST_CODE 研究摘要🌏  ",
);

fn assert_models_stop(callsite: &str, error: &anyhow::Error, observed_calls: &[Call]) {
    assert!(
        matches!(error.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::StageNotMigrated { next: UnmigratedStage::ModelsSearchAndReport })),
        "TEST_CODE real prepare must complete five Gateway lanes and six Web dimensions, \
         then stop at ModelsSearchAndReport at {callsite}; received {error:?}; \
         actual Macro calls={observed_calls:?}"
    );
    let failure = error.downcast_ref::<PreparationFailure>().unwrap();
    assert_eq!(
        failure.macro_context().as_bytes(),
        EXPECTED_MACRO.as_bytes(),
        "TEST_CODE Macro body mismatch at {callsite}; error={error:?}; stage={:?}; \
         actual Macro calls={observed_calls:?}; actual Macro body={:?}",
        failure.stage(),
        failure.macro_context()
    );
    assert_eq!(failure.stage(), PreparationStage::ModelsSearchAndReport);
    assert_eq!(
        failure.completed_stages().last(),
        Some(&PreparationStage::Macro)
    );
    assert_eq!(
        failure.lhb_map()["TEST_CODE_600001"].to_bits(),
        12.5_f64.to_bits()
    );
    assert_eq!(
        failure.lhb_source().source(),
        Some("TEST_CODE_LHB_SOURCE_FIRST")
    );
}

#[test]
fn legacy_macro_public_future_remains_send_without_polling() {
    fn require_send<T: std::future::Future + Send>(_: T) {}
    let service = macro_search_service(&[]);
    // Creating and dropping the actual public future executes no async body.
    require_send(service.search_macro_news(3));
}

fn assert_economic_native(outcome: &NativeOutcome) {
    let NativeOutcome::Economic(Ok(batch)) = outcome else {
        panic!("TEST_CODE expected full Economic native batch: {outcome:?}");
    };
    assert!(!batch.is_verified_empty());
    assert_eq!(
        batch.evidence().provider,
        crate::market_domain::ProviderId::Jin10
    );
    assert_eq!(batch.evidence().source, "jin10-flash-v1");
    assert_eq!(
        batch.evidence().source_at.as_deref(),
        Some("2026-09-14T15:30:00.123456789+08:00")
    );
    assert_eq!(
        batch.evidence().observed_at,
        "2026-09-14T15:30:00.987654321+08:00"
    );
    assert_eq!(batch.evidence().batch_id, "TEST_CODE_FULL_E");
    let expected = crate::data_gateway::EconomicReleaseFact {
        event_id: "TEST_CODE_RELEASE_一".to_owned(),
        indicator_id: 123,
        country: "TEST_CODE 中国".to_owned(),
        name: "TEST_CODE 实际发布🌏".to_owned(),
        period: Some("2026-08".to_owned()),
        scheduled_at: DateTime::parse_from_rfc3339("2026-09-14T07:29:00.123456789Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        released_at: DateTime::parse_from_rfc3339("2026-09-14T07:30:00.987654321Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        previous: Some("-0.0000000000000001".to_owned()),
        consensus: None,
        actual: Some("3.141592653589793".to_owned()),
        revised: Some(String::new()),
        unit: Some("%".to_owned()),
        importance: 3,
        impact: Some("TEST_CODE 原始影响  ".to_owned()),
        evidence: crate::market_domain::SourceEvidence::new(
            crate::market_domain::ProviderId::Jin10,
            "2026-09-14T15:30:00.987654321+08:00",
            "TEST_CODE_FULL_E",
        )
        .unwrap()
        .with_source_at("2026-09-14T15:30:00.123456789+08:00")
        .unwrap(),
    };
    assert_eq!(batch.records(), &[expected]);
}

fn assert_web_native(
    outcome: &NativeOutcome,
    dimension: usize,
    provider: GeneralWebResearchProvider,
) {
    use crate::data_gateway::general_web_research::{
        GeneralWebResearchBatch, GeneralWebResearchBatchEvidence, GeneralWebResearchRecord,
        GeneralWebResearchRecordEvidence, PublicationTimeQuality, ResearchUseScope,
    };
    let NativeOutcome::Web(Ok(actual)) = outcome else {
        panic!("TEST_CODE expected full Web native batch: {outcome:?}");
    };
    let observed_at = DateTime::parse_from_rfc3339("2026-09-14T07:30:00.987654321Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let batch_id = format!("TEST_CODE_WEB_{}_{}", dimension + 1, provider.wire_name());
    let evidence = GeneralWebResearchBatchEvidence {
        provider,
        source: match provider {
            GeneralWebResearchProvider::Bocha => "bocha-general-web",
            GeneralWebResearchProvider::SerpApi => "serpapi-general-web",
            GeneralWebResearchProvider::Tavily => "tavily-general-web",
        }
        .to_owned(),
        query: QUERIES[dimension].to_owned(),
        observed_at,
        batch_id: batch_id.clone(),
        use_scope: ResearchUseScope::ResearchOnly,
    };
    let expected = if provider == GeneralWebResearchProvider::Bocha && dimension < 5 {
        GeneralWebResearchBatch::Available {
            evidence,
            records: vec![GeneralWebResearchRecord {
                title: format!("TEST_CODE 网页维度{}", dimension + 1),
                snippet: "TEST_CODE 研究摘要🌏  ".to_owned(),
                url: "https://example.invalid/TEST_CODE/web?q=一".to_owned(),
                publisher: "TEST_CODE 网页发布者".to_owned(),
                published_at_raw: Some("2026-09-14T15:29:00.123456789+08:00".to_owned()),
                published_at: Some(
                    DateTime::parse_from_rfc3339("2026-09-14T07:29:00.123456789Z")
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                ),
                evidence: GeneralWebResearchRecordEvidence {
                    provider,
                    observed_at,
                    batch_id,
                    item_id: format!("TEST_CODE_WEB_ITEM_{}", dimension + 1),
                    publication_quality: PublicationTimeQuality::ExactProviderTime,
                    use_scope: ResearchUseScope::ResearchOnly,
                },
            }],
        }
    } else {
        GeneralWebResearchBatch::VerifiedEmpty(evidence)
    };
    assert_eq!(actual, &expected);
}

#[tokio::test]
async fn single_user_local_prepare_completes_full_macro_and_reopens_before_models() {
    full_macro_scenario(FullMacroScenario::FullSuccess).await;
}

#[tokio::test]
async fn single_user_local_monotonic_expiry_after_five_confirmed_gateways_preserves_wall_and_reopens_empty_final() {
    full_macro_scenario(FullMacroScenario::MonotonicExpiry).await;
}

#[tokio::test]
async fn single_user_local_monotonic_expiry_finalize_commit_unknown_survives_database_reopen() {
    full_macro_scenario(FullMacroScenario::FinalizeCommitUnknown).await;
}

#[derive(Clone, Copy)]
enum FullMacroScenario {
    FullSuccess,
    MonotonicExpiry,
    FinalizeCommitUnknown,
}

struct MacroFinalizeCommitClock {
    inner: MacroClock,
    intent: String,
    reader: std::rc::Rc<RefCell<Option<rusqlite::Connection>>>,
    armed: Cell<bool>,
    locked_begin: RefCell<Option<(u64, u64, Vec<u8>)>>,
}

impl MacroFinalizeCommitClock {
    fn new(
        inner: MacroClock,
        intent: String,
        database: std::path::PathBuf,
        reader: std::rc::Rc<RefCell<Option<rusqlite::Connection>>>,
    ) -> Self {
        let connection = rusqlite::Connection::open_with_flags(
            database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .expect("TEST_CODE open owned Macro final commit reader");
        connection
            .busy_timeout(Duration::ZERO)
            .expect("TEST_CODE Macro final commit reader zero timeout");
        let journal_mode: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("TEST_CODE Macro final commit reader journal mode");
        assert_eq!(journal_mode.to_ascii_lowercase(), "delete");
        assert!(reader.borrow().is_none());
        *reader.borrow_mut() = Some(connection);
        Self {
            inner,
            intent,
            reader,
            armed: Cell::new(false),
            locked_begin: RefCell::new(None),
        }
    }

    fn locked_begin(&self) -> (u64, u64, Vec<u8>) {
        self.locked_begin
            .borrow()
            .clone()
            .expect("TEST_CODE real B must arm final commit fault")
    }
}

impl ConceptEffectClock for MacroFinalizeCommitClock {
    fn now(&self) -> UtcMicros {
        if !self.armed.get() {
            let mut locked_begin = None;
            {
                let reader = self.reader.borrow();
                let reader = reader
                    .as_ref()
                    .expect("TEST_CODE Macro final commit reader remains owned");
                let begin_without_final = reader
                    .query_row(
                        "SELECT EXISTS( \
                           SELECT 1 FROM chain_post_close_macro_finalize_begins \
                           WHERE intent_id=?1 \
                         ) AND NOT EXISTS( \
                           SELECT 1 FROM chain_post_close_macro_stage_finals \
                           WHERE intent_id=?1 \
                         )",
                        [self.intent.as_str()],
                        |row| row.get::<_, bool>(0),
                    )
                    .expect("TEST_CODE probe same-intent B without F");
                if begin_without_final {
                    reader
                        .execute_batch("BEGIN DEFERRED;")
                        .expect("TEST_CODE begin Macro final commit read lock");
                    locked_begin = Some(
                        reader
                            .query_row(
                                "SELECT r.head_version,b.run_version,b.bytes \
                                 FROM chain_post_close_runs r \
                                 JOIN chain_post_close_macro_finalize_begins b \
                                   ON b.intent_id=r.intent_id \
                                 WHERE r.intent_id=?1 AND NOT EXISTS( \
                                   SELECT 1 FROM chain_post_close_macro_stage_finals f \
                                   WHERE f.intent_id=r.intent_id \
                                 )",
                                [self.intent.as_str()],
                                |row| {
                                    Ok((
                                        row.get::<_, u64>(0)?,
                                        row.get::<_, u64>(1)?,
                                        row.get::<_, Vec<u8>>(2)?,
                                    ))
                                },
                            )
                            .expect("TEST_CODE lock committed B before F"),
                    );
                }
            }
            if let Some(locked_begin) = locked_begin {
                *self.locked_begin.borrow_mut() = Some(locked_begin);
                self.armed.set(true);
            }
        }
        self.inner.now()
    }
}

impl PositionObservationClock for MacroFinalizeCommitClock {
    fn cache_observation(&self) -> PositionCacheObservation {
        self.inner.cache_observation()
    }
}

impl DragonTigerObservationClock for MacroFinalizeCommitClock {
    fn dragon_tiger_request_observation(&self) -> DateTime<chrono::FixedOffset> {
        self.inner.dragon_tiger_request_observation()
    }
}

impl MacroObservationClock for MacroFinalizeCommitClock {
    fn macro_request_observation(&self) -> DateTime<chrono::FixedOffset> {
        self.inner.macro_request_observation()
    }
}

enum FullMacroClock {
    Regular(MacroClock),
    FinalizeCommitUnknown(MacroFinalizeCommitClock),
}

impl FullMacroClock {
    fn inner(&self) -> &MacroClock {
        match self {
            Self::Regular(clock) => clock,
            Self::FinalizeCommitUnknown(clock) => &clock.inner,
        }
    }

    fn finalize_commit_fault(&self) -> Option<&MacroFinalizeCommitClock> {
        match self {
            Self::Regular(_) => None,
            Self::FinalizeCommitUnknown(clock) => Some(clock),
        }
    }
}

impl ConceptEffectClock for FullMacroClock {
    fn now(&self) -> UtcMicros {
        match self {
            Self::Regular(clock) => clock.now(),
            Self::FinalizeCommitUnknown(clock) => clock.now(),
        }
    }
}

impl PositionObservationClock for FullMacroClock {
    fn cache_observation(&self) -> PositionCacheObservation {
        self.inner().cache_observation()
    }
}

impl DragonTigerObservationClock for FullMacroClock {
    fn dragon_tiger_request_observation(&self) -> DateTime<chrono::FixedOffset> {
        self.inner().dragon_tiger_request_observation()
    }
}

impl MacroObservationClock for FullMacroClock {
    fn macro_request_observation(&self) -> DateTime<chrono::FixedOffset> {
        self.inner().macro_request_observation()
    }
}

fn close_macro_finalize_fault_reader(
    reader: &std::rc::Rc<RefCell<Option<rusqlite::Connection>>>,
) -> rusqlite::Result<()> {
    let Some(connection) = reader.borrow_mut().take() else {
        return Ok(());
    };
    let rollback = if connection.is_autocommit() {
        Ok(())
    } else {
        connection.execute_batch("ROLLBACK;")
    };
    let close = connection.close().map_err(|(_, error)| error);
    rollback.and(close)
}

fn assert_empty_models_stop(error: &anyhow::Error, calls: &[Call]) {
    assert!(matches!(error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated { next: UnmigratedStage::ModelsSearchAndReport })),
        "TEST_CODE after five confirmed Gateways, monotonic expiry must complete Macro with empty context; actual={error:?}; calls={calls:?}");
    let failure = error.downcast_ref::<PreparationFailure>().unwrap();
    assert_eq!(failure.stage(), PreparationStage::ModelsSearchAndReport);
    assert_eq!(failure.completed_stages().last(), Some(&PreparationStage::Macro));
    assert_eq!(failure.macro_context().as_bytes(), b"");
    assert_eq!(failure.lhb_map()["TEST_CODE_600001"].to_bits(), 12.5_f64.to_bits());
    assert_eq!(failure.lhb_source().source(), Some("TEST_CODE_LHB_SOURCE_FIRST"));
}

async fn full_macro_scenario(scenario: FullMacroScenario) {
    let mut fixture = V2BusinessFixture::new();
    let mut parent_server = None;
    let finalize_fault_reader =
        std::rc::Rc::new(RefCell::new(None::<rusqlite::Connection>));
    let mut macro_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), async {
        let (parent_endpoint, server) = tokio::time::timeout(Duration::from_secs(5),
            spawn_macro_parent_listener(DRAGON_TIGER_RECORDS.as_bytes())).await
            .expect("TEST_CODE parent listener deadline");
        parent_server = Some(server);
        macro_server = Some(MacroFullLoopbackServer::bind().await);
        let server = macro_server.as_ref().unwrap();
        let parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await);
        let queries = parent_source.connected_board_queries().await.unwrap();
        let (stocks, config, intent, v9_head) = populate_completed_v9_parent(
            &mut fixture, &queries, "TEST_CODE_RUN_FULL_MACRO").await;
        fixture.chain_post_close().migrate_schema_v9_to_v10().unwrap();
        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(&intent, lease_request(
            "TEST_CODE_FULL_PARENT_OWNER", 68_300_000_000, 90_000_000_000, Some(v9_head))).unwrap();
        let parent_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-22T10:30:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-22T10:30:00+08:00").unwrap(),
            request_calls: Cell::new(0), cache_calls: Cell::new(0),
        };
        let mut io = local.dragon_tiger_preparation_io_v10(
            lease, &queries, &parent_clock, FixedClusterConfiguration::resolve(Some("2")),
            &parent_source).unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks.clone(), None, &mut io)
            .await.expect_err("TEST_CODE real parent stops at Macro");
        assert_partial_macro_stop(&stopped);
        drop(io);
        let parent_final = local.inspect_dragon_tiger(&intent).unwrap().final_bytes().unwrap().to_vec();
        let parent_head = local.inspect_run(&intent).unwrap().head_version();
        drop(local);
        let audit_count = fixture.count("data_acquisition_audit");
        let parent_network = parent_server.as_ref().unwrap().snapshot();
        let parent_memberships = parent_server.as_ref().unwrap().membership_snapshot();
        let database = fixture.database();
        fixture.chain_post_close().migrate_schema_v10_to_v11().unwrap();
        fixture.chain_post_close().migrate_schema_v11_to_v12().unwrap();
        let macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await, server.endpoint().to_owned());
        let started_at = micros("2026-09-14T15:30:00+08:00");
        let base_clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-14T15:30:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let clock = match scenario {
            FullMacroScenario::FinalizeCommitUnknown => {
                let journal_mode: String = fixture.connection()
                    .query_row("PRAGMA journal_mode", [], |row| row.get(0)).unwrap();
                assert_eq!(journal_mode.to_ascii_lowercase(), "delete");
                let writer_busy_ms: i64 = fixture.connection()
                    .query_row("PRAGMA busy_timeout", [], |row| row.get(0)).unwrap();
                assert_eq!(writer_busy_ms, 250,
                    "TEST_CODE owned writer must retain its bounded busy timeout");
                FullMacroClock::FinalizeCommitUnknown(MacroFinalizeCommitClock::new(
                    base_clock,
                    intent.as_str().to_owned(),
                    database.clone(),
                    finalize_fault_reader.clone(),
                ))
            }
            FullMacroScenario::FullSuccess | FullMacroScenario::MonotonicExpiry => {
                FullMacroClock::Regular(base_clock)
            }
        };
        let registered = [GeneralWebResearchProvider::SerpApi,
            GeneralWebResearchProvider::Bocha, GeneralWebResearchProvider::Tavily];
        let search_service = macro_search_service(&registered);
        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(&intent, macro_lease(
            "TEST_CODE_FULL_MACRO_OWNER", started_at, started_at + 60_000_000, parent_head)).unwrap();
        let original_generation = lease.generation();
        let mut io = local.macro_preparation_io_v12(
            lease, &queries, &clock, FixedClusterConfiguration::resolve(Some("2")),
            &parent_source, &macro_source, &search_service).unwrap();
        let mut reader = BusinessIntentStore::open(&database).unwrap();
        let mut read_local = reader.single_user_local_chain_post_close(&config).unwrap();
        let mut monotonic_confirmed = None;
        let stopped = {
            let prepared = prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks.clone(), None, &mut io);
            tokio::pin!(prepared);
            // Hold every real Gateway request before releasing any lane. Keep
            // polling this same prepare so an actual early return fails here.
            tokio::select! {
                result = &mut prepared => {
                    let calls = server.snapshot().calls.into_iter().map(|value| value.call).collect::<Vec<_>>();
                    panic!("TEST_CODE prepare returned before five Gateway requests were held; actual calls={calls:?}; actual result={result:?}");
                }
                _ = server.wait_for_gateway_count(5) => {},
            };
            let first_wave = server.snapshot();
            assert_eq!(first_wave.calls.len(), 5);
            for lane in [Lane::A, Lane::B, Lane::C, Lane::D, Lane::E] {
                assert_eq!(first_wave.calls.iter().filter(|call| call.call == Call::Gateway(lane)).count(), 1);
            }
            assert!(first_wave.calls.iter().all(|call| call.authorized && call.response.is_none()));

            let release_order = [Lane::B, Lane::E, Lane::D, Lane::A, Lane::C];
            let mut receipts = Vec::new();
            if matches!(scenario, FullMacroScenario::MonotonicExpiry
                | FullMacroScenario::FinalizeCommitUnknown) {
                server.release_all();
            }
            for (index, lane) in release_order.into_iter().enumerate() {
                if matches!(scenario, FullMacroScenario::FullSuccess) {
                    clock.inner().now.set(UtcMicros::try_new(started_at + (index as i64 + 1) * 100_000).unwrap());
                    server.release(lane);
                }
                loop {
                    let observed = if matches!(scenario, FullMacroScenario::FullSuccess)
                        || monotonic_confirmed.is_none()
                    {
                        tokio::select! {
                            result = &mut prepared => {
                                let stopped = result.expect_err("TEST_CODE model stage is guarded");
                                match scenario {
                                    FullMacroScenario::FullSuccess => {
                                        let calls = server.snapshot().calls.into_iter()
                                            .map(|value| value.call).collect::<Vec<_>>();
                                        assert_models_stop(
                                            &format!("per-lane persistence release_index={index} lane={lane:?}"),
                                            &stopped,
                                            &calls,
                                        );
                                    }
                                    FullMacroScenario::MonotonicExpiry
                                    | FullMacroScenario::FinalizeCommitUnknown => panic!(
                                        "TEST_CODE setup did not reach five confirmed Gateways; not target monotonic RED: {stopped:?}; calls={:?}",
                                        server.snapshot().calls),
                                }
                                panic!("TEST_CODE prepare returned before immediate per-lane persistence");
                            }
                            _ = tokio::time::sleep(Duration::from_millis(10)) => {}
                        }
                        let observed = read_local.inspect_macro(&intent).unwrap();
                        if matches!(scenario, FullMacroScenario::MonotonicExpiry)
                            && observed.attempts().iter().filter(|attempt| attempt.result_version().is_some()).count() != 5
                        {
                            continue;
                        }
                        Some(observed)
                    } else {
                        // The first five-confirmed snapshot is immutable. Do not
                        // poll prepare or await again while checking its lanes.
                        None
                    };
                    let recovery = match scenario {
                        FullMacroScenario::FullSuccess => observed.as_ref().unwrap(),
                        FullMacroScenario::MonotonicExpiry
                        | FullMacroScenario::FinalizeCommitUnknown => {
                            if let Some(observed) = observed {
                                monotonic_confirmed = Some(observed);
                            }
                            monotonic_confirmed.as_ref().unwrap()
                        }
                    };
                    let wire = server.snapshot();
                    let captured = wire.calls.iter().find(|call| call.call == Call::Gateway(lane)).unwrap();
                    let Some(attempt) = recovery.attempts().iter()
                        .find(|attempt| attempt.request_bytes() == captured.request.as_slice()) else {
                            panic!("TEST_CODE wire request must already have a confirmed begin");
                        };
                    if attempt.result_version().is_none() { continue; }
                    assert_eq!(attempt.response_bytes(), captured.response.as_deref());
                    let query = QueryKey::Gateway(u8::try_from(lane.index()+1).unwrap());
                    assert_eq!(attempt.query_key(), query);
                    assert_eq!(attempt.continuation(), Some(MacroContinuation::Terminal));
                    match scenario {
                        FullMacroScenario::FullSuccess => {
                            assert_eq!(recovery.attempts().iter().filter(|attempt| attempt.result_version().is_some()).count(), index + 1);
                            for pending_lane in &release_order[index + 1..] {
                                assert!(wire.calls.iter().find(|call| call.call == Call::Gateway(*pending_lane)).unwrap().response.is_none());
                            }
                        }
                        FullMacroScenario::MonotonicExpiry
                        | FullMacroScenario::FinalizeCommitUnknown => assert_eq!(recovery.attempts().len(), 5),
                    }
                    let query_terminal = recovery.query_terminal(query).expect("TEST_CODE result+native+QueryTerminal are one commit");
                    assert_eq!(query_terminal.query_key(), query);
                    assert_eq!(query_terminal.version(), attempt.result_version().unwrap()+1);
                    match scenario {
                        FullMacroScenario::FullSuccess => assert_eq!(query_terminal.recorded_at(), started_at+(index as i64+1)*100_000),
                        FullMacroScenario::MonotonicExpiry
                        | FullMacroScenario::FinalizeCommitUnknown => assert_eq!(query_terminal.recorded_at(), started_at),
                    }
                    assert!(query_terminal.was_called());
                    assert!(!query_terminal.native_bytes().is_empty());
                    let receipt = query_terminal.audit_receipt().expect("TEST_CODE all five native gateways have their unique atomic BR159");
                    assert!(!receipts.iter().any(|earlier: &crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt| earlier.audit_id == receipt.audit_id));
                    receipts.push(receipt.clone());
                    if lane == Lane::E { assert_economic_native(query_terminal.native()); }
                    if let Some(provider) = lane.news() {
                        let terminal = recovery.global_news(provider).expect("TEST_CODE atomic native/terminal");
                        assert!(terminal.is_complete());
                        let batch = terminal.batch().expect("TEST_CODE native news batch");
                        assert_eq!(batch.evidence().provider, provider.provider_id());
                        assert_eq!(batch.evidence().source, provider.source());
                        assert_eq!(batch.evidence().source_at.as_deref(), Some("2026-09-14T15:30:00.123456789+08:00"));
                        assert_eq!(batch.records().len(), 1);
                        let record = &batch.records()[0];
                        assert_eq!(record.title, format!("TEST_CODE {} 财经🌏", provider.wire_name()));
                        assert_eq!(record.summary.as_deref(), Some("TEST_CODE 摘要  "));
                        assert_eq!(record.content, None);
                        assert_eq!(record.item_id, format!("TEST_CODE_{}_一", provider.wire_name()));
                        assert_eq!(record.publisher, "TEST_CODE 发布者");
                        assert_eq!(record.canonical_url, "https://example.invalid/TEST_CODE/一");
                        assert_eq!(record.instruments, ["TEST_CODE_600001.SH"]);
                        assert_eq!(record.topics, ["TEST_CODE 政策"]);
                        assert_eq!(record.language, "zh-CN");
                        assert_eq!(record.observed_at.timestamp_subsec_nanos(), 987654321);
                        assert_eq!(record.evidence.provider(), provider.provider_id());
                        assert_eq!(record.evidence.source_at(), Some("2026-09-14T15:30:00.123456789+08:00"));
                        assert_eq!(record.evidence.observed_at(), "2026-09-14T15:30:00.987654321+08:00");
                        assert_eq!(record.evidence.batch_id(), format!("TEST_CODE_FULL_{}", provider.wire_name()));
                        assert_eq!(record.published_at.timestamp_subsec_nanos(), 123456789);
                        let receipt = terminal.audit_receipt().expect("TEST_CODE same-snapshot unique BR159");
                        assert_eq!(Some(receipt), query_terminal.audit_receipt());
                    }
                    break;
                }
            }
            assert_eq!(receipts.len(), 5);
            if matches!(scenario, FullMacroScenario::MonotonicExpiry
                | FullMacroScenario::FinalizeCommitUnknown) {
                let confirmed = monotonic_confirmed.as_ref().expect("TEST_CODE all-confirmed snapshot");
                assert_eq!(clock.inner().now.get().get(), started_at);
                assert_eq!(confirmed.attempts().len(), 5);
                assert!(confirmed.attempts().iter().all(|attempt| attempt.result_version().is_some()));
                assert!(confirmed.readiness_episodes().is_empty());
                assert!(!confirmed.has_unconfirmed_effect());
                assert!(confirmed.stage_final().is_none());
                assert!(confirmed.pending_source_identities().is_empty());
                let wire = server.snapshot();
                assert_eq!(wire.calls.len(), 5);
                assert!(wire.calls.iter().all(|call| matches!(call.call, Call::Gateway(_))
                    && call.authorized && call.response.is_some()));
                assert_eq!(wire.controls, 0);
                assert!(wire.unexpected.is_empty());
            }
            drop(read_local);
            reader.connection.close().unwrap();
            let stopped = match scenario {
                FullMacroScenario::FullSuccess => {
                    // Advance the controlled wall clock alongside the existing monotonic
                    // clock while the real runner consumes 200ms + six 300ms waits.
                    let paced_at = tokio::time::Instant::now();
                    loop {
                        tokio::select! {
                            result = &mut prepared => break result.expect_err("TEST_CODE model stage is guarded"),
                            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                                clock.inner().now.set(UtcMicros::try_new(started_at + 500_000
                                    + i64::try_from(paced_at.elapsed().as_micros()).unwrap()).unwrap());
                            }
                        }
                    }
                }
                FullMacroScenario::MonotonicExpiry
                | FullMacroScenario::FinalizeCommitUnknown => {
                    // Wall remains S. Only the runner's existing monotonic limit
                    // can end its Gateway pace; the outer watchdog is not proof.
                    let stopped = prepared.await.expect_err("TEST_CODE model stage is guarded");
                    assert_eq!(clock.inner().now.get().get(), started_at);
                    stopped
                }
            };
            stopped
        };
        let wire = server.snapshot();
        let calls = wire.calls.iter().map(|call| call.call.clone()).collect::<Vec<_>>();
        match scenario {
            FullMacroScenario::FullSuccess => {
                assert_models_stop(
                    "initial full-success completion",
                    &stopped,
                    &calls,
                );
                drop(io);
                let recovery = local.inspect_macro(&intent).unwrap();
                assert!(recovery.is_complete(),
                    "TEST_CODE full final must be sealed; frozen independent expected output:\n{EXPECTED_MACRO}");
                assert!(!recovery.has_unconfirmed_effect());
                assert!(recovery.pending_source_identities().is_empty());
                assert!(recovery.pending_research_queries().is_empty());
                assert_eq!(recovery.plan().research_queries(), QUERIES);
                assert_eq!(recovery.plan().research_providers(), registered);
                assert_eq!(recovery.parent_final_bytes(), parent_final);
                assert_eq!(recovery.plan().deadline_at().get(), started_at + 15_000_000);
                let final_ = recovery.stage_final().expect("TEST_CODE full StageFinal after separate FinalizeBegin");
                assert_eq!(final_.kind(), crate::push_foundation::intent_store::chain_post_close::macro_native::FinalKind::Complete);
                let begin = recovery.finalize_begin().unwrap();
                assert_eq!(begin.expiry(), &ExpiryBasis::None);
                assert_eq!(begin.kind(), final_.kind());
                assert_eq!(begin.bytes(), final_.finalize_begin_bytes());
                assert!(begin.pending().is_empty());
                assert_eq!(final_.output_bytes(), EXPECTED_MACRO.as_bytes());
                assert_eq!(final_.version(), final_.finalize_begin_version()+1);
                assert_eq!(final_.plan_version(), recovery.plan_version());
                assert_eq!(final_.started_at(), started_at);
                assert_eq!(final_.deadline_at(), started_at+15_000_000);
                let stage_final_bytes = final_.bytes().to_vec();
                let finalize_begin_bytes = final_.finalize_begin_bytes().to_vec();
                let plan_bytes = recovery.plan_bytes().to_vec();
                let persisted_wire = recovery.attempts().iter().map(|attempt|
                    (attempt.request_bytes().to_vec(), attempt.response_bytes().unwrap().to_vec()))
                    .collect::<Vec<_>>();
                assert_eq!(persisted_wire.len(), 18); // five Gateway + (2 * five selected) + three exhausted
                let expected_web = [
                    (0, GeneralWebResearchProvider::SerpApi), (0, GeneralWebResearchProvider::Bocha),
                    (1, GeneralWebResearchProvider::SerpApi), (1, GeneralWebResearchProvider::Bocha),
                    (2, GeneralWebResearchProvider::SerpApi), (2, GeneralWebResearchProvider::Bocha),
                    (3, GeneralWebResearchProvider::SerpApi), (3, GeneralWebResearchProvider::Bocha),
                    (4, GeneralWebResearchProvider::SerpApi), (4, GeneralWebResearchProvider::Bocha),
                    (5, GeneralWebResearchProvider::SerpApi), (5, GeneralWebResearchProvider::Bocha),
                    (5, GeneralWebResearchProvider::Tavily),
                ];
                let actual_web = calls.iter().filter_map(|call| match call {
                    Call::Web { dimension, provider } => Some((*dimension, *provider)), _ => None,
                }).collect::<Vec<_>>();
                assert_eq!(actual_web, expected_web);
                assert_economic_native(recovery.query_terminal(QueryKey::Gateway(5)).unwrap().native());
                let mut persisted_native = (1..=5).map(|ordinal| {
                    let key = QueryKey::Gateway(ordinal);
                    (key, recovery.query_terminal(key).unwrap().native_bytes().to_vec())
                }).collect::<Vec<_>>();
                for (dimension, provider) in expected_web {
                    let candidate = registered.iter().position(|actual| *actual == provider).unwrap() as u32+1;
                    let key = QueryKey::Web { dimension:dimension as u8+1, candidate };
                    let terminal = recovery.query_terminal(key).expect("TEST_CODE every Web candidate has a native terminal");
                    assert_web_native(terminal.native(), dimension, provider);
                    assert!(terminal.was_called());
                    assert!(terminal.audit_receipt().is_none(), "TEST_CODE Web must not acquire financial BR159");
                    persisted_native.push((key, terminal.native_bytes().to_vec()));
                }
                assert!(wire.calls.iter().all(|call| call.authorized && call.response.is_some()));
                assert_eq!(wire.controls, 0);
                assert!(wire.unexpected.is_empty());
                let first_head = local.inspect_run(&intent).unwrap().head_version();
                drop(local);
                assert_eq!(fixture.count("data_acquisition_audit"), audit_count + 5);
                drop(queries);
                drop(parent_source);
                drop(macro_source);
                fixture.reopen();
                server.release_all(); // A mistaken replay returns, so counts catch it.
                let changed_source = GrpcSource::from_macro_loopback_test_client(
                    server.connect().await, server.endpoint().to_owned());
                let parent_before_changed_connect = parent_server
                    .as_ref()
                    .unwrap()
                    .snapshot_with_tcp_for_test();
                let parent_memberships_before_changed_connect = parent_server
                    .as_ref()
                    .unwrap()
                    .membership_snapshot();
                let changed_parent = GrpcSource::from_board_loopback_test_client(
                    connect_parent_instance(&parent_endpoint).await);
                let expected_changed_parent_tcp = parent_before_changed_connect.0 + 1;
                tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let observed = parent_server
                            .as_ref()
                            .unwrap()
                            .snapshot_with_tcp_for_test();
                        assert_eq!(
                            observed.1, parent_before_changed_connect.1,
                            "TEST_CODE changed parent connect must not issue an RPC",
                        );
                        assert_eq!(
                            parent_server.as_ref().unwrap().membership_snapshot(),
                            parent_memberships_before_changed_connect,
                            "TEST_CODE changed parent connect must not issue membership RPC",
                        );
                        if observed.0 == expected_changed_parent_tcp {
                            break;
                        }
                        assert_eq!(
                            observed.0, parent_before_changed_connect.0,
                            "TEST_CODE changed parent connect must add exactly one TCP accept",
                        );
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .expect("TEST_CODE changed parent TCP accept deadline");
                let changed_queries = changed_parent.connected_board_queries().await.unwrap();
                let changed_clock = MacroClock {
                    now: Cell::new(UtcMicros::try_new(started_at + 86_400_000_000).unwrap()),
                    observation: DateTime::parse_from_rfc3339("2026-09-15T15:30:00+08:00").unwrap(),
                    observation_calls: Cell::new(0),
                };
                let changed_service = macro_search_service(&[
                    GeneralWebResearchProvider::Tavily, GeneralWebResearchProvider::Bocha,
                    GeneralWebResearchProvider::SerpApi,
                ]);
                let mut local = fixture.store.as_mut().unwrap()
                    .single_user_local_chain_post_close(&config).unwrap();
                let lease = local.resume_run(&intent, macro_lease(
                    "TEST_CODE_FULL_REOPEN_OWNER", started_at + 86_400_000_000,
                    started_at + 86_460_000_000, first_head)).unwrap();
                let mut io = local.macro_preparation_io_v12(
                    lease, &changed_queries, &changed_clock, FixedClusterConfiguration::resolve(Some("2")),
                    &changed_parent, &changed_source, &changed_service).unwrap();
                let stopped = prepare_chain_analysis_with_io(
                    NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks, None, &mut io)
                    .await.expect_err("TEST_CODE reopen also stops before models");
                let reopened_calls = server.snapshot().calls.into_iter()
                    .map(|value| value.call).collect::<Vec<_>>();
                assert_models_stop("reopened full-success recovery", &stopped, &reopened_calls);
                drop(io);
                let reopened = local.inspect_macro(&intent).unwrap();
                assert!(reopened.is_complete());
                assert_eq!(reopened.plan_bytes(), plan_bytes);
                let final_ = reopened.stage_final().unwrap();
                assert_eq!(final_.output_bytes(), EXPECTED_MACRO.as_bytes());
                assert_eq!(final_.bytes(), stage_final_bytes);
                assert_eq!(final_.finalize_begin_bytes(), finalize_begin_bytes);
                for (key, bytes) in persisted_native {
                    assert_eq!(reopened.query_terminal(key).unwrap().native_bytes(), bytes);
                }
                assert_economic_native(reopened.query_terminal(QueryKey::Gateway(5)).unwrap().native());
                for (dimension, provider) in expected_web {
                    let candidate = registered.iter().position(|actual| *actual == provider).unwrap() as u32+1;
                    assert_web_native(reopened.query_terminal(QueryKey::Web { dimension:dimension as u8+1,candidate }).unwrap().native(), dimension, provider);
                }
                assert_eq!(reopened.attempts().iter().map(|attempt|
                    (attempt.request_bytes().to_vec(), attempt.response_bytes().unwrap().to_vec()))
                    .collect::<Vec<_>>(), persisted_wire);
                assert_eq!(changed_clock.observation_calls.get(), 0);
                drop(local);
                assert_eq!(fixture.count("data_acquisition_audit"), audit_count + 5);
                assert_eq!(server.snapshot(), wire);
                assert_eq!(parent_server.as_ref().unwrap().snapshot(), parent_network);
                assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), parent_memberships);
                drop(changed_queries);
                drop(changed_parent);
                drop(changed_source);
                let store = fixture
                    .store
                    .take()
                    .expect("TEST_CODE full fact matrix owns final store");
                assert!(store.connection.is_autocommit());
                store
                    .connection
                    .close()
                    .expect("TEST_CODE full fact matrix closes seed database");
                super::corruption_tests::assert_v12_full_fact_corruption_matrix(
                    &database,
                    &config,
                    &intent,
                    server,
                    parent_server.as_ref().unwrap(),
                    EXPECTED_MACRO.as_bytes(),
                )
                .await;
            }
            FullMacroScenario::MonotonicExpiry => {
                const PENDING_KEYS: [QueryKey; 18] = [
                    QueryKey::Web { dimension: 1, candidate: 1 },
                    QueryKey::Web { dimension: 1, candidate: 2 },
                    QueryKey::Web { dimension: 1, candidate: 3 },
                    QueryKey::Web { dimension: 2, candidate: 1 },
                    QueryKey::Web { dimension: 2, candidate: 2 },
                    QueryKey::Web { dimension: 2, candidate: 3 },
                    QueryKey::Web { dimension: 3, candidate: 1 },
                    QueryKey::Web { dimension: 3, candidate: 2 },
                    QueryKey::Web { dimension: 3, candidate: 3 },
                    QueryKey::Web { dimension: 4, candidate: 1 },
                    QueryKey::Web { dimension: 4, candidate: 2 },
                    QueryKey::Web { dimension: 4, candidate: 3 },
                    QueryKey::Web { dimension: 5, candidate: 1 },
                    QueryKey::Web { dimension: 5, candidate: 2 },
                    QueryKey::Web { dimension: 5, candidate: 3 },
                    QueryKey::Web { dimension: 6, candidate: 1 },
                    QueryKey::Web { dimension: 6, candidate: 2 },
                    QueryKey::Web { dimension: 6, candidate: 3 },
                ];
                const PENDING_QUERIES: [&str; 6] = [
                    "2026年09月14日A股 大盘 股市 最新动态",
                    "2026年09月14日国际财经 地缘政治 最新消息",
                    "2026年09月14日美股 美联储 大宗商品 今日",
                    "2026年09月14日中国 央行 财政 产业政策 重要新闻",
                    "2026年09月14日高盛 摩根 大摩 美银 JPMorgan 中国A股 市场观点 研报",
                    "2026年09月14日证券时报 第一财经 21世纪经济报道 重要财经",
                ];
                assert_empty_models_stop(&stopped, &calls);
                assert_eq!(clock.inner().now.get().get(), started_at);
                assert_eq!(wire.calls.len(), 5);
                assert!(wire.calls.iter().all(|call| matches!(call.call, Call::Gateway(_))
                    && call.authorized && call.response.is_some()));
                assert_eq!(wire.controls, 0);
                assert!(wire.unexpected.is_empty());
                drop(io);
                let confirmed = monotonic_confirmed.as_ref().unwrap();
                let recovery = local.inspect_macro(&intent).unwrap();
                assert!(recovery.is_complete());
                assert!(!recovery.has_unconfirmed_effect());
                assert!(recovery.pending_source_identities().is_empty());
                assert_eq!(recovery.pending_research_queries(), PENDING_QUERIES);
                assert_eq!(recovery.plan().research_queries(), PENDING_QUERIES);
                assert_eq!(recovery.plan().research_providers(), registered);
                assert_eq!(recovery.parent_final_bytes(), parent_final);
                assert_eq!(recovery.plan_bytes(), confirmed.plan_bytes());
                let final_ = recovery.stage_final().expect("TEST_CODE mono-only expiry must seal a real empty final");
                assert_eq!(final_.kind(), crate::push_foundation::intent_store::chain_post_close::macro_native::FinalKind::BudgetExpired);
                assert_eq!(final_.output_bytes(), b"");
                assert_eq!(final_.version(), final_.finalize_begin_version() + 1);
                assert_eq!(final_.plan_version(), recovery.plan_version());
                assert_eq!(final_.started_at(), started_at);
                assert_eq!(final_.deadline_at(), started_at + 15_000_000);
                let begin = recovery.finalize_begin().unwrap();
                assert_eq!(begin.kind(), final_.kind());
                assert_eq!(begin.version(), final_.finalize_begin_version());
                assert_eq!(begin.bytes(), final_.finalize_begin_bytes());
                assert_eq!(begin.recorded_at(), started_at);
                assert_eq!(final_.recorded_at(), started_at);
                assert_eq!(begin.owner(), "TEST_CODE_FULL_MACRO_OWNER");
                assert_eq!(final_.owner(), "TEST_CODE_FULL_MACRO_OWNER");
                assert_eq!(begin.generation(), original_generation);
                assert_eq!(final_.generation(), original_generation);
                match begin.expiry() {
                    ExpiryBasis::MonotonicRemaining { opened_wall_at, elapsed_us } => {
                        assert_eq!(*opened_wall_at, started_at);
                        assert!(*elapsed_us >= 15_000_000);
                    }
                    actual => panic!("TEST_CODE fixed-wall budget requires monotonic evidence: {actual:?}"),
                }
                assert_eq!(begin.pending(), PENDING_KEYS);
                assert!(begin.pending().iter().all(|key| matches!(key, QueryKey::Web { .. })));
                assert_eq!(serde_json::from_slice::<serde_json::Value>(begin.bytes()).unwrap()["version"], 3);
                assert_eq!(serde_json::from_slice::<serde_json::Value>(final_.bytes()).unwrap()["version"], 2);
                let final_bytes = final_.bytes().to_vec();
                let begin_bytes = final_.finalize_begin_bytes().to_vec();
                let plan_bytes = recovery.plan_bytes().to_vec();
                let persisted_wire = confirmed.attempts().iter().map(|attempt|
                    (attempt.request_bytes().to_vec(), attempt.response_bytes().unwrap().to_vec()))
                    .collect::<Vec<_>>();
                assert_eq!(persisted_wire.len(), 5);
                assert_eq!(recovery.attempts().iter().map(|attempt|
                    (attempt.request_bytes().to_vec(), attempt.response_bytes().unwrap().to_vec()))
                    .collect::<Vec<_>>(), persisted_wire);
                let persisted_native = (1..=5).map(|ordinal| {
                    let key = QueryKey::Gateway(ordinal);
                    let original = confirmed.query_terminal(key).unwrap();
                    let final_terminal = recovery.query_terminal(key).unwrap();
                    assert_eq!(final_terminal.native_bytes(), original.native_bytes());
                    assert_eq!(final_terminal.audit_receipt(), original.audit_receipt());
                    (key, original.native_bytes().to_vec())
                }).collect::<Vec<_>>();
                assert_economic_native(recovery.query_terminal(QueryKey::Gateway(5)).unwrap().native());
                let first_run = local.inspect_run(&intent).unwrap();
                let first_head = first_run.head_version();
                let first_generation = first_run.lease_generation();
                drop(local);
                assert_eq!(fixture.count("data_acquisition_audit"), audit_count + 5);
                fixture.reopen();
                let mut local = fixture.store.as_mut().unwrap()
                    .single_user_local_chain_post_close(&config).unwrap();
                let reopened = local.inspect_macro(&intent).unwrap();
                assert_eq!(reopened.plan_bytes(), plan_bytes);
                assert_eq!(reopened.stage_final().unwrap().bytes(), final_bytes);
                assert_eq!(reopened.stage_final().unwrap().finalize_begin_bytes(), begin_bytes);
                assert_eq!(reopened.finalize_begin().unwrap().bytes(), begin_bytes);
                assert_eq!(reopened.finalize_begin().unwrap().pending(), PENDING_KEYS);
                for (key, bytes) in &persisted_native {
                    assert_eq!(reopened.query_terminal(*key).unwrap().native_bytes(), bytes);
                }
                assert_eq!(reopened.attempts().iter().map(|attempt|
                    (attempt.request_bytes().to_vec(), attempt.response_bytes().unwrap().to_vec()))
                    .collect::<Vec<_>>(), persisted_wire);
                for owner in ["TEST_CODE_FULL_MACRO_OWNER", "TEST_CODE_MONO_REOPEN_OWNER"] {
                    match local.resume_run(&intent, macro_lease(owner, started_at,
                        started_at + 120_000_000, first_head)) {
                        Err(ChainPostCloseError::LeaseHeld { intent_id }) => assert_eq!(intent_id, intent.as_str()),
                        Err(error) => panic!("TEST_CODE fixed-wall reopen must be LeaseHeld: {error:?}"),
                        Ok(_) => panic!("TEST_CODE monotonic expiry must not expire the wall lease"),
                    }
                    let unchanged = local.inspect_run(&intent).unwrap();
                    assert_eq!(unchanged.head_version(), first_head);
                    assert_eq!(unchanged.lease_generation(), first_generation);
                }
                // A separate recovery scenario reaches the real old lease expiry.
                // The persisted plan/final keep their original S/D and owner facts.
                let reopened_clock = MacroClock {
                    now: Cell::new(UtcMicros::try_new(started_at + 60_000_000).unwrap()),
                    observation: DateTime::parse_from_rfc3339("2026-09-15T15:30:00+08:00").unwrap(),
                    observation_calls: Cell::new(0),
                };
                let changed_service = macro_search_service(&[]);
                let lease = local.resume_run(&intent, macro_lease("TEST_CODE_MONO_REOPEN_OWNER",
                    started_at + 60_000_000, started_at + 120_000_000, first_head)).unwrap();
                assert_eq!(lease.head_version(), first_head + 1);
                assert_eq!(lease.generation(), first_generation + 1);
                let mut io = local.macro_preparation_io_v12(
                    lease, &queries, &reopened_clock, FixedClusterConfiguration::resolve(Some("2")),
                    &parent_source, &macro_source, &changed_service).unwrap();
                server.release_all();
                let stopped = prepare_chain_analysis_with_io(
                    NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks, None, &mut io)
                    .await.expect_err("TEST_CODE reopened empty final still stops before models");
                assert_empty_models_stop(&stopped, &calls);
                drop(io);
                let reopened = local.inspect_macro(&intent).unwrap();
                assert!(reopened.is_complete());
                assert!(!reopened.has_unconfirmed_effect());
                assert_eq!(reopened.plan_bytes(), plan_bytes);
                assert!(reopened.pending_source_identities().is_empty());
                assert_eq!(reopened.pending_research_queries(), PENDING_QUERIES);
                let final_ = reopened.stage_final().unwrap();
                assert_eq!(final_.kind(), crate::push_foundation::intent_store::chain_post_close::macro_native::FinalKind::BudgetExpired);
                assert_eq!(final_.output_bytes(), b"");
                assert_eq!(final_.started_at(), started_at);
                assert_eq!(final_.deadline_at(), started_at + 15_000_000);
                assert_eq!(final_.bytes(), final_bytes);
                assert_eq!(final_.finalize_begin_bytes(), begin_bytes);
                let begin = reopened.finalize_begin().unwrap();
                assert_eq!(begin.bytes(), begin_bytes);
                assert_eq!(begin.pending(), PENDING_KEYS);
                assert_eq!(begin.recorded_at(), started_at);
                assert_eq!(final_.recorded_at(), started_at);
                assert_eq!(begin.owner(), "TEST_CODE_FULL_MACRO_OWNER");
                assert_eq!(final_.owner(), "TEST_CODE_FULL_MACRO_OWNER");
                assert_eq!(begin.generation(), original_generation);
                assert_eq!(final_.generation(), original_generation);
                for (key, bytes) in persisted_native {
                    assert_eq!(reopened.query_terminal(key).unwrap().native_bytes(), bytes);
                    assert_eq!(reopened.query_terminal(key).unwrap().audit_receipt(),
                        confirmed.query_terminal(key).unwrap().audit_receipt());
                }
                assert_eq!(reopened.attempts().iter().map(|attempt|
                    (attempt.request_bytes().to_vec(), attempt.response_bytes().unwrap().to_vec()))
                    .collect::<Vec<_>>(), persisted_wire);
                assert_eq!(local.inspect_run(&intent).unwrap().head_version(), first_head + 1);
                assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), first_generation + 1);
                assert_eq!(reopened_clock.observation_calls.get(), 0);
                drop(local);
                assert_eq!(fixture.count("data_acquisition_audit"), audit_count + 5);
                assert_eq!(server.snapshot(), wire);
                assert_eq!(parent_server.as_ref().unwrap().snapshot(), parent_network);
                assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), parent_memberships);
            }
            FullMacroScenario::FinalizeCommitUnknown => {
                const PENDING_KEYS: [QueryKey; 18] = [
                    QueryKey::Web { dimension: 1, candidate: 1 },
                    QueryKey::Web { dimension: 1, candidate: 2 },
                    QueryKey::Web { dimension: 1, candidate: 3 },
                    QueryKey::Web { dimension: 2, candidate: 1 },
                    QueryKey::Web { dimension: 2, candidate: 2 },
                    QueryKey::Web { dimension: 2, candidate: 3 },
                    QueryKey::Web { dimension: 3, candidate: 1 },
                    QueryKey::Web { dimension: 3, candidate: 2 },
                    QueryKey::Web { dimension: 3, candidate: 3 },
                    QueryKey::Web { dimension: 4, candidate: 1 },
                    QueryKey::Web { dimension: 4, candidate: 2 },
                    QueryKey::Web { dimension: 4, candidate: 3 },
                    QueryKey::Web { dimension: 5, candidate: 1 },
                    QueryKey::Web { dimension: 5, candidate: 2 },
                    QueryKey::Web { dimension: 5, candidate: 3 },
                    QueryKey::Web { dimension: 6, candidate: 1 },
                    QueryKey::Web { dimension: 6, candidate: 2 },
                    QueryKey::Web { dimension: 6, candidate: 3 },
                ];
                const PENDING_QUERIES: [&str; 6] = [
                    "2026年09月14日A股 大盘 股市 最新动态",
                    "2026年09月14日国际财经 地缘政治 最新消息",
                    "2026年09月14日美股 美联储 大宗商品 今日",
                    "2026年09月14日中国 央行 财政 产业政策 重要新闻",
                    "2026年09月14日高盛 摩根 大摩 美银 JPMorgan 中国A股 市场观点 研报",
                    "2026年09月14日证券时报 第一财经 21世纪经济报道 重要财经",
                ];
                assert!(matches!(stopped.downcast_ref::<PreparationStop>(),
                    Some(PreparationStop::ResultUnconfirmed { intent_id })
                        if intent_id == intent.as_str()),
                    "TEST_CODE F COMMIT failure must be ResultUnconfirmed: {stopped:?}");
                assert!(matches!(stopped.downcast_ref::<ChainPostCloseError>(),
                    Some(ChainPostCloseError::StorageFailed {
                        operation: "Macro StageFinal commit",
                    })),
                    "TEST_CODE F COMMIT must retain the concrete storage failure: {stopped:?}");
                assert!(!matches!(stopped.downcast_ref::<PreparationStop>(),
                    Some(PreparationStop::StageNotMigrated { .. })));
                assert!(stopped.downcast_ref::<
                    crate::search_service::macro_news::runner::BudgetExpired>().is_none());
                let failure = stopped.downcast_ref::<PreparationFailure>().unwrap();
                assert_eq!(failure.stage(), PreparationStage::Macro);
                assert_eq!(failure.completed_stages().last(),
                    Some(&PreparationStage::DragonTiger));
                assert!(!failure.completed_stages().contains(&PreparationStage::Macro));
                assert!(!failure.completed_stages()
                    .contains(&PreparationStage::ModelsSearchAndReport));
                let fault = clock.finalize_commit_fault()
                    .expect("TEST_CODE final commit scenario clock");
                assert!(fault.armed.get(),
                    "TEST_CODE clock must observe committed B before blocking F");
                let (locked_head, begin_version, begin_bytes) = fault.locked_begin();
                assert_eq!(locked_head, begin_version);
                assert_eq!(clock.inner().now.get().get(), started_at);
                assert_eq!(wire.calls.len(), 5);
                assert!(wire.calls.iter().all(|call| matches!(call.call, Call::Gateway(_))
                    && call.authorized && call.response.is_some()));
                assert_eq!(wire.controls, 0);
                assert!(wire.unexpected.is_empty());
                drop(io);

                let confirmed = monotonic_confirmed.as_ref().unwrap();
                let plan_bytes = confirmed.plan_bytes().to_vec();
                let persisted_gateway = (1..=5).map(|ordinal| {
                    let key = QueryKey::Gateway(ordinal);
                    let attempt = confirmed.attempts().iter()
                        .find(|attempt| attempt.query_key() == key).unwrap();
                    let terminal = confirmed.query_terminal(key).unwrap();
                    (
                        key,
                        attempt.request_bytes().to_vec(),
                        attempt.response_bytes().unwrap().to_vec(),
                        terminal.native_bytes().to_vec(),
                        terminal.audit_receipt().unwrap().clone(),
                    )
                }).collect::<Vec<_>>();
                let mut audit_ids = std::collections::BTreeSet::new();
                assert!(persisted_gateway.iter()
                    .all(|gateway| audit_ids.insert(gateway.4.audit_id)));
                assert_eq!(audit_ids.len(), 5);
                drop(local);
                close_macro_finalize_fault_reader(&finalize_fault_reader)
                    .expect("TEST_CODE release and close Macro final commit reader");
                fixture.reopen();
                assert_eq!(fixture.count("data_acquisition_audit"), audit_count + 5);

                let mut local = fixture.store.as_mut().unwrap()
                    .single_user_local_chain_post_close(&config).unwrap();
                let reopened_run = local.inspect_run(&intent).unwrap();
                assert_eq!(reopened_run.head_version(), begin_version);
                assert_eq!(reopened_run.lease_generation(), original_generation);
                let first_generation = reopened_run.lease_generation();
                let reopened = local.inspect_macro(&intent).unwrap();
                assert!(!reopened.is_complete());
                assert!(reopened.has_unconfirmed_effect());
                assert!(reopened.stage_final().is_none());
                assert_eq!(reopened.plan_bytes(), plan_bytes);
                assert_eq!(reopened.plan().started_at().get(), started_at);
                assert_eq!(reopened.plan().deadline_at().get(), started_at + 15_000_000);
                assert_eq!(reopened.plan().research_queries(), PENDING_QUERIES);
                assert_eq!(reopened.plan().research_providers(), registered);
                assert_eq!(reopened.parent_final_bytes(), parent_final);
                assert!(reopened.pending_source_identities().is_empty());
                assert_eq!(reopened.pending_research_queries(), PENDING_QUERIES);
                let begin = reopened.finalize_begin()
                    .expect("TEST_CODE committed B survives true database reopen");
                assert_eq!(begin.version(), begin_version);
                assert_eq!(begin.bytes(), begin_bytes);
                assert_eq!(begin.kind(), crate::push_foundation::intent_store::chain_post_close::macro_native::FinalKind::BudgetExpired);
                assert_eq!(begin.recorded_at(), started_at);
                assert_eq!(begin.owner(), "TEST_CODE_FULL_MACRO_OWNER");
                assert_eq!(begin.generation(), original_generation);
                match begin.expiry() {
                    ExpiryBasis::MonotonicRemaining { opened_wall_at, elapsed_us } => {
                        assert_eq!(*opened_wall_at, started_at);
                        assert!(*elapsed_us >= 15_000_000);
                    }
                    actual => panic!("TEST_CODE B requires monotonic evidence: {actual:?}"),
                }
                assert_eq!(begin.pending(), PENDING_KEYS);
                assert_eq!(reopened.attempts().len(), 5);
                for (key, request, response, native, receipt) in &persisted_gateway {
                    let attempt = reopened.attempts().iter()
                        .find(|attempt| attempt.query_key() == *key).unwrap();
                    assert_eq!(attempt.request_bytes(), request);
                    assert_eq!(attempt.response_bytes(), Some(response.as_slice()));
                    let terminal = reopened.query_terminal(*key).unwrap();
                    assert_eq!(terminal.native_bytes(), native);
                    assert_eq!(terminal.audit_receipt(), Some(receipt));
                }
                assert_eq!(server.snapshot(), wire);

                let resumed_at = started_at + 60_000_001;
                let reopened_clock = MacroClock {
                    now: Cell::new(UtcMicros::try_new(resumed_at).unwrap()),
                    observation: DateTime::parse_from_rfc3339(
                        "2026-09-15T15:30:00+08:00").unwrap(),
                    observation_calls: Cell::new(0),
                };
                let empty_registry = macro_search_service(&[]);
                let lease = local.resume_run(&intent, macro_lease(
                    "TEST_CODE_FINALIZE_UNKNOWN_REOPEN_OWNER",
                    resumed_at,
                    started_at + 120_000_000,
                    begin_version,
                )).unwrap();
                assert_eq!(lease.head_version(), begin_version + 1);
                assert_eq!(lease.generation(), first_generation + 1);
                let mut io = local.macro_preparation_io_v12(
                    lease, &queries, &reopened_clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &parent_source, &macro_source, &empty_registry).unwrap();
                let stopped = prepare_chain_analysis_with_io(
                    NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks, None, &mut io)
                    .await.expect_err("TEST_CODE B-only state must remain Unknown on reopen");
                assert!(matches!(stopped.downcast_ref::<PreparationStop>(),
                    Some(PreparationStop::IncompleteOnReopen { intent_id })
                        if intent_id == intent.as_str()),
                    "TEST_CODE B-only reopen must expose public IncompleteOnReopen: {stopped:?}");
                let failure = stopped.downcast_ref::<PreparationFailure>().unwrap();
                assert_eq!(failure.stage(), PreparationStage::Macro);
                assert_eq!(failure.completed_stages().last(),
                    Some(&PreparationStage::DragonTiger));
                assert!(!failure.completed_stages().contains(&PreparationStage::Macro));
                assert!(!failure.completed_stages()
                    .contains(&PreparationStage::ModelsSearchAndReport));
                drop(io);
                let final_reopened = local.inspect_macro(&intent).unwrap();
                assert!(!final_reopened.is_complete());
                assert!(final_reopened.has_unconfirmed_effect());
                assert!(final_reopened.stage_final().is_none());
                assert_eq!(final_reopened.plan_bytes(), plan_bytes);
                let final_begin = final_reopened.finalize_begin().unwrap();
                assert_eq!(final_begin.version(), begin_version);
                assert_eq!(final_begin.bytes(), begin_bytes);
                assert_eq!(final_begin.recorded_at(), started_at);
                assert_eq!(final_begin.owner(), "TEST_CODE_FULL_MACRO_OWNER");
                assert_eq!(final_begin.generation(), original_generation);
                assert_eq!(final_begin.pending(), PENDING_KEYS);
                for (key, request, response, native, receipt) in persisted_gateway {
                    let attempt = final_reopened.attempts().iter()
                        .find(|attempt| attempt.query_key() == key).unwrap();
                    assert_eq!(attempt.request_bytes(), request);
                    assert_eq!(attempt.response_bytes(), Some(response.as_slice()));
                    let terminal = final_reopened.query_terminal(key).unwrap();
                    assert_eq!(terminal.native_bytes(), native);
                    assert_eq!(terminal.audit_receipt(), Some(&receipt));
                }
                assert_eq!(local.inspect_run(&intent).unwrap().head_version(), begin_version + 1);
                assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), first_generation + 1);
                assert_eq!(reopened_clock.observation_calls.get(), 0);
                drop(local);
                assert_eq!(fixture.count("data_acquisition_audit"), audit_count + 5);
                assert_eq!(server.snapshot(), wire);
                assert_eq!(parent_server.as_ref().unwrap().snapshot(), parent_network);
                assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), parent_memberships);
            }
        }
    })).catch_unwind().await;

    // Each owner has its own bounded shutdown; never timeout a taken finish
    // JoinHandle from outside. Always join both before closing/removing the DB.
    let fault_reader_cleanup = close_macro_finalize_fault_reader(&finalize_fault_reader);
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
        .expect("TEST_CODE full cleanup panic")
        .expect("TEST_CODE full cleanup");
    parent_cleanup.expect("TEST_CODE parent cleanup after join");
    if let Some(result) = database_cleanup {
        result.expect("TEST_CODE database close");
    }
    match body {
        Ok(result) => result.expect("TEST_CODE full Macro fixture watchdog"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
    fault_reader_cleanup.expect("TEST_CODE Macro final commit reader cleanup");
}

#[tokio::test]
async fn gateway_begin_wall_budget_rollback_preserves_confirmed_lease_for_close() {
    gateway_begin_expiry_scenario(GatewayBeginExpiryScenario::RollbackBeforeCommit).await;
}

#[tokio::test]
async fn gateway_begin_late_confirmed_commit_preserves_head_receipt_and_unknown() {
    gateway_begin_expiry_scenario(GatewayBeginExpiryScenario::LateConfirmedCommit).await;
}

#[tokio::test]
async fn gateway_begin_cancelled_before_immediate_preserves_database_and_zero_rpc() {
    gateway_begin_expiry_scenario(GatewayBeginExpiryScenario::CancelledBeforeBegin).await;
}

#[tokio::test]
async fn gateway_begin_fresh_stolen_lease_precedes_duplicate_active_rejection() {
    gateway_begin_expiry_scenario(GatewayBeginExpiryScenario::FreshLeasePrecedesDuplicate).await;
}

#[tokio::test]
async fn gateway_begin_immediate_lock_conflict_is_bounded_and_preserves_database() {
    gateway_begin_expiry_scenario(GatewayBeginExpiryScenario::ImmediateLockConflict).await;
}

enum GatewayBeginExpiryScenario {
    RollbackBeforeCommit,
    LateConfirmedCommit,
    CancelledBeforeBegin,
    FreshLeasePrecedesDuplicate,
    ImmediateLockConflict,
}

async fn gateway_begin_expiry_scenario(scenario: GatewayBeginExpiryScenario) {
    use crate::push_foundation::intent_store::chain_post_close::macro_live::Live;
    use crate::search_service::macro_news::runner::{BudgetExpired, RunEnd};
    use std::rc::Rc;
    use sha2::Digest as _;

    let mut fixture = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut macro_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), async {
        let (parent_endpoint, server) = tokio::time::timeout(Duration::from_secs(5),
            spawn_macro_parent_listener(DRAGON_TIGER_RECORDS.as_bytes())).await
            .expect("TEST_CODE rollback parent listener deadline");
        parent_server = Some(server);
        macro_server = Some(MacroFullLoopbackServer::bind().await);
        let server = macro_server.as_ref().unwrap();
        let parent_source = GrpcSource::from_board_loopback_test_client(
            connect_parent_instance(&parent_endpoint).await);
        let queries = parent_source.connected_board_queries().await.unwrap();
        let (stocks, config, intent, v9_head) = populate_completed_v9_parent(
            &mut fixture, &queries, "TEST_CODE_RUN_MACRO_BUDGET_ROLLBACK").await;
        fixture.chain_post_close().migrate_schema_v9_to_v10().unwrap();
        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(&intent, lease_request(
            "TEST_CODE_ROLLBACK_PARENT_OWNER", 68_300_000_000, 90_000_000_000, Some(v9_head))).unwrap();
        let parent_clock = DragonTigerClock {
            now: UtcMicros::try_new(micros("2026-07-22T10:30:00+08:00")).unwrap(),
            request_observed_at: DateTime::parse_from_rfc3339("2026-07-22T10:30:00+08:00").unwrap(),
            request_calls: Cell::new(0), cache_calls: Cell::new(0),
        };
        let mut io = local.dragon_tiger_preparation_io_v10(
            lease, &queries, &parent_clock, FixedClusterConfiguration::resolve(Some("2")),
            &parent_source).unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks.clone(), None, &mut io)
            .await.expect_err("TEST_CODE real parent stops at Macro");
        assert_partial_macro_stop(&stopped);
        drop(io);
        let parent_head = local.inspect_run(&intent).unwrap().head_version();
        drop(local);
        let audit_count = fixture.count("data_acquisition_audit");
        let parent_network = parent_server.as_ref().unwrap().snapshot();
        let parent_memberships = parent_server.as_ref().unwrap().membership_snapshot();
        let database = fixture.database();
        fixture.chain_post_close().migrate_schema_v10_to_v11().unwrap();
        fixture.chain_post_close().migrate_schema_v11_to_v12().unwrap();
        let macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await, server.endpoint().to_owned());
        let connected = macro_source.connected_local_macro_queries().unwrap();
        let registered = [GeneralWebResearchProvider::SerpApi,
            GeneralWebResearchProvider::Bocha, GeneralWebResearchProvider::Tavily];
        let search_service = macro_search_service(&registered);
        let web = search_service.macro_web_snapshot(&macro_source).unwrap();
        // Capture real Local requests without polling an authorized effect.
        let identities = [
            MacroQueryIdentity::GlobalNews { provider: GlobalNewsProvider::Eastmoney, limit: 20 },
            MacroQueryIdentity::GlobalNews { provider: GlobalNewsProvider::Cailianpress, limit: 20 },
            MacroQueryIdentity::GlobalNews { provider: GlobalNewsProvider::Jin10, limit: 20 },
            MacroQueryIdentity::GlobalNews { provider: GlobalNewsProvider::ThePaper, limit: 20 },
            MacroQueryIdentity::EconomicCalendar,
        ];
        let requests = identities.into_iter().enumerate().map(|(index, identity)| {
            let request = macro_codec::Request::capture_for(&identity,
                &connected.session(identity.clone()).unwrap().authorize_next().unwrap()).unwrap();
            (QueryKey::Gateway(u8::try_from(index + 1).unwrap()), request,
                connected.endpoint().to_owned())
        }).collect::<Vec<_>>();
        let gateway_a = requests[0].1.clone();
        let started_at = micros("2026-09-14T15:30:00+08:00");
        let deadline = started_at + 15_000_000;
        let until = started_at + 60_000_000;
        let clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-14T15:30:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        if matches!(&scenario, GatewayBeginExpiryScenario::ImmediateLockConflict) {
            let connection = &fixture.store.as_ref().unwrap().connection;
            connection.busy_timeout(Duration::ZERO)
                .expect("TEST_CODE bound data-begin lock refusal");
            assert_eq!(connection.query_row("PRAGMA busy_timeout", [],
                |row| row.get::<_, i64>(0)).unwrap(), 0);
        }
        let mut local = fixture.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&config).unwrap();
        let lease = local.resume_run(&intent, macro_lease(
            "TEST_CODE_ROLLBACK_MACRO_OWNER", started_at, until, parent_head)).unwrap();
        let generation = lease.generation();
        let cancelled = Rc::new(Cell::new(false));
        let mut live = Live::open(&mut local, lease, &clock, cancelled.clone())
            .expect("TEST_CODE setup must open within original budget");
        live.initialize(clock.macro_request_observation(), connected.endpoint(), requests, None, &web)
            .expect("TEST_CODE setup must initialize within original budget, not target RED");
        assert_eq!(live.deadline(), deadline);
        assert_eq!(live.lease_until(), until);

        // Every invocation opens and closes a new connection: no old read
        // transaction can hide a committed begin or falsely prove rollback.
        let read_committed = || {
            let mut reader = BusinessIntentStore::open(&database).unwrap();
            let mut read_local = reader.single_user_local_chain_post_close(&config).unwrap();
            let run = read_local.inspect_run(&intent).unwrap();
            let recovered = read_local.inspect_macro(&intent).unwrap();
            drop(read_local);
            let identity = reader.connection.query_row(
                "SELECT run_id,lease_owner,lease_generation,lease_until FROM chain_post_close_runs WHERE intent_id=?1",
                [intent.as_str()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?, row.get::<_, i64>(3)?))).unwrap();
            let requests = {
                let mut statement = reader.connection.prepare(
                    "SELECT phase,item_ordinal,candidate_ordinal,bytes FROM chain_post_close_macro_request_plans WHERE intent_id=?1 ORDER BY item_ordinal").unwrap();
                let rows = statement.query_map([intent.as_str()], |row| Ok((row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, Vec<u8>>(3)?)))
                    .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
                rows
            };
            let audits = reader.connection.query_row("SELECT count(*) FROM data_acquisition_audit", [],
                |row| row.get::<_, i64>(0)).unwrap();
            reader.connection.close().unwrap();
            (run, recovered, identity, requests, audits)
        };
        let database_state = || {
            let reader = BusinessIntentStore::open(&database).unwrap();
            let state = super::v11_migration_tests::DatabaseState::capture(&reader.connection);
            reader.connection.close().unwrap();
            state
        };
        let (before_run, before_macro, before_identity, before_requests, before_audits) = read_committed();
        let confirmed_head = before_run.head_version();
        assert_eq!(before_run.lease_generation(), generation);
        assert_eq!(before_identity.1, "TEST_CODE_ROLLBACK_MACRO_OWNER");
        assert_eq!(before_identity.2, i64::try_from(generation).unwrap());
        assert_eq!(before_identity.3, until);
        assert_eq!(before_requests.len(), 5);
        for (index, (phase, item, candidate, bytes)) in before_requests.iter().enumerate() {
            assert_eq!(phase, "Gateway");
            assert_eq!(*item, i64::try_from(index + 1).unwrap());
            assert_eq!(*candidate, 1);
            assert!(!bytes.is_empty());
        }
        assert_eq!(before_audits, audit_count);
        assert!(before_macro.attempts().is_empty());
        assert!(!before_macro.has_unconfirmed_effect());
        assert_eq!(before_macro.plan().deadline_at().get(), deadline);
        assert!(server.snapshot().calls.is_empty());

        match scenario {
            GatewayBeginExpiryScenario::CancelledBeforeBegin => {
                let before = database_state();
                cancelled.set(true);
                let begin_error = match live.begin_data(
                    QueryKey::Gateway(1), 1, gateway_a, connected.endpoint()) {
                    Err(error) => error,
                    Ok(ticket) => {
                        drop(ticket);
                        panic!("TEST_CODE cancelled owner reached Immediate DataBegin");
                    }
                };
                assert!(matches!(begin_error.downcast_ref::<PreparationStop>(),
                    Some(PreparationStop::ResultUnconfirmed { intent_id })
                        if intent_id == intent.as_str()),
                    "TEST_CODE cancelled data begin error: {begin_error:?}");
                let lease = live.into_lease();
                assert_eq!(lease.head_version(), confirmed_head);
                assert_eq!(lease.generation(), generation);
                drop(lease);
                drop(local);
                fixture.reopen();
                assert_eq!(database_state(), before);
            }
            GatewayBeginExpiryScenario::FreshLeasePrecedesDuplicate => {
                let ticket = live.begin_data(
                    QueryKey::Gateway(1), 1, gateway_a.clone(), connected.endpoint())
                    .expect("TEST_CODE first real DataBegin");
                assert!(!cancelled.get());
                assert!(server.snapshot().calls.is_empty());

                let stealer = BusinessIntentStore::open(&database).unwrap();
                assert_eq!(stealer.connection.execute(
                    "UPDATE chain_post_close_runs SET \
                     lease_owner=?1,lease_generation=lease_generation+1, \
                     lease_until=lease_until+1,head_version=head_version+1, \
                     updated_at=updated_at+1 WHERE intent_id=?2",
                    rusqlite::params!["TEST_CODE_STOLEN_MACRO_OWNER", intent.as_str()]).unwrap(), 1);
                stealer.connection.close().unwrap();
                let injected = database_state();

                let begin_error = match live.begin_data(
                    QueryKey::Gateway(1), 1, gateway_a, connected.endpoint()) {
                    Err(error) => error,
                    Ok(second_ticket) => {
                        drop(second_ticket);
                        panic!("TEST_CODE stale lease admitted duplicate active effect");
                    }
                };
                assert!(matches!(begin_error.downcast_ref::<ChainPostCloseError>(),
                    Some(ChainPostCloseError::StaleLease { intent_id })
                        if intent_id == intent.as_str()),
                    "TEST_CODE fresh actual priority error: {begin_error:?}");
                assert_eq!(database_state(), injected,
                    "TEST_CODE rejected duplicate must not append another fact");
                assert!(server.snapshot().calls.is_empty());
                drop(ticket);
                assert!(cancelled.get(), "TEST_CODE owned first ticket remains armed");
                let stale_lease = live.into_lease();
                assert_eq!(stale_lease.head_version(), confirmed_head + 1);
                assert_eq!(stale_lease.generation(), generation);
                drop(stale_lease);
                drop(local);
                fixture.reopen();
                assert_eq!(database_state(), injected);
            }
            GatewayBeginExpiryScenario::ImmediateLockConflict => {
                let before = database_state();
                let mut locker = BusinessIntentStore::open(&database).unwrap();
                let lock = locker.connection
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .expect("TEST_CODE hold competing Immediate transaction");
                let begin_error = match live.begin_data(
                    QueryKey::Gateway(1), 1, gateway_a, connected.endpoint()) {
                    Err(error) => error,
                    Ok(ticket) => {
                        drop(ticket);
                        panic!("TEST_CODE competing Immediate admitted DataBegin");
                    }
                };
                assert!(matches!(begin_error.downcast_ref::<ChainPostCloseError>(),
                    Some(ChainPostCloseError::StorageFailed { operation })
                        if *operation == "full Macro data begin"),
                    "TEST_CODE Immediate conflict error: {begin_error:?}");
                drop(lock);
                locker.connection.close().unwrap();
                assert_eq!(database_state(), before);
                assert!(server.snapshot().calls.is_empty());
                let lease = live.into_lease();
                assert_eq!(lease.head_version(), confirmed_head);
                assert_eq!(lease.generation(), generation);
                drop(lease);
                drop(local);
                fixture.reopen();
                assert_eq!(database_state(), before);
            }
            GatewayBeginExpiryScenario::RollbackBeforeCommit => {
                live.arm_gateway_a_begin_wall_expiry_for_test(&clock.now);
                let begin_error = match live.begin_data(QueryKey::Gateway(1), 1, gateway_a, connected.endpoint()) {
                    Err(error) => error,
                    Ok(ticket) => {
                        drop(ticket);
                        panic!("TEST_CODE precommit expiry must not mint a ticket");
                    }
                };
                let budget_expired = begin_error.downcast_ref::<BudgetExpired>().is_some();
                assert!(budget_expired, "TEST_CODE expected real gate BudgetExpired, got {begin_error:?}");
                assert_eq!(clock.now.get().get(), deadline,
                    "TEST_CODE exact post-admitted hook must fire; earlier timeout is not target RED");
                let (after_run, after_macro, after_identity, after_requests, after_audits) = read_committed();
                assert_eq!(after_run.head_version(), confirmed_head, "TEST_CODE fresh DB confirms rollback");
                assert_eq!(after_identity, before_identity);
                assert_eq!(after_requests, before_requests);
                assert_eq!(after_macro.plan_bytes(), before_macro.plan_bytes());
                assert!(after_macro.attempts().is_empty(), "TEST_CODE no committed DataBegin or Result");
                assert!(!after_macro.has_unconfirmed_effect());
                assert!(after_macro.stage_final().is_none());
                for ordinal in 1..=5 {
                    assert!(after_macro.query_terminal(QueryKey::Gateway(ordinal)).is_none());
                }
                assert_eq!(after_audits, before_audits);
                assert!(!cancelled.get());
                assert!(server.snapshot().calls.is_empty());
                // First regression gate: a real driver capability must expose only the
                // confirmed H, not the rolled-back DataBegin's speculative H+1.
                let lease = live.into_lease();
                assert_eq!(lease.head_version(), confirmed_head,
                    "TEST_CODE budget rollback published an unconfirmed head: capability={}, database={}, D={}, until={}, BudgetExpired={}",
                    lease.head_version(), after_run.head_version(), deadline, until, budget_expired);
                assert_eq!(lease.generation(), generation);

                // Same capability and clock, no resume_run/owner change/new budget.
                // This exercises Live re-entry at D, not the full same-Live runner.
                let mut live = Live::open(&mut local, lease, &clock, cancelled)
                    .expect("TEST_CODE confirmed capability must reopen at original deadline");
                assert_eq!(live.deadline(), deadline);
                assert_eq!(live.lease_until(), until);
                assert_eq!(live.close(RunEnd::BudgetExpired).unwrap(), "");
                let lease = live.into_lease();
                assert_eq!(lease.head_version(), confirmed_head + 2);
                assert_eq!(lease.generation(), generation);
                drop(lease);
                drop(local);
                fixture.reopen();
                let (final_run, final_macro, final_identity, final_requests, final_audits) = read_committed();
                assert_eq!(final_run.head_version(), confirmed_head + 2);
                assert_eq!(final_identity, before_identity);
                assert_eq!(final_requests, before_requests);
                assert_eq!(final_macro.plan_bytes(), before_macro.plan_bytes());
                assert!(final_macro.is_complete());
                assert!(!final_macro.has_unconfirmed_effect());
                assert!(final_macro.attempts().is_empty());
                let final_ = final_macro.stage_final().unwrap();
                let begin = final_macro.finalize_begin().unwrap();
                assert_eq!(begin.expiry(), &ExpiryBasis::WallDeadline);
                assert_eq!(begin.recorded_at(), deadline);
                assert_eq!(final_.recorded_at(), deadline);
                assert_eq!(begin.bytes(), final_.finalize_begin_bytes());
                assert_eq!(final_.kind(), crate::push_foundation::intent_store::chain_post_close::macro_native::FinalKind::BudgetExpired);
                assert_eq!(final_.finalize_begin_version(), confirmed_head + 1);
                assert_eq!(final_.version(), confirmed_head + 2);
                assert_eq!(final_.started_at(), started_at);
                assert_eq!(final_.deadline_at(), deadline);
                assert_eq!(final_.output_bytes(), b"");
                assert_eq!(final_audits, before_audits);
            }
            GatewayBeginExpiryScenario::LateConfirmedCommit => {
                live.arm_gateway_a_begin_late_wall_expiry_for_test(&clock.now);
                let request_bytes = gateway_a.bytes.clone();
                let begin_error = match live.begin_data(QueryKey::Gateway(1), 1, gateway_a, connected.endpoint()) {
                    Err(error) => error,
                    Ok(ticket) => {
                        drop(ticket);
                        panic!("TEST_CODE late confirmed begin must not return an effect ticket");
                    }
                };
                let receipt = match begin_error.downcast_ref::<PreparationStop>() {
                    Some(PreparationStop::DeadlineAfterConfirmedCommit { receipt }) => receipt,
                    _ => panic!("TEST_CODE expected confirmed-commit receipt, got {begin_error:?}"),
                };
                assert_eq!(clock.now.get().get(), deadline,
                    "TEST_CODE AfterCommit phase must actually advance original wall clock");
                assert_eq!(receipt.intent_id, intent.as_str());
                assert_eq!(receipt.first_version, confirmed_head + 1);
                assert_eq!(receipt.last_version, confirmed_head + 1);
                assert!(receipt.audit_receipts.is_empty());
                let (after_run, after_macro, after_identity, after_requests, after_audits) = read_committed();
                assert_eq!(after_run.head_version(), confirmed_head + 1);
                assert_eq!(after_identity, before_identity);
                assert_eq!(after_requests, before_requests);
                assert_eq!(after_macro.plan_bytes(), before_macro.plan_bytes());
                assert_eq!(after_macro.plan().started_at().get(), started_at);
                assert_eq!(after_macro.plan().deadline_at().get(), deadline);
                assert!(after_macro.has_unconfirmed_effect());
                assert!(!after_macro.is_complete());
                assert!(after_macro.stage_final().is_none());
                assert_eq!(after_macro.attempts().len(), 1);
                let attempt = &after_macro.attempts()[0];
                assert_eq!(attempt.query_key(), QueryKey::Gateway(1));
                assert_eq!(attempt.attempt_ordinal(), 1);
                assert_eq!(attempt.begin_version(), confirmed_head + 1);
                assert_eq!(attempt.request_bytes(), request_bytes);
                assert!(attempt.result_version().is_none());
                assert!(attempt.response_bytes().is_none());
                assert!(attempt.continuation().is_none());
                for ordinal in 1..=5 {
                    assert!(after_macro.query_terminal(QueryKey::Gateway(ordinal)).is_none());
                }
                assert_eq!(after_audits, before_audits);
                let fact_tables = [
                    "chain_post_close_macro_plans",
                    "chain_post_close_macro_request_plans",
                    "chain_post_close_macro_readiness_episode_plans",
                    "chain_post_close_macro_control_attempt_begins",
                    "chain_post_close_macro_control_attempt_results",
                    "chain_post_close_macro_attempt_begins",
                    "chain_post_close_macro_attempt_results",
                    "chain_post_close_macro_source_finals",
                    "chain_post_close_macro_query_terminals",
                    "chain_post_close_macro_dimension_terminals",
                    "chain_post_close_macro_finalize_begins",
                    "chain_post_close_macro_stage_finals",
                ].map(str::to_owned).to_vec();
                let reader = BusinessIntentStore::open(&database).unwrap();
                let (recorded_at, begin_bytes, begin_sha) = reader.connection.query_row(
                    "SELECT recorded_at,bytes,sha256 FROM chain_post_close_macro_attempt_begins WHERE intent_id=?1 AND run_version=?2",
                    rusqlite::params![intent.as_str(), confirmed_head + 1],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, String>(2)?))).unwrap();
                let confirmed_facts = old_fact_rows(&reader.connection, &fact_tables);
                reader.connection.close().unwrap();
                assert_eq!(recorded_at, started_at, "TEST_CODE historical begin time is not rewritten to D");
                assert_eq!(receipt.last_fact_sha256, begin_sha);
                assert_eq!(format!("{:x}", sha2::Sha256::digest(&begin_bytes)), begin_sha);
                for table in [
                    "chain_post_close_macro_attempt_results",
                    "chain_post_close_macro_source_finals",
                    "chain_post_close_macro_query_terminals",
                    "chain_post_close_macro_dimension_terminals",
                    "chain_post_close_macro_finalize_begins",
                    "chain_post_close_macro_stage_finals",
                ] {
                    assert!(confirmed_facts[table].is_empty());
                }
                assert!(cancelled.get(), "TEST_CODE unreturned armed ticket stops this owner");
                let stopped = live.checkpoint().expect_err("TEST_CODE late owner remains stopped");
                assert!(matches!(stopped.downcast_ref::<PreparationStop>(),
                    Some(PreparationStop::ResultUnconfirmed { intent_id }) if intent_id == intent.as_str()));
                let lease = live.into_lease();
                assert_eq!(lease.head_version(), confirmed_head + 1,
                    "TEST_CODE confirmed head must not roll back when after_commit returns Err");
                assert_eq!(lease.generation(), generation);
                assert!(server.snapshot().calls.is_empty());
                drop(local);
                fixture.reopen();
                assert!(deadline < until);
                let observations = clock.observation_calls.get();
                let mut local = fixture.store.as_mut().unwrap()
                    .single_user_local_chain_post_close(&config).unwrap();
                let mut io = local.macro_preparation_io_v12(
                    lease, &queries, &clock, FixedClusterConfiguration::resolve(Some("2")),
                    &parent_source, &macro_source, &search_service).unwrap();
                // A mistaken replay returns instead of hanging on a held fixture lane.
                server.release_all();
                let stopped = prepare_chain_analysis_with_io(
                    NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), stocks, None, &mut io)
                    .await.expect_err("TEST_CODE persisted U must reopen Unknown before budget handling");
                assert!(matches!(stopped.downcast_ref::<PreparationStop>(),
                    Some(PreparationStop::IncompleteOnReopen { intent_id }) if intent_id == intent.as_str()),
                    "TEST_CODE actual recovery stop: {stopped:?}");
                let failure = stopped.downcast_ref::<PreparationFailure>().unwrap();
                assert_eq!(failure.stage(), PreparationStage::Macro);
                assert_eq!(failure.completed_stages().last(), Some(&PreparationStage::DragonTiger));
                assert!(!failure.completed_stages().contains(&PreparationStage::Macro));
                assert!(!failure.completed_stages().contains(&PreparationStage::ModelsSearchAndReport));
                drop(io);
                drop(local);
                let (reopened_run, reopened, reopened_identity, reopened_requests, reopened_audits) = read_committed();
                assert_eq!(reopened_run.head_version(), confirmed_head + 1);
                assert_eq!(reopened_identity, before_identity);
                assert_eq!(reopened_requests, before_requests);
                assert_eq!(reopened.plan_bytes(), before_macro.plan_bytes());
                assert_eq!(reopened.plan().started_at().get(), started_at);
                assert_eq!(reopened.plan().deadline_at().get(), deadline);
                assert!(reopened.has_unconfirmed_effect());
                assert!(!reopened.is_complete());
                assert!(reopened.stage_final().is_none());
                assert_eq!(reopened.attempts().len(), 1);
                assert_eq!(reopened.attempts()[0].begin_version(), confirmed_head + 1);
                assert!(reopened.attempts()[0].result_version().is_none());
                assert_eq!(reopened_audits, before_audits);
                assert_eq!(clock.observation_calls.get(), observations);
                assert_eq!(clock.now.get().get(), deadline);
                let reader = BusinessIntentStore::open(&database).unwrap();
                assert_eq!(old_fact_rows(&reader.connection, &fact_tables), confirmed_facts);
                reader.connection.close().unwrap();
            }
        }
        let wire = server.snapshot();
        assert!(wire.calls.is_empty());
        assert_eq!(wire.controls, 0);
        assert!(wire.unexpected.is_empty());
        assert_eq!(parent_server.as_ref().unwrap().snapshot(), parent_network);
        assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), parent_memberships);
    })).catch_unwind().await;

    // The body and its Live/IO/reader borrows are gone on panic/timeout too.
    // Owners bound their own shutdown: never timeout a taken finish handle.
    let macro_cleanup = match macro_server.take() {
        Some(server) => std::panic::AssertUnwindSafe(server.finish()).catch_unwind().await,
        None => Ok(Ok(())),
    };
    let parent_cleanup = match parent_server.take() {
        Some(server) => std::panic::AssertUnwindSafe(server.finish()).catch_unwind().await.map(|_| ()),
        None => Ok(()),
    };
    let database_cleanup = fixture.store.take().map(|store| {
        store.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        })
    });
    drop(fixture);
    macro_cleanup.expect("TEST_CODE rollback Macro cleanup panic").expect("TEST_CODE rollback Macro cleanup");
    parent_cleanup.expect("TEST_CODE rollback parent cleanup after join");
    if let Some(result) = database_cleanup {
        result.expect("TEST_CODE rollback database close");
    }
    match body {
        Ok(result) => result.expect("TEST_CODE rollback fixture watchdog"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

// Independent literal: the original HealthNotReady decision is reused for four
// logical news acquisitions; original explicit Local unavailability supplies E.
// Do not derive this expectation from the renderer or native codec under test.
const EXPECTED_V2_REJECTED_MACRO: &str = concat!(
    "## 📡 今日宏观 / 市场背景（2026年09月14日）\n\n",
    "### 📰 东方财富财经要闻\n",
    "- 数据不可用：reason_code=external_health_not_ready retryable=true\n\n",
    "### 🧭 财联社电报\n",
    "- 数据不可用：reason_code=external_health_not_ready retryable=true\n\n",
    "### 📣 金十快讯\n",
    "- 数据不可用：reason_code=external_health_not_ready retryable=true\n\n",
    "### 🌐 澎湃财经\n",
    "- 数据不可用：reason_code=external_health_not_ready retryable=true\n\n",
    "### 📊 最新经济数据发布（金十）\n",
    "- 数据不可用：reason_code=no_verified_batch retryable=false",
);

#[derive(Clone, Debug, PartialEq)]
struct HistoricalGroupGatewayFive {
    version: u64,
    fact_bytes: Vec<u8>,
    native_bytes: Vec<u8>,
    receipt: crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt,
    owner: String,
    generation: u64,
    recorded_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
struct HistoricalGroupSnapshot {
    run: (u64, String, u64, i64),
    gateway_five: HistoricalGroupGatewayFive,
    pending_group: (i64, i64),
    source_final_count: i64,
    prefix: Vec<(&'static str, Vec<Vec<rusqlite::types::Value>>)>,
    finalization: Vec<(&'static str, Vec<Vec<rusqlite::types::Value>>)>,
    audits: Vec<Vec<rusqlite::types::Value>>,
    audit_chain: Vec<Vec<rusqlite::types::Value>>,
}

fn historical_rows(
    connection: &rusqlite::Connection,
    table: &'static str,
    intent: &str,
) -> Vec<Vec<rusqlite::types::Value>> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT * FROM {table} WHERE intent_id=?1 ORDER BY run_version"
        ))
        .expect("TEST_CODE prepare historical group snapshot");
    let width = statement.column_count();
    let rows = statement
        .query_map([intent], move |row| {
            (0..width)
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .expect("TEST_CODE query historical group snapshot");
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .expect("TEST_CODE collect historical group snapshot")
}

fn historical_all_rows(
    connection: &rusqlite::Connection,
    sql: &str,
) -> Vec<Vec<rusqlite::types::Value>> {
    let mut statement = connection
        .prepare(sql)
        .expect("TEST_CODE prepare historical audit snapshot");
    let width = statement.column_count();
    let rows = statement
        .query_map([], move |row| {
            (0..width)
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .expect("TEST_CODE query historical audit snapshot");
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .expect("TEST_CODE collect historical audit snapshot")
}

fn historical_group_snapshot(
    connection: &rusqlite::Connection,
    intent: &str,
) -> HistoricalGroupSnapshot {
    let run = connection
        .query_row(
            "SELECT head_version,lease_owner,lease_generation,lease_until \
             FROM chain_post_close_runs WHERE intent_id=?1",
            [intent],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("TEST_CODE snapshot historical group run");
    let gateway_five = connection
        .query_row(
            "SELECT run_version,bytes,native_bytes,audit_id,audit_record_hash, \
                    previous_outcome,current_outcome,lease_owner,lease_generation,recorded_at \
             FROM chain_post_close_macro_query_terminals \
             WHERE intent_id=?1 AND phase='Gateway' AND item_ordinal=5 AND candidate_ordinal=1",
            [intent],
            |row| {
                Ok(HistoricalGroupGatewayFive {
                    version: row.get(0)?,
                    fact_bytes: row.get(1)?,
                    native_bytes: row.get(2)?,
                    receipt: crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt {
                        audit_id: row.get(3)?,
                        record_hash: row.get(4)?,
                        previous_outcome: row.get(5)?,
                        current_outcome: row.get(6)?,
                    },
                    owner: row.get(7)?,
                    generation: row.get(8)?,
                    recorded_at: row.get(9)?,
                })
            },
        )
        .expect("TEST_CODE snapshot committed Gateway5 terminal");
    let pending_group = (
        connection
            .query_row(
                "SELECT count(*) FROM chain_post_close_macro_request_plans \
                 WHERE intent_id=?1 AND phase='Gateway' AND item_ordinal BETWEEN 2 AND 4",
                [intent],
                |row| row.get(0),
            )
            .expect("TEST_CODE count historical group requests"),
        connection
            .query_row(
                "SELECT count(*) FROM chain_post_close_macro_query_terminals \
                 WHERE intent_id=?1 AND phase='Gateway' AND item_ordinal BETWEEN 2 AND 4",
                [intent],
                |row| row.get(0),
            )
            .expect("TEST_CODE count historical group terminals"),
    );
    let source_final_count = connection
        .query_row(
            "SELECT count(*) FROM chain_post_close_macro_source_finals WHERE intent_id=?1",
            [intent],
            |row| row.get(0),
        )
        .expect("TEST_CODE count original SourceFinal");
    let prefix = [
        "chain_post_close_macro_plans",
        "chain_post_close_macro_request_plans",
        "chain_post_close_macro_readiness_episode_plans",
        "chain_post_close_macro_control_attempt_begins",
        "chain_post_close_macro_control_attempt_results",
        "chain_post_close_macro_source_finals",
        "chain_post_close_macro_query_terminals",
    ]
    .into_iter()
    .map(|table| (table, historical_rows(connection, table, intent)))
    .collect();
    let finalization = [
        "chain_post_close_macro_dimension_terminals",
        "chain_post_close_macro_finalize_begins",
        "chain_post_close_macro_stage_finals",
    ]
    .into_iter()
    .map(|table| (table, historical_rows(connection, table, intent)))
    .collect();
    HistoricalGroupSnapshot {
        run,
        gateway_five,
        pending_group,
        source_final_count,
        prefix,
        finalization,
        audits: historical_all_rows(
            connection,
            "SELECT * FROM data_acquisition_audit ORDER BY id",
        ),
        audit_chain: historical_all_rows(
            connection,
            "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id",
        ),
    }
}

struct HistoricalGroupCommitClock {
    inner: MacroClock,
    intent: String,
    reader: std::rc::Rc<RefCell<Option<rusqlite::Connection>>>,
    armed: Cell<bool>,
    after_arm_calls: Cell<usize>,
    locked: RefCell<Option<HistoricalGroupSnapshot>>,
}

impl HistoricalGroupCommitClock {
    fn new(
        inner: MacroClock,
        intent: String,
        database: std::path::PathBuf,
        reader: std::rc::Rc<RefCell<Option<rusqlite::Connection>>>,
    ) -> Self {
        let connection = rusqlite::Connection::open_with_flags(
            database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .expect("TEST_CODE open owned historical group commit reader");
        connection
            .busy_timeout(Duration::from_millis(250))
            .expect("TEST_CODE bound historical group reader timeout");
        let journal_mode: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("TEST_CODE historical group reader journal mode");
        assert_eq!(journal_mode.to_ascii_lowercase(), "delete");
        assert!(reader.borrow().is_none());
        *reader.borrow_mut() = Some(connection);
        Self {
            inner,
            intent,
            reader,
            armed: Cell::new(false),
            after_arm_calls: Cell::new(0),
            locked: RefCell::new(None),
        }
    }

    fn locked_snapshot(&self) -> HistoricalGroupSnapshot {
        self.locked
            .borrow()
            .clone()
            .expect("TEST_CODE historical group COMMIT fault must arm")
    }
}

impl ConceptEffectClock for HistoricalGroupCommitClock {
    fn now(&self) -> UtcMicros {
        if self.armed.get() {
            self.after_arm_calls.set(self.after_arm_calls.get() + 1);
            return self.inner.now();
        }
        let reader = self.reader.borrow();
        let reader = reader
            .as_ref()
            .expect("TEST_CODE historical group reader remains owned");
        let gateway_five_without_group = reader
            .query_row(
                "SELECT EXISTS( \
                     SELECT 1 FROM chain_post_close_macro_query_terminals \
                     WHERE intent_id=?1 AND phase='Gateway' AND item_ordinal=5 \
                 ) AND NOT EXISTS( \
                     SELECT 1 FROM chain_post_close_macro_request_plans \
                     WHERE intent_id=?1 AND phase='Gateway' AND item_ordinal BETWEEN 2 AND 4 \
                 ) AND NOT EXISTS( \
                     SELECT 1 FROM chain_post_close_macro_query_terminals \
                     WHERE intent_id=?1 AND phase='Gateway' AND item_ordinal BETWEEN 2 AND 4 \
                 )",
                [self.intent.as_str()],
                |row| row.get::<_, bool>(0),
            )
            .expect("TEST_CODE probe committed Gateway5 before historical group");
        if gateway_five_without_group {
            reader
                .execute_batch("BEGIN DEFERRED;")
                .expect("TEST_CODE begin historical group COMMIT read lock");
            let locked = historical_group_snapshot(reader, self.intent.as_str());
            assert_eq!(locked.pending_group, (0, 0));
            assert_eq!(locked.run.0, locked.gateway_five.version);
            *self.locked.borrow_mut() = Some(locked);
            self.armed.set(true);
        }
        self.inner.now()
    }
}

impl PositionObservationClock for HistoricalGroupCommitClock {
    fn cache_observation(&self) -> PositionCacheObservation {
        self.inner.cache_observation()
    }
}

impl DragonTigerObservationClock for HistoricalGroupCommitClock {
    fn dragon_tiger_request_observation(&self) -> DateTime<chrono::FixedOffset> {
        self.inner.dragon_tiger_request_observation()
    }
}

impl MacroObservationClock for HistoricalGroupCommitClock {
    fn macro_request_observation(&self) -> DateTime<chrono::FixedOffset> {
        self.inner.macro_request_observation()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum HistoricalRejectionScenario {
    Success,
    GroupCommitFailure,
}

enum HistoricalContinuationClock {
    Regular(MacroClock),
    GroupCommitFailure(HistoricalGroupCommitClock),
}

impl HistoricalContinuationClock {
    fn inner(&self) -> &MacroClock {
        match self {
            Self::Regular(clock) => clock,
            Self::GroupCommitFailure(clock) => &clock.inner,
        }
    }

    fn set_now(&self, now: UtcMicros) {
        self.inner().now.set(now);
    }

    fn group_commit_fault(&self) -> Option<&HistoricalGroupCommitClock> {
        match self {
            Self::Regular(_) => None,
            Self::GroupCommitFailure(clock) => Some(clock),
        }
    }
}

impl ConceptEffectClock for HistoricalContinuationClock {
    fn now(&self) -> UtcMicros {
        match self {
            Self::Regular(clock) => clock.now(),
            Self::GroupCommitFailure(clock) => clock.now(),
        }
    }
}

impl PositionObservationClock for HistoricalContinuationClock {
    fn cache_observation(&self) -> PositionCacheObservation {
        self.inner().cache_observation()
    }
}

impl DragonTigerObservationClock for HistoricalContinuationClock {
    fn dragon_tiger_request_observation(&self) -> DateTime<chrono::FixedOffset> {
        self.inner().dragon_tiger_request_observation()
    }
}

impl MacroObservationClock for HistoricalContinuationClock {
    fn macro_request_observation(&self) -> DateTime<chrono::FixedOffset> {
        self.inner().macro_request_observation()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn single_user_v2_external_rejected_prefix_continues_under_new_owner_without_replaying_controls() {
    historical_rejection_scenario(HistoricalRejectionScenario::Success).await;
}

#[tokio::test(flavor = "current_thread")]
async fn single_user_v2_historical_rejection_group_commit_failure_rolls_back_atomically_after_reopen() {
    historical_rejection_scenario(HistoricalRejectionScenario::GroupCommitFailure).await;
}

async fn historical_rejection_scenario(scenario: HistoricalRejectionScenario) {
    use crate::grpc_client::client::external_control_loopback_fixture::ExternalMtlsMacroFixture;
    use crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroControlOutcome;

    fn fact_sha(bytes: &[u8]) -> String {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(bytes))
    }

    fn assert_final_stop(error: &anyhow::Error) {
        assert!(
            matches!(error.downcast_ref::<PreparationStop>(),
                Some(PreparationStop::StageNotMigrated { next: UnmigratedStage::ModelsSearchAndReport })),
            "TEST_CODE old v2 rejected prefix must actually continue to ModelsSearchAndReport; received {error:?}"
        );
        let failure = error.downcast_ref::<PreparationFailure>().unwrap();
        assert_eq!(failure.stage(), PreparationStage::ModelsSearchAndReport);
        assert_eq!(failure.completed_stages().last(), Some(&PreparationStage::Macro));
        assert_eq!(failure.macro_context().as_bytes(), EXPECTED_V2_REJECTED_MACRO.as_bytes());
        assert_eq!(failure.lhb_map()["TEST_CODE_600001"].to_bits(), 12.5_f64.to_bits());
    }

    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let historical_fault_reader =
        std::rc::Rc::new(RefCell::new(None::<rusqlite::Connection>));
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(120), async {
        let baseline = control_tests::setup_external_parent(
            &mut business, &mut parent_server, "TEST_CODE_V2_REJECTED_FORWARD",
        ).await;
        external_server = Some(ExternalMtlsMacroFixture::bind_health_not_ready_for_test()
            .await.expect("TEST_CODE owned HealthNotReady fixture"));
        let external = external_server.as_ref().unwrap();
        let macro_source = GrpcSource::from_external_macro_bundle_for_test(
            external.bundle_path().to_path_buf(),
        );
        let registered = [
            GeneralWebResearchProvider::SerpApi,
            GeneralWebResearchProvider::Bocha,
            GeneralWebResearchProvider::Tavily,
        ];
        let search = macro_search_service(&registered);
        let started_at = micros("2026-09-14T15:31:00+08:00");
        let clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-14T15:31:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let audit_before = business.count("data_acquisition_audit");
        let mut local = business.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&baseline.config).unwrap();
        let lease = local.resume_run(&baseline.intent, macro_lease(
            "TEST_CODE_V2_REJECTED_OWNER_A", started_at, started_at + 2_000_000, baseline.head,
        )).unwrap();
        let mut io = local.macro_preparation_io_v11(
            lease, &baseline.queries, &clock, FixedClusterConfiguration::resolve(Some("2")),
            &baseline.source, &macro_source, &search,
        ).unwrap();
        let stopped = {
            let mut prepared = Box::pin(prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), baseline.stocks.clone(), None, &mut io,
            ));
            let received_by = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                tokio::select! {
                    biased;
                    result = &mut prepared => panic!(
                        "TEST_CODE v11 prepare returned before the actual Health request: {result:?}"
                    ),
                    _ = tokio::task::yield_now() => {}
                }
                if external.snapshot().health_requests.len() == 1 { break; }
                assert!(std::time::Instant::now() < received_by,
                    "TEST_CODE Health receipt watchdog is not the expected RED");
            }
            clock.now.set(UtcMicros::try_new(started_at + 100_000).unwrap());
            external.release_health();
            tokio::time::timeout(Duration::from_secs(5), &mut prepared).await
                .expect("TEST_CODE actual v11 Health-result watchdog")
                .expect_err("TEST_CODE v11 Macro remains intentionally partial")
        };
        assert_partial_macro_stop(&stopped);
        drop(io);
        let original = local.inspect_macro(&baseline.intent).unwrap();
        assert!(!original.has_unconfirmed_effect());
        assert!(original.attempts().is_empty());
        assert_eq!(original.plan().started_at().get(), started_at);
        assert_eq!(original.plan().deadline_at().get(), started_at + 15_000_000);
        assert_eq!(original.plan().research_providers(), registered);
        assert!(original.plan().research_decisions().iter().all(|decision|
            !decision.is_available() &&
            decision.availability_source() == "explicit-registry-local-semantic-search-disconnected"));
        assert_eq!(serde_json::from_slice::<serde_json::Value>(original.plan_bytes()).unwrap()["version"], 2);
        let plan_bytes = original.plan_bytes().to_vec();
        let original_request = original.plan().first_source_request().request_bytes().to_vec();
        let controls = original.readiness_episodes().iter().flat_map(|episode| episode.controls().iter())
            .map(|control| (
                control.request_bytes().to_vec(), control.begin_version(), control.result_version(),
                control.response_bytes().map(<[u8]>::to_vec), control.outcome(),
            )).collect::<Vec<_>>();
        assert_eq!(controls.len(), 2);
        assert_eq!(controls[0].4, Some(MacroControlOutcome::Rejected));
        assert_eq!(controls[1].1, None);
        assert_eq!(controls[1].2, None);
        let first = original.global_news(GlobalNewsProvider::Eastmoney).unwrap();
        let original_native = first.final_bytes().unwrap().to_vec();
        assert_eq!(original_native, r#"{"error":{"audit_outcome":"unavailable","capability":"GrpcExternalV1","message":"ExternalV1 health 未达到 live+ready","provider":null,"reason_code":"external_health_not_ready","retryable":true},"kind":"Error","version":1}"#.as_bytes());
        let original_receipt = first.audit_receipt().unwrap().clone();
        let old_run = local.inspect_run(&baseline.intent).unwrap();
        let old_head = old_run.head_version();
        let old_generation = old_run.lease_generation();
        drop(local);
        assert_eq!(business.count("data_acquisition_audit"), audit_before + 1);
        assert_eq!(business.count("chain_post_close_macro_source_finals"), 1);
        let original_wire = external.snapshot();
        assert_eq!(original_wire.health_requests.len(), 1);
        assert_eq!(original_wire.health_responses.len(), 1);
        assert_eq!(original_wire.health_authorized, [true]);
        assert_eq!(original_wire.capabilities_calls, 0);
        assert_eq!(original_wire.data_calls, 0);
        assert_eq!(controls[0].0, original_wire.health_requests[0]);
        assert_eq!(controls[0].3.as_deref(), Some(original_wire.health_responses[0].as_slice()));
        let parent_wire = parent_server.as_ref().unwrap().snapshot();
        let parent_memberships = parent_server.as_ref().unwrap().membership_snapshot();
        external.set_reject_new_connections_for_test(true);

        // An actual closed connection and an expired owner A lease, but no fresh budget.
        business.reopen();
        assert_eq!(business.chain_post_close().migrate_schema_v11_to_v12().unwrap().schema_version(), 12);
        let resumed_at = started_at + 3_000_000;
        assert!(resumed_at > started_at + 2_000_000 && resumed_at < started_at + 15_000_000);
        let resumed_inner = MacroClock {
            now: Cell::new(UtcMicros::try_new(resumed_at).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-15T15:31:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let writer_busy_timeout: i64 = business.connection()
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0)).unwrap();
        assert_eq!(writer_busy_timeout, 250, "TEST_CODE preserve production writer timeout");
        let resumed_clock = match scenario {
            HistoricalRejectionScenario::Success => HistoricalContinuationClock::Regular(resumed_inner),
            HistoricalRejectionScenario::GroupCommitFailure => {
                HistoricalContinuationClock::GroupCommitFailure(HistoricalGroupCommitClock::new(
                    resumed_inner, baseline.intent.as_str().to_owned(), business.database(),
                    historical_fault_reader.clone(),
                ))
            }
        };
        let changed_search = macro_search_service(&[
            GeneralWebResearchProvider::Tavily, GeneralWebResearchProvider::Bocha,
            GeneralWebResearchProvider::SerpApi,
        ]);
        let mut local = business.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&baseline.config).unwrap();
        let migrated = local.inspect_macro(&baseline.intent).unwrap();
        assert_eq!(migrated.plan_bytes(), plan_bytes);
        let old_terminal = migrated.query_terminal(QueryKey::Gateway(1)).unwrap();
        let original_terminal_version = old_terminal.version();
        let original_terminal_at = old_terminal.recorded_at();
        assert_eq!(original_terminal_at, started_at + 100_000);
        assert_eq!(old_terminal.audit_receipt(), Some(&original_receipt));
        let origin = migrated.historical_rejection_origin().unwrap();
        let old_control_bytes = origin.control().bytes().to_vec();
        let old_source_bytes = origin.source().bytes().to_vec();
        let old_control_sha = fact_sha(&old_control_bytes);
        let old_source_sha = fact_sha(&old_source_bytes);
        assert_eq!(origin.control().sha256(), old_control_sha);
        assert_eq!(origin.source().sha256(), old_source_sha);
        assert_eq!(Some(origin.control().version()), controls[0].2);
        assert_eq!(origin.source().version(), original_terminal_version);
        for fact in [origin.control(), origin.source()] {
            assert_eq!(fact.owner(), "TEST_CODE_V2_REJECTED_OWNER_A");
            assert_eq!(fact.generation(), old_generation);
            assert_eq!(fact.recorded_at(), original_terminal_at);
        }
        assert_eq!(old_terminal.owner(), "TEST_CODE_V2_REJECTED_OWNER_A");
        assert_eq!(old_terminal.generation(), old_generation);
        let lease = local.resume_run(&baseline.intent, macro_lease(
            "TEST_CODE_V2_REJECTED_OWNER_B", resumed_at, started_at + 20_000_000, old_head,
        )).unwrap();
        assert!(local.inspect_run(&baseline.intent).unwrap().lease_generation() > old_generation);
        let continued_generation = lease.generation();
        let mut io = local.macro_preparation_io_v12(
            lease, &baseline.queries, &resumed_clock, FixedClusterConfiguration::resolve(Some("2")),
            &baseline.source, &macro_source, &changed_search,
        ).unwrap();
        let stopped = {
            let mut prepared = Box::pin(prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), baseline.stocks.clone(), None, &mut io,
            ));
            let paced_at = tokio::time::Instant::now();
            loop {
                tokio::select! {
                    result = &mut prepared => break result.expect_err("TEST_CODE models remain guarded"),
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {
                        resumed_clock.set_now(UtcMicros::try_new(resumed_at
                            + i64::try_from(paced_at.elapsed().as_micros()).unwrap()).unwrap());
                    }
                }
            }
        };
        if scenario == HistoricalRejectionScenario::GroupCommitFailure {
            assert!(matches!(stopped.downcast_ref::<PreparationStop>(),
                Some(PreparationStop::ResultUnconfirmed { intent_id })
                    if intent_id == baseline.intent.as_str()),
                "TEST_CODE historical six-fact COMMIT must return its public stop: {stopped:?}");
            let failure = stopped.downcast_ref::<PreparationFailure>().unwrap();
            assert_eq!(failure.stage(), PreparationStage::Macro);
            assert_eq!(failure.completed_stages().last(), Some(&PreparationStage::DragonTiger));
            assert!(!failure.completed_stages().contains(&PreparationStage::Macro));
            let fault = resumed_clock.group_commit_fault().unwrap();
            assert!(fault.armed.get(), "TEST_CODE Gateway5 must arm the real COMMIT fault");
            assert!(fault.after_arm_calls.get() > 0,
                "TEST_CODE group writer must sample after the Gateway5 lock was armed");
            let locked = fault.locked_snapshot();
            assert_eq!(locked.pending_group, (0, 0));
            assert_eq!(locked.source_final_count, 1);
            assert!(locked.finalization.iter().all(|(_, rows)| rows.is_empty()));
            assert_eq!(locked.run, (
                locked.gateway_five.version,
                "TEST_CODE_V2_REJECTED_OWNER_B".to_owned(),
                continued_generation,
                started_at + 20_000_000,
            ));
            assert_eq!(locked.gateway_five.owner, "TEST_CODE_V2_REJECTED_OWNER_B");
            assert_eq!(locked.gateway_five.generation, continued_generation);
            assert!(locked.gateway_five.recorded_at >= resumed_at);
            assert_ne!(locked.gateway_five.receipt.audit_id, original_receipt.audit_id);
            assert_eq!(
                audit_before,
                i64::try_from(baseline.audit.len())
                    .expect("TEST_CODE parent audit count fits SQLite count"),
            );
            let expected_audit_count = baseline.audit.len().checked_add(2)
                .expect("TEST_CODE parent plus G1 and G5 audit count");
            assert_eq!(locked.audits.len(), expected_audit_count);
            assert_eq!(locked.audit_chain.len(), expected_audit_count);
            assert_eq!(&locked.audits[..baseline.audit.len()], baseline.audit.as_slice());
            let mut expected_audit_ids = baseline.audit.iter().map(|row|
                row.first().cloned().expect("TEST_CODE parent audit row has id")
            ).collect::<Vec<_>>();
            expected_audit_ids.extend([
                rusqlite::types::Value::Integer(original_receipt.audit_id),
                rusqlite::types::Value::Integer(locked.gateway_five.receipt.audit_id),
            ]);
            assert_eq!(
                locked.audits.iter().map(|row| row[0].clone()).collect::<Vec<_>>(),
                expected_audit_ids,
            );
            drop(io);
            drop(local);
            close_macro_finalize_fault_reader(&historical_fault_reader)
                .expect("TEST_CODE rollback and close historical group fault reader");
            business.reopen();

            // Only a fresh connection after lock release is rollback evidence.
            let fresh = historical_group_snapshot(business.connection(), baseline.intent.as_str());
            assert_eq!(fresh, locked);
            assert_eq!(fresh.pending_group, (0, 0));
            assert_eq!(fresh.source_final_count, 1);
            assert!(fresh.finalization.iter().all(|(_, rows)| rows.is_empty()));
            let mut local = business.store.as_mut().unwrap()
                .single_user_local_chain_post_close(&baseline.config).unwrap();
            let fresh_run = local.inspect_run(&baseline.intent).unwrap();
            assert_eq!(fresh_run.head_version(), fresh.gateway_five.version);
            assert_eq!(fresh_run.lease_generation(), continued_generation);
            let recovered = local.inspect_macro(&baseline.intent).unwrap();
            assert!(!recovered.is_complete());
            assert!(!recovered.has_unconfirmed_effect());
            assert!(recovered.finalize_begin().is_none());
            assert!(recovered.stage_final().is_none());
            assert!(recovered.attempts().is_empty());
            assert_eq!(recovered.plan_bytes(), plan_bytes);
            assert_eq!(recovered.plan().started_at().get(), started_at);
            assert_eq!(recovered.plan().deadline_at().get(), started_at + 15_000_000);
            assert_eq!(recovered.plan().first_source_request().request_bytes(), original_request);
            assert_eq!(recovered.parent_final_bytes(), baseline.final_bytes);
            assert_eq!(recovered.readiness_episodes().iter().flat_map(|episode| episode.controls().iter())
                .map(|control| (
                    control.request_bytes().to_vec(), control.begin_version(), control.result_version(),
                    control.response_bytes().map(<[u8]>::to_vec), control.outcome(),
                )).collect::<Vec<_>>(), controls);
            let first = recovered.query_terminal(QueryKey::Gateway(1)).unwrap();
            assert_eq!(first.version(), original_terminal_version);
            assert_eq!(first.recorded_at(), original_terminal_at);
            assert_eq!(first.native_bytes(), original_native);
            assert_eq!(first.audit_receipt(), Some(&original_receipt));
            assert_eq!(first.owner(), "TEST_CODE_V2_REJECTED_OWNER_A");
            assert_eq!(first.generation(), old_generation);
            let origin = recovered.historical_rejection_origin().unwrap();
            assert_eq!(origin.control().bytes(), old_control_bytes);
            assert_eq!(origin.source().bytes(), old_source_bytes);
            assert_eq!(origin.control().sha256(), old_control_sha);
            assert_eq!(origin.source().sha256(), old_source_sha);
            assert_eq!(origin.control().version(), controls[0].2.unwrap());
            assert_eq!(origin.source().version(), original_terminal_version);
            for fact in [origin.control(), origin.source()] {
                assert_eq!(fact.owner(), "TEST_CODE_V2_REJECTED_OWNER_A");
                assert_eq!(fact.generation(), old_generation);
                assert_eq!(fact.recorded_at(), original_terminal_at);
            }
            let gateway_five = recovered.query_terminal(QueryKey::Gateway(5)).unwrap();
            assert_eq!(gateway_five.version(), fresh.gateway_five.version);
            assert_eq!(gateway_five.native_bytes(), fresh.gateway_five.native_bytes);
            assert_eq!(gateway_five.audit_receipt(), Some(&fresh.gateway_five.receipt));
            assert_eq!(gateway_five.owner(), "TEST_CODE_V2_REJECTED_OWNER_B");
            assert_eq!(gateway_five.generation(), continued_generation);
            assert_eq!(gateway_five.recorded_at(), fresh.gateway_five.recorded_at);
            match gateway_five.native() {
                NativeOutcome::Economic(Err(error)) => {
                    assert_eq!(error.capability(), "GrpcBridge");
                    assert_eq!(error.reason_code(), "no_verified_batch");
                    assert!(!error.retryable());
                    assert_eq!(error.message(),
                        "explicit Macro Local transport was not connected at the original observation");
                    assert_eq!(error.provider(), None);
                    assert_eq!(error.audit_outcome(), "unavailable");
                }
                other => panic!("TEST_CODE expected committed Gateway5 Local unavailable: {other:?}"),
            }
            assert_eq!(resumed_clock.inner().observation_calls.get(), 0);
            drop(local);
            assert_eq!(external.snapshot(), original_wire);
            assert_eq!(parent_server.as_ref().unwrap().snapshot(), parent_wire);
            assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), parent_memberships);
            return;
        }
        // This actual stage assertion is the expected behavior RED; preserve the
        // original return type and full network observation instead of timing out.
        assert!(
            matches!(stopped.downcast_ref::<PreparationStop>(),
                Some(PreparationStop::StageNotMigrated { next: UnmigratedStage::ModelsSearchAndReport })),
            "TEST_CODE real v2 continuation failed: {stopped:?}; original={original_wire:?}; current={:?}",
            external.snapshot()
        );
        assert_final_stop(&stopped);
        drop(io);
        let completed = local.inspect_macro(&baseline.intent).unwrap();
        assert!(completed.is_complete());
        assert!(!completed.has_unconfirmed_effect());
        assert!(completed.attempts().is_empty(), "TEST_CODE NotCalled never fabricates a Data begin");
        assert_eq!(completed.plan_bytes(), plan_bytes);
        assert_eq!(completed.plan().first_source_request().request_bytes(), original_request);
        assert_eq!(completed.parent_final_bytes(), baseline.final_bytes);
        assert_eq!(completed.readiness_episodes().iter().flat_map(|episode| episode.controls().iter())
            .map(|control| (
                control.request_bytes().to_vec(), control.begin_version(), control.result_version(),
                control.response_bytes().map(<[u8]>::to_vec), control.outcome(),
            )).collect::<Vec<_>>(), controls);
        let first = completed.query_terminal(QueryKey::Gateway(1)).unwrap();
        assert_eq!(first.version(), original_terminal_version);
        assert_eq!(first.recorded_at(), original_terminal_at);
        assert_eq!(first.native_bytes(), original_native);
        assert_eq!(first.audit_receipt(), Some(&original_receipt));
        assert_eq!(first.owner(), "TEST_CODE_V2_REJECTED_OWNER_A");
        assert_eq!(first.generation(), old_generation);
        let origin = completed.historical_rejection_origin().unwrap();
        assert_eq!(origin.control().bytes(), old_control_bytes);
        assert_eq!(origin.source().bytes(), old_source_bytes);
        let group_base = completed.query_terminal(QueryKey::Gateway(5)).unwrap().version();
        let group_time = completed.query_terminal(QueryKey::Gateway(2)).unwrap().recorded_at();
        let mut group_evidence = Vec::new();
        for (ordinal, request_offset, terminal_offset) in [(2, 1, 4), (3, 2, 5), (4, 3, 6)] {
            let terminal = completed.query_terminal(QueryKey::Gateway(ordinal)).unwrap();
            assert_eq!(terminal.request_version(), Some(group_base + request_offset));
            assert_eq!(terminal.version(), group_base + terminal_offset);
            assert_eq!(terminal.owner(), "TEST_CODE_V2_REJECTED_OWNER_B");
            assert_eq!(terminal.generation(), continued_generation);
            assert_eq!(terminal.recorded_at(), group_time);
            assert!(group_time >= resumed_at && group_time > original_terminal_at);
            let link = terminal.historical_rejection().unwrap();
            assert_eq!(Some(link.control_result_version), controls[0].2);
            assert_eq!(link.control_result_sha256, old_control_sha);
            assert_eq!(link.source_final_version, original_terminal_version);
            assert_eq!(link.source_final_sha256, old_source_sha);
            group_evidence.push((ordinal, terminal.request_version(), terminal.version(), terminal.recorded_at()));
        }
        let mut receipt_ids = std::collections::BTreeSet::new();
        let mut native = Vec::new();
        for ordinal in 1..=5 {
            let terminal = completed.query_terminal(QueryKey::Gateway(ordinal)).unwrap();
            assert!(!terminal.was_called());
            assert!(receipt_ids.insert(terminal.audit_receipt().unwrap().audit_id));
            if ordinal > 1 {
                assert!(terminal.recorded_at() >= resumed_at);
                assert!(terminal.recorded_at() > original_terminal_at);
            }
            let error = match terminal.native() {
                NativeOutcome::News(Err(error)) if ordinal <= 4 => {
                    assert_eq!(error.capability(), "GrpcExternalV1");
                    assert_eq!(error.reason_code(), "external_health_not_ready");
                    assert!(error.retryable());
                    assert_eq!(error.message(), "ExternalV1 health 未达到 live+ready");
                    error
                }
                NativeOutcome::Economic(Err(error)) if ordinal == 5 => {
                    assert_eq!(error.capability(), "GrpcBridge");
                    assert_eq!(error.reason_code(), "no_verified_batch");
                    assert!(!error.retryable());
                    assert_eq!(error.message(),
                        "explicit Macro Local transport was not connected at the original observation");
                    error
                }
                other => panic!("TEST_CODE expected native NotCalled error for {ordinal}: {other:?}"),
            };
            assert_eq!(error.provider(), None);
            assert_eq!(error.audit_outcome(), "unavailable");
            native.push((terminal.native_bytes().to_vec(), terminal.audit_receipt().unwrap().clone()));
        }
        assert_eq!(receipt_ids.len(), 5);
        let final_ = completed.stage_final().unwrap();
        assert_eq!(completed.finalize_begin().unwrap().expiry(), &ExpiryBasis::None);
        assert_eq!(final_.output_bytes(), EXPECTED_V2_REJECTED_MACRO.as_bytes());
        assert_eq!(final_.kind(), crate::push_foundation::intent_store::chain_post_close::macro_native::FinalKind::Complete);
        assert_eq!(final_.started_at(), started_at);
        assert_eq!(final_.deadline_at(), started_at + 15_000_000);
        assert_eq!(final_.plan_version(), completed.plan_version());
        assert_eq!(final_.version(), final_.finalize_begin_version() + 1);
        let final_bytes = final_.bytes().to_vec();
        let begin_bytes = final_.finalize_begin_bytes().to_vec();
        let complete_head = local.inspect_run(&baseline.intent).unwrap().head_version();
        assert_eq!(resumed_clock.inner().observation_calls.get(), 0);
        drop(local);
        assert_eq!(business.count("data_acquisition_audit"), audit_before + 5);
        assert_eq!(business.count("chain_post_close_macro_source_finals"), 1);
        assert_eq!(external.snapshot(), original_wire);
        assert_eq!(parent_server.as_ref().unwrap().snapshot(), parent_wire);
        assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), parent_memberships);

        business.reopen();
        let final_clock = MacroClock {
            now: Cell::new(UtcMicros::try_new(started_at + 25_000_000).unwrap()),
            observation: DateTime::parse_from_rfc3339("2026-09-16T15:31:00+08:00").unwrap(),
            observation_calls: Cell::new(0),
        };
        let empty_current_registry = macro_search_service(&[]);
        let mut local = business.store.as_mut().unwrap()
            .single_user_local_chain_post_close(&baseline.config).unwrap();
        let lease = local.resume_run(&baseline.intent, macro_lease(
            "TEST_CODE_V2_REJECTED_OWNER_C", started_at + 25_000_000,
            started_at + 60_000_000, complete_head,
        )).unwrap();
        let mut io = local.macro_preparation_io_v12(
            lease, &baseline.queries, &final_clock, FixedClusterConfiguration::resolve(Some("2")),
            &baseline.source, &macro_source, &empty_current_registry,
        ).unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(), baseline.stocks.clone(), None, &mut io,
        ).await.expect_err("TEST_CODE reopened final remains before models");
        assert_final_stop(&stopped);
        drop(io);
        let reopened = local.inspect_macro(&baseline.intent).unwrap();
        let origin = reopened.historical_rejection_origin().unwrap();
        assert_eq!(origin.control().bytes(), old_control_bytes);
        assert_eq!(origin.source().bytes(), old_source_bytes);
        assert_eq!(origin.control().sha256(), old_control_sha);
        assert_eq!(origin.source().sha256(), old_source_sha);
        for (ordinal, request_version, version, time) in group_evidence {
            let terminal = reopened.query_terminal(QueryKey::Gateway(ordinal)).unwrap();
            assert_eq!(terminal.request_version(), request_version);
            assert_eq!(terminal.version(), version);
            assert_eq!(terminal.recorded_at(), time);
            assert_eq!(terminal.owner(), "TEST_CODE_V2_REJECTED_OWNER_B");
            assert_eq!(terminal.generation(), continued_generation);
            let link = terminal.historical_rejection().unwrap();
            assert_eq!(Some(link.control_result_version), controls[0].2);
            assert_eq!(link.control_result_sha256, old_control_sha);
            assert_eq!(link.source_final_version, original_terminal_version);
            assert_eq!(link.source_final_sha256, old_source_sha);
        }
        assert_eq!(reopened.plan_bytes(), plan_bytes);
        assert_eq!(reopened.plan().first_source_request().request_bytes(), original_request);
        assert!(reopened.attempts().is_empty());
        assert!(!reopened.has_unconfirmed_effect());
        assert_eq!(reopened.stage_final().unwrap().bytes(), final_bytes);
        assert_eq!(reopened.stage_final().unwrap().finalize_begin_bytes(), begin_bytes);
        assert_eq!(reopened.stage_final().unwrap().output_bytes(), EXPECTED_V2_REJECTED_MACRO.as_bytes());
        assert_eq!(reopened.readiness_episodes().iter().flat_map(|episode| episode.controls().iter())
            .map(|control| (
                control.request_bytes().to_vec(), control.begin_version(), control.result_version(),
                control.response_bytes().map(<[u8]>::to_vec), control.outcome(),
            )).collect::<Vec<_>>(), controls);
        for (index, (bytes, receipt)) in native.iter().enumerate() {
            let terminal = reopened.query_terminal(QueryKey::Gateway(index as u8 + 1)).unwrap();
            assert_eq!(terminal.native_bytes(), bytes);
            assert_eq!(terminal.audit_receipt(), Some(receipt));
        }
        assert_eq!(final_clock.observation_calls.get(), 0);
        drop(local);
        assert_eq!(business.count("data_acquisition_audit"), audit_before + 5);
        assert_eq!(business.count("chain_post_close_macro_source_finals"), 1);
        assert_eq!(external.snapshot(), original_wire);
        assert_eq!(parent_server.as_ref().unwrap().snapshot(), parent_wire);
        assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), parent_memberships);
    })).catch_unwind().await;

    // These owners are outside the caught body. Both finishes run even if either
    // one panics, and each fixture handles its own abort+join deadline.
    let reader_cleanup = close_macro_finalize_fault_reader(&historical_fault_reader);
    let external_cleanup = match external_server.take() {
        Some(server) => std::panic::AssertUnwindSafe(server.finish()).catch_unwind().await,
        None => Ok(Ok(())),
    };
    let parent_cleanup = match parent_server.take() {
        Some(server) => std::panic::AssertUnwindSafe(server.finish()).catch_unwind().await.map(|_| ()),
        None => Ok(()),
    };
    let database_cleanup = business.store.take().map(|store| {
        store.connection.close().map_err(|(connection, error)| { drop(connection); error })
    });
    drop(business);
    reader_cleanup.expect("TEST_CODE historical group fault reader cleanup");
    external_cleanup.expect("TEST_CODE External finish panic").expect("TEST_CODE External finish");
    parent_cleanup.expect("TEST_CODE parent finish panic after join");
    if let Some(result) = database_cleanup { result.expect("TEST_CODE database close"); }
    match body {
        Ok(result) => result.expect("TEST_CODE v2 rejected fixture watchdog"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[path = "chain_post_close_macro_pace_tests.rs"]
mod pace_tests;

#[path = "chain_post_close_macro_timeout_tests.rs"]
mod timeout_tests;
