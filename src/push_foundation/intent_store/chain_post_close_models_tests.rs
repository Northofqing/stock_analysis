//! V13 vertical slice: the full post-close chain runs to a committed
//! Models/Search/Report artifact, a reopen replays every journaled effect with
//! zero remote calls, and a begun-unconfirmed model call fails closed on reopen.
use super::*;
use crate::grpc_client::client::macro_full_loopback_fixture::MacroFullLoopbackServer;
use crate::pipeline::chain_analysis::preparation::{
    ModelStage, ModelsObservationClock, PreparationStage,
};

impl ModelsObservationClock for MacroClock {
    fn models_local_observation(&self) -> DateTime<chrono::FixedOffset> {
        self.observation
    }
}

/// Wall clock anchored at a fixed instant that advances with real time, so the
/// Macro runner's due/deadline scheduling progresses without test-side nudging.
struct LiveClock {
    base: i64,
    origin: std::time::Instant,
    observation: DateTime<chrono::FixedOffset>,
}

impl LiveClock {
    fn new(base: i64, observation: &str) -> Self {
        Self {
            base,
            origin: std::time::Instant::now(),
            observation: DateTime::parse_from_rfc3339(observation).unwrap(),
        }
    }
}

impl ConceptEffectClock for LiveClock {
    fn now(&self) -> UtcMicros {
        let elapsed = i64::try_from(self.origin.elapsed().as_micros()).unwrap();
        UtcMicros::try_new(self.base + elapsed).unwrap()
    }
}

impl PositionObservationClock for LiveClock {
    fn cache_observation(&self) -> PositionCacheObservation {
        panic!("TEST_CODE v13 must recover the sealed position parent")
    }
}

impl DragonTigerObservationClock for LiveClock {
    fn dragon_tiger_request_observation(&self) -> DateTime<chrono::FixedOffset> {
        self.observation
    }
}

impl MacroObservationClock for LiveClock {
    fn macro_request_observation(&self) -> DateTime<chrono::FixedOffset> {
        self.observation
    }
}

impl ModelsObservationClock for LiveClock {
    fn models_local_observation(&self) -> DateTime<chrono::FixedOffset> {
        self.observation
    }
}

const OVERVIEW_TEXT: &str = "### 核心矛盾与主线优先级\n TEST_CODE_V13_总览原响应 Ω\t \n";

fn loopback_analyzer(server_base_url: &str) -> crate::analyzer::GeminiAnalyzer {
    crate::analyzer::GeminiAnalyzer::with_loopback_client(crate::analyzer::GeminiConfig {
        doubao_api_key: Some("TEST_CODE_V13_LOCAL_KEY".into()),
        doubao_base_url: Some(server_base_url.to_owned()),
        doubao_model: "TEST_CODE_V13_MODEL".into(),
        max_retries: 1,
        retry_delay: 0.0,
        request_delay: 0.0,
        agent_pipeline: false,
        ..crate::analyzer::GeminiConfig::default()
    })
}

fn model_response(text: &str) -> String {
    serde_json::json!({"choices":[{"message":{"content":text}}]}).to_string()
}

/// Loopback chat-completions responder without an arrival deadline: the Macro
/// stage runs for several seconds before the first model request is issued.
struct ModelServer {
    base_url: String,
    requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ModelServer {
    fn new(mut responses: Vec<String>) -> Self {
        use std::io::{Read, Write};
        use std::sync::atomic::Ordering;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (requests_thread, stop_thread) = (requests.clone(), stop.clone());
        responses.reverse();
        let thread = std::thread::spawn(move || loop {
            if stop_thread.load(Ordering::SeqCst) {
                break;
            }
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(error) => panic!("TEST_CODE model server accept failed: {error}"),
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut raw = Vec::new();
            let mut chunk = [0_u8; 4096];
            let header_end = loop {
                let count = stream.read(&mut chunk).unwrap();
                if count == 0 {
                    break None;
                }
                raw.extend_from_slice(&chunk[..count]);
                if let Some(index) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
                    break Some(index + 4);
                }
            };
            let Some(header_end) = header_end else {
                continue;
            };
            let header = String::from_utf8_lossy(&raw[..header_end]).into_owned();
            let path = header
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("")
                .to_owned();
            let content_length = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while raw.len() < header_end + content_length {
                let count = stream.read(&mut chunk).unwrap();
                if count == 0 {
                    break;
                }
                raw.extend_from_slice(&chunk[..count]);
            }
            requests_thread.lock().unwrap().push(path);
            let (status, body) = match responses.pop() {
                Some(body) => ("200 OK", body),
                None => ("503 Service Unavailable", String::new()),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });
        Self {
            base_url,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn finish(mut self) -> Vec<String> {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
        self.requests.lock().unwrap().clone()
    }
}

/// Same owner-join protocol as the Macro full tests: never drop a listener
/// without joining its finish, and close the owned database last.
async fn finish_servers(
    fixture: &mut V2BusinessFixture,
    macro_server: Option<MacroFullLoopbackServer>,
    parent_server: Option<crate::grpc_client::client::board_loopback_fixture::BoardLoopbackServer>,
) {
    let macro_cleanup = match macro_server {
        Some(server) => {
            std::panic::AssertUnwindSafe(server.finish())
                .catch_unwind()
                .await
        }
        None => Ok(Ok(())),
    };
    let parent_cleanup = match parent_server {
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
    macro_cleanup
        .expect("TEST_CODE v13 macro cleanup panic")
        .expect("TEST_CODE v13 macro cleanup");
    parent_cleanup.expect("TEST_CODE v13 parent cleanup after join");
    if let Some(result) = database_cleanup {
        result.expect("TEST_CODE v13 database close");
    }
}

fn count(connection: &rusqlite::Connection, table: &str, intent: &IntentId) -> i64 {
    connection
        .query_row(
            &format!("SELECT count(*) FROM {table} WHERE intent_id=?1"),
            [intent.as_str()],
            |row| row.get(0),
        )
        .unwrap()
}

/// Brings an owned business store to a completed v9 parent + v10 DragonTiger,
/// then migrates to v13. Returns everything a v13 prepare needs.
async fn v13_parent(
    fixture: &mut V2BusinessFixture,
    parent_server: &mut Option<
        crate::grpc_client::client::board_loopback_fixture::BoardLoopbackServer,
    >,
    run_id: &str,
) -> (
    Vec<TopStock>,
    LocalChainPostCloseConfig,
    IntentId,
    u64,
    GrpcSource,
    crate::data_gateway::grpc_source::ConnectedBoardQueries,
) {
    let (parent_endpoint, server) = tokio::time::timeout(
        Duration::from_secs(5),
        spawn_macro_parent_listener(DRAGON_TIGER_RECORDS.as_bytes()),
    )
    .await
    .expect("TEST_CODE parent listener deadline");
    *parent_server = Some(server);
    let parent_source = GrpcSource::from_board_loopback_test_client(
        connect_parent_instance(&parent_endpoint).await,
    );
    let queries = parent_source.connected_board_queries().await.unwrap();
    let (stocks, config, intent, v9_head) =
        populate_completed_v9_parent(fixture, &queries, run_id).await;
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
                "TEST_CODE_V13_PARENT_OWNER",
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
    .expect_err("TEST_CODE real parent stops at Macro");
    assert_partial_macro_stop(&stopped);
    drop(io);
    let parent_head = local.inspect_run(&intent).unwrap().head_version();
    drop(local);
    fixture
        .chain_post_close()
        .migrate_schema_v10_to_v11()
        .unwrap();
    fixture
        .chain_post_close()
        .migrate_schema_v11_to_v12()
        .unwrap();
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v12_to_v13()
            .unwrap()
            .schema_version(),
        13
    );
    (stocks, config, intent, parent_head, parent_source, queries)
}

#[tokio::test]
async fn single_user_v13_prepare_completes_models_and_reopen_replays_without_remote_calls() {
    let mut fixture = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut macro_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), async {
        let (stocks, config, intent, parent_head, parent_source, queries) =
            v13_parent(&mut fixture, &mut parent_server, "TEST_CODE_RUN_V13_MODELS").await;
        macro_server = Some(MacroFullLoopbackServer::bind().await);
        let server = macro_server.as_ref().unwrap();
        server.release_all();
        let macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await,
            server.endpoint().to_owned(),
        );
        let started_at = micros("2026-09-14T15:30:00+08:00");
        let clock = LiveClock::new(started_at, "2026-09-14T15:30:00+08:00");
        let search_service = macro_search_service(&[]);
        // Cluster A has 2 stocks (below Tier-2), so the only remote model call
        // is the Overview; search is unavailable in this fixture.
        let model_server = ModelServer::new(vec![model_response(OVERVIEW_TEXT)]);
        let analyzer = loopback_analyzer(model_server.base_url());
        let database = fixture.database();
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
                    "TEST_CODE_V13_OWNER",
                    started_at,
                    started_at + 60_000_000,
                    parent_head,
                ),
            )
            .unwrap();
        let mut io = local
            .models_preparation_io_v13(
                lease,
                &queries,
                &clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &parent_source,
                &macro_source,
                &search_service,
                &analyzer,
            )
            .unwrap();
        let prepared = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect("TEST_CODE v13 prepare completes the whole chain");
        drop(io);
        assert_eq!(
            prepared.completed_stages().last(),
            Some(&PreparationStage::ModelsSearchAndReport)
        );
        let calls = prepared.model_calls();
        let overview = calls
            .iter()
            .find(|call| call.stage() == ModelStage::Overview)
            .expect("TEST_CODE overview model call observed");
        assert_eq!(
            overview.response(),
            Some(OVERVIEW_TEXT),
            "TEST_CODE overview not returned; failure={:?} not_called={:?} calls={calls:?} clusters={}",
            overview.failure(),
            overview.not_called_reason(),
            prepared.clusters().len()
        );
        assert!(prepared.report().contains("TEST_CODE_V13_总览原响应 Ω"));
        assert!(!prepared.macro_context().is_empty());
        let first_artifact = prepared.to_artifact_bytes().unwrap();
        let first_report = prepared.report().to_owned();
        let first_head = local.inspect_run(&intent).unwrap().head_version();
        drop(local);
        let paths = model_server.finish();
        assert_eq!(paths, vec!["/chat/completions".to_owned()]);
        let macro_calls_after_first = server.snapshot().calls.len();

        // Journal shape: ModelAvailable, SearchAvailable, Overview → 3 effects,
        // all confirmed, one sealed final.
        {
            let reader = rusqlite::Connection::open(&database).unwrap();
            assert_eq!(count(&reader, "chain_post_close_models_effect_begins", &intent), 3);
            assert_eq!(count(&reader, "chain_post_close_models_effect_results", &intent), 3);
            assert_eq!(count(&reader, "chain_post_close_models_stage_finals", &intent), 1);
            let kinds: Vec<String> = reader
                .prepare(
                    "SELECT effect_kind FROM chain_post_close_models_effect_begins \
                     WHERE intent_id=?1 ORDER BY effect_ordinal",
                )
                .unwrap()
                .query_map([intent.as_str()], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(kinds, ["ModelAvailable", "SearchAvailable", "Model"]);
            let (artifact, report): (Vec<u8>, Vec<u8>) = reader
                .query_row(
                    "SELECT artifact_bytes,report_bytes FROM chain_post_close_models_stage_finals \
                     WHERE intent_id=?1",
                    [intent.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(artifact, first_artifact);
            assert_eq!(report, first_report.as_bytes());
        }

        // True reopen: a new store handle, a new lease, a model endpoint that
        // must never be hit. Macro and every models effect replay from facts.
        let dead_model_server = ModelServer::new(Vec::new());
        let reopened_analyzer = loopback_analyzer(dead_model_server.base_url());
        let reopen_clock = LiveClock::new(started_at + 86_400_000_000, "2026-09-15T15:30:00+08:00");
        let mut reopened_store = BusinessIntentStore::open(&database).unwrap();
        let mut local = reopened_store
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                macro_lease(
                    "TEST_CODE_V13_REOPEN_OWNER",
                    started_at + 86_400_000_000,
                    started_at + 86_460_000_000,
                    first_head,
                ),
            )
            .unwrap();
        let mut io = local
            .models_preparation_io_v13(
                lease,
                &queries,
                &reopen_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &parent_source,
                &macro_source,
                &search_service,
                &reopened_analyzer,
            )
            .unwrap();
        let replayed = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect("TEST_CODE v13 reopen replays to the same artifact");
        drop(io);
        let replayed_artifact = replayed.to_artifact_bytes().unwrap();
        if replayed_artifact != first_artifact {
            let first = String::from_utf8_lossy(&first_artifact);
            let second = String::from_utf8_lossy(&replayed_artifact);
            let divergence = first
                .char_indices()
                .zip(second.chars())
                .find(|((_, a), b)| a != b)
                .map(|((index, _), _)| index)
                .unwrap_or(first.len().min(second.len()));
            let window = |text: &str| {
                text.chars()
                    .skip(divergence.saturating_sub(200))
                    .take(600)
                    .collect::<String>()
            };
            panic!(
                "TEST_CODE replay artifact diverges at byte {divergence}\nfirst:  {}\nreplay: {}",
                window(&first),
                window(&second)
            );
        }
        assert_eq!(replayed.report(), first_report);
        assert_eq!(dead_model_server.finish(), Vec::<String>::new());
        assert_eq!(server.snapshot().calls.len(), macro_calls_after_first);
        let reopened_head = local.inspect_run(&intent).unwrap().head_version();
        // Lease reissue bumps the head once; no new fact rows were written.
        assert_eq!(reopened_head, first_head + 1);
        {
            let reader = rusqlite::Connection::open(&database).unwrap();
            assert_eq!(count(&reader, "chain_post_close_models_effect_begins", &intent), 3);
            assert_eq!(count(&reader, "chain_post_close_models_effect_results", &intent), 3);
            assert_eq!(count(&reader, "chain_post_close_models_stage_finals", &intent), 1);
        }
    }))
    .catch_unwind()
    .await;
    finish_servers(&mut fixture, macro_server, parent_server).await;
    match body {
        Ok(result) => result.expect("TEST_CODE v13 scenario deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn single_user_v13_model_call_dropped_in_flight_fails_closed_on_reopen() {
    let mut fixture = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut macro_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), async {
        let (stocks, config, intent, parent_head, parent_source, queries) =
            v13_parent(&mut fixture, &mut parent_server, "TEST_CODE_RUN_V13_INCOMPLETE").await;
        macro_server = Some(MacroFullLoopbackServer::bind().await);
        let server = macro_server.as_ref().unwrap();
        server.release_all();
        let macro_source = GrpcSource::from_macro_loopback_test_client(
            server.connect().await,
            server.endpoint().to_owned(),
        );
        let started_at = micros("2026-09-14T15:30:00+08:00");
        let clock = LiveClock::new(started_at, "2026-09-14T15:30:00+08:00");
        let search_service = macro_search_service(&[]);
        // A listener that accepts nothing: the model request connects but never
        // gets a response, so the prepare is dropped mid-effect.
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let silent_url = format!("http://{}", silent.local_addr().unwrap());
        let analyzer = loopback_analyzer(&silent_url);
        let database = fixture.database();
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
                    "TEST_CODE_V13_DROP_OWNER",
                    started_at,
                    started_at + 60_000_000,
                    parent_head,
                ),
            )
            .unwrap();
        let mut io = local
            .models_preparation_io_v13(
                lease,
                &queries,
                &clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &parent_source,
                &macro_source,
                &search_service,
                &analyzer,
            )
            .unwrap();
        let dropped = {
            let prepared = prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                stocks.clone(),
                None,
                &mut io,
            );
            tokio::pin!(prepared);
            // Drop the prepare while the Overview request is in flight: its
            // begin row is committed, its result row is not.
            let reader = rusqlite::Connection::open(&database).unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
            loop {
                tokio::select! {
                    result = &mut prepared => {
                        panic!("TEST_CODE prepare must hang on the silent model endpoint; returned {result:?}");
                    }
                    _ = tokio::time::sleep(Duration::from_millis(5)) => {}
                }
                if count(&reader, "chain_post_close_models_effect_begins", &intent) == 3
                    && count(&reader, "chain_post_close_models_effect_results", &intent) == 2
                {
                    break;
                }
                assert!(tokio::time::Instant::now() < deadline, "TEST_CODE overview begin never journaled");
            }
        };
        drop(dropped);
        drop(io);
        drop(silent);
        let first_head = local.inspect_run(&intent).unwrap().head_version();
        drop(local);
        {
            let reader = rusqlite::Connection::open(&database).unwrap();
            assert_eq!(count(&reader, "chain_post_close_models_effect_begins", &intent), 3);
            assert_eq!(count(&reader, "chain_post_close_models_effect_results", &intent), 2);
            assert_eq!(count(&reader, "chain_post_close_models_stage_finals", &intent), 0);
        }

        let dead_model_server = ModelServer::new(Vec::new());
        let reopened_analyzer = loopback_analyzer(dead_model_server.base_url());
        let reopen_clock = LiveClock::new(started_at + 86_400_000_000, "2026-09-15T15:30:00+08:00");
        let mut reopened_store = BusinessIntentStore::open(&database).unwrap();
        let mut local = reopened_store
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                macro_lease(
                    "TEST_CODE_V13_DROP_REOPEN_OWNER",
                    started_at + 86_400_000_000,
                    started_at + 86_460_000_000,
                    first_head,
                ),
            )
            .unwrap();
        let mut io = local
            .models_preparation_io_v13(
                lease,
                &queries,
                &reopen_clock,
                FixedClusterConfiguration::resolve(Some("2")),
                &parent_source,
                &macro_source,
                &search_service,
                &reopened_analyzer,
            )
            .unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE begun-unconfirmed model call must fail closed");
        drop(io);
        assert!(
            matches!(
                stopped.downcast_ref::<PreparationStop>(),
                Some(PreparationStop::IncompleteOnReopen { intent_id }) if intent_id == intent.as_str()
            ),
            "TEST_CODE expected IncompleteOnReopen; received {stopped:?}"
        );
        let failure = stopped.downcast_ref::<PreparationFailure>().unwrap();
        assert_eq!(failure.stage(), PreparationStage::ModelsSearchAndReport);
        assert_eq!(dead_model_server.finish(), Vec::<String>::new());
        {
            let reader = rusqlite::Connection::open(&database).unwrap();
            assert_eq!(count(&reader, "chain_post_close_models_effect_begins", &intent), 3);
            assert_eq!(count(&reader, "chain_post_close_models_effect_results", &intent), 2);
        }
    }))
    .catch_unwind()
    .await;
    finish_servers(&mut fixture, macro_server, parent_server).await;
    match body {
        Ok(result) => result.expect("TEST_CODE v13 scenario deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
