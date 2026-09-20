use std::cell::RefCell;
use std::collections::HashMap;
use std::future::{poll_fn, Future};
use std::rc::Rc;
use std::task::{Poll, Waker};

use super::*;
use crate::pipeline::chain_analysis::preparation::PreparationStage;

const MISSING_1_RAW: &str = "{\"all_boards\":[\"TEST_CODE_CONCEPT_1\"]}";
const MISSING_2_RAW: &str =
    "{\"all_boards\":[\"TEST_CODE_CONCEPT_2_A\",\"TEST_CODE_CONCEPT_2_B\"]}";

struct BatchRawProvider {
    database: PathBuf,
    calls: RefCell<Vec<String>>,
    panic_if_called: bool,
}

impl BatchRawProvider {
    fn returning(database: PathBuf) -> Self {
        Self {
            database,
            calls: RefCell::new(Vec::new()),
            panic_if_called: false,
        }
    }

    fn panic_if_called(database: PathBuf) -> Self {
        Self {
            database,
            calls: RefCell::new(Vec::new()),
            panic_if_called: true,
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

#[async_trait::async_trait(?Send)]
impl ConceptProviderRawIo for BatchRawProvider {
    async fn call_raw(&self, code: &str) -> std::result::Result<String, String> {
        assert!(!self.panic_if_called, "recovery must not call provider");
        self.calls.borrow_mut().push(code.to_owned());

        let connection = Connection::open_with_flags(
            &self.database,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        let ordinal: i64 = connection
            .query_row(
                "SELECT effect_ordinal FROM chain_post_close_stage_begins \
                 WHERE effect_kind='ConceptProvider' AND effect_key=?1",
                [code],
                |row| row.get(0),
            )
            .expect("each provider call must observe its committed begin");
        let expected_ordinal = match code {
            "TEST_CODE_MISSING_1" => 0,
            "TEST_CODE_MISSING_2" => 1,
            _ => panic!("unexpected provider code: {code}"),
        };
        assert_eq!(ordinal, expected_ordinal);
        connection.close().unwrap();

        Ok(match code {
            "TEST_CODE_MISSING_1" => MISSING_1_RAW,
            "TEST_CODE_MISSING_2" => MISSING_2_RAW,
            _ => unreachable!(),
        }
        .to_owned())
    }
}

async fn run_public_batch_prepare<P, C>(
    local: &mut super::super::LocalChainPostClose<'_>,
    lease: super::super::RunLease,
    provider: &P,
    clock: &C,
    input_stocks: Vec<TopStock>,
) -> anyhow::Error
where
    P: ConceptProviderRawIo,
    C: ConceptEffectClock,
{
    let mut io = local
        .concept_batch_preparation_io(lease, provider, clock)
        .unwrap();
    prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        input_stocks,
        None,
        &mut io,
    )
    .await
    .expect_err("local preparation must stop before unfixed cluster configuration")
}

fn stored_raw_results(fixture: &V2BusinessFixture) -> Vec<(i64, String, Vec<u8>)> {
    fixture
        .connection()
        .prepare(
            "SELECT effect_ordinal,outcome,CAST(result_bytes AS BLOB) \
             FROM chain_post_close_stage_results ORDER BY effect_ordinal",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn stored_cache_rows(fixture: &V2BusinessFixture) -> Vec<(String, String)> {
    fixture
        .connection()
        .prepare(
            "SELECT code,concepts FROM stock_concepts \
             WHERE code GLOB 'TEST_CODE_*' ORDER BY code",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn stored_cache_rows_with_time(fixture: &V2BusinessFixture) -> Vec<(String, String, String)> {
    fixture
        .connection()
        .prepare(
            "SELECT code,concepts,updated_at FROM stock_concepts \
             WHERE code GLOB 'TEST_CODE_*' ORDER BY code",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[tokio::test]
async fn single_user_local_complete_concept_batch_reopens_without_provider_or_cache_rewrite() {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v2_to_v3()
            .unwrap()
            .schema_version(),
        3
    );
    seed_cache(&fixture);

    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_COMPLETE_CONCEPT_BATCH"),
    )
    .unwrap();
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            context,
            fixed_input(stocks()),
            lease_request("TEST_CODE_BATCH_OWNER_A", 1_000, 5_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let provider = BatchRawProvider::returning(database);
    let clock = ControlledClock::new(at(1_100));
    let error = run_public_batch_prepare(&mut local, lease, &provider, &clock, stocks()).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::ClusterConfiguration,
        })
    ));
    assert_eq!(
        provider.calls(),
        vec!["TEST_CODE_MISSING_1", "TEST_CODE_MISSING_2"]
    );

    let expected = HashMap::from([
        (
            "TEST_CODE_HIT".to_owned(),
            vec!["TEST_CODE_CACHED".to_owned()],
        ),
        (
            "TEST_CODE_MISSING_1".to_owned(),
            vec!["TEST_CODE_CONCEPT_1".to_owned()],
        ),
        (
            "TEST_CODE_MISSING_2".to_owned(),
            vec![
                "TEST_CODE_CONCEPT_2_A".to_owned(),
                "TEST_CODE_CONCEPT_2_B".to_owned(),
            ],
        ),
    ]);
    let inspection = local.inspect_concept_batch(&intent_id).unwrap();
    assert_eq!(inspection.concepts(), &expected);
    assert_eq!(
        inspection.applied_codes(),
        ["TEST_CODE_MISSING_1", "TEST_CODE_MISSING_2"]
    );
    drop(local);

    assert_eq!(
        stored_raw_results(&fixture),
        vec![
            (0, "Returned".to_owned(), MISSING_1_RAW.as_bytes().to_vec()),
            (1, "Returned".to_owned(), MISSING_2_RAW.as_bytes().to_vec()),
        ]
    );
    assert_eq!(fixture.count("chain_post_close_concept_cache_writes"), 2);
    assert_eq!(
        stored_cache_rows(&fixture),
        vec![
            (
                "TEST_CODE_EXPIRED".to_owned(),
                "[\"TEST_CODE_OLD\"]".to_owned(),
            ),
            (
                "TEST_CODE_HIT".to_owned(),
                "[\"TEST_CODE_CACHED\"]".to_owned(),
            ),
            (
                "TEST_CODE_MISSING_1".to_owned(),
                "[\"TEST_CODE_CONCEPT_1\"]".to_owned(),
            ),
            (
                "TEST_CODE_MISSING_2".to_owned(),
                "[\"TEST_CODE_CONCEPT_2_A\",\"TEST_CODE_CONCEPT_2_B\"]".to_owned(),
            ),
        ]
    );

    fixture.execute(
        "UPDATE stock_concepts SET concepts='[\"TEST_CODE_LIVE_MUTATION\"]' \
         WHERE code IN ('TEST_CODE_HIT','TEST_CODE_MISSING_1','TEST_CODE_MISSING_2');",
    );
    fixture.reopen();
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let head = local.inspect_run(&intent_id).unwrap().head_version();
    let lease = local
        .resume_run(
            &intent_id,
            lease_request("TEST_CODE_BATCH_OWNER_B", 5_001, 9_000, Some(head)),
        )
        .unwrap();
    let provider = BatchRawProvider::panic_if_called(database);
    let clock = ControlledClock::new(at(5_100));
    let error = run_public_batch_prepare(&mut local, lease, &provider, &clock, stocks()).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::ClusterConfiguration,
        })
    ));
    assert!(provider.calls().is_empty());
    let inspection = local.inspect_concept_batch(&intent_id).unwrap();
    assert_eq!(inspection.concepts(), &expected);
    assert_eq!(
        inspection.applied_codes(),
        ["TEST_CODE_MISSING_1", "TEST_CODE_MISSING_2"]
    );
    drop(local);

    assert_eq!(fixture.count("chain_post_close_concept_cache_writes"), 2);
    assert_eq!(
        stored_cache_rows(&fixture),
        vec![
            (
                "TEST_CODE_EXPIRED".to_owned(),
                "[\"TEST_CODE_OLD\"]".to_owned(),
            ),
            (
                "TEST_CODE_HIT".to_owned(),
                "[\"TEST_CODE_LIVE_MUTATION\"]".to_owned(),
            ),
            (
                "TEST_CODE_MISSING_1".to_owned(),
                "[\"TEST_CODE_LIVE_MUTATION\"]".to_owned(),
            ),
            (
                "TEST_CODE_MISSING_2".to_owned(),
                "[\"TEST_CODE_LIVE_MUTATION\"]".to_owned(),
            ),
        ]
    );
}

fn install_v3(fixture: &mut V2BusinessFixture) {
    fixture.install_v2();
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v2_to_v3()
            .unwrap()
            .schema_version(),
        3
    );
}

fn batch_stocks(missing: usize) -> Vec<TopStock> {
    std::iter::once("TEST_CODE_HIT".to_owned())
        .chain((0..missing).map(|index| format!("TEST_CODE_BATCH_MISSING_{index}")))
        .enumerate()
        .map(|(index, code)| TopStock {
            code,
            name: format!("TEST_CODE_BATCH_NAME_{index}"),
            change_pct: 10.0 - index as f64 / 100.0,
            price: 20.0 + index as f64,
            ..TopStock::default()
        })
        .collect()
}

fn provider_raw(code: &str) -> String {
    format!("{{\"all_boards\":[\"TEST_CODE_CONCEPT_FOR_{code}\"]}}")
}

#[derive(Clone)]
enum DeferredReply {
    Returned(String),
    BusinessError(String),
}

#[derive(Default)]
struct DeferredState {
    calls: Vec<String>,
    ready: HashMap<String, DeferredReply>,
    wakers: HashMap<String, Waker>,
    active: usize,
    max_active: usize,
}

struct ActiveCall {
    state: Rc<RefCell<DeferredState>>,
}

impl Drop for ActiveCall {
    fn drop(&mut self) {
        let mut state = self.state.borrow_mut();
        state.active -= 1;
    }
}

struct DeferredRawProvider {
    database: PathBuf,
    state: Rc<RefCell<DeferredState>>,
}

impl DeferredRawProvider {
    fn new(database: PathBuf) -> Self {
        Self {
            database,
            state: Rc::new(RefCell::new(DeferredState::default())),
        }
    }

    fn release(&self, code: &str, reply: DeferredReply) {
        let waker = {
            let mut state = self.state.borrow_mut();
            assert!(state.ready.insert(code.to_owned(), reply).is_none());
            state.wakers.remove(code)
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn calls(&self) -> Vec<String> {
        self.state.borrow().calls.clone()
    }

    fn max_active(&self) -> usize {
        self.state.borrow().max_active
    }
}

#[async_trait::async_trait(?Send)]
impl ConceptProviderRawIo for DeferredRawProvider {
    async fn call_raw(&self, code: &str) -> std::result::Result<String, String> {
        let ordinal: i64 = Connection::open_with_flags(
            &self.database,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap()
        .query_row(
            "SELECT effect_ordinal FROM chain_post_close_stage_begins \
             WHERE effect_kind='ConceptProvider' AND effect_key=?1",
            [code],
            |row| row.get(0),
        )
        .expect("deferred provider must observe its committed begin");
        assert_eq!(
            ordinal,
            code.rsplit('_').next().unwrap().parse::<i64>().unwrap()
        );
        {
            let mut state = self.state.borrow_mut();
            state.calls.push(code.to_owned());
            state.active += 1;
            state.max_active = state.max_active.max(state.active);
        }
        let _active = ActiveCall {
            state: Rc::clone(&self.state),
        };
        poll_fn(|context| {
            let mut state = self.state.borrow_mut();
            match state.ready.remove(code) {
                Some(DeferredReply::Returned(raw)) => Poll::Ready(Ok(raw)),
                Some(DeferredReply::BusinessError(error)) => Poll::Ready(Err(error)),
                None => {
                    state
                        .wakers
                        .insert(code.to_owned(), context.waker().clone());
                    Poll::Pending
                }
            }
        })
        .await
    }
}

fn query_count(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn completion_codes(connection: &Connection, table: &str) -> Vec<String> {
    let sql = match table {
        "chain_post_close_stage_results" => {
            "SELECT begun.effect_key FROM chain_post_close_stage_results AS fact \
             JOIN chain_post_close_stage_begins AS begun \
               ON begun.intent_id=fact.intent_id \
              AND begun.effect_kind=fact.effect_kind \
              AND begun.effect_ordinal=fact.effect_ordinal \
             ORDER BY fact.run_version"
        }
        "chain_post_close_concept_cache_writes" => {
            "SELECT code FROM chain_post_close_concept_cache_writes ORDER BY run_version"
        }
        _ => panic!("unexpected completion table"),
    };
    connection
        .prepare(sql)
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn batch_cache_rows(connection: &Connection) -> Vec<(String, String)> {
    connection
        .prepare(
            "SELECT code,concepts FROM stock_concepts \
             WHERE code GLOB 'TEST_CODE_BATCH_MISSING_*' ORDER BY code",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn ordered_raw_results(connection: &Connection) -> Vec<(i64, String, Vec<u8>)> {
    connection
        .prepare(
            "SELECT effect_ordinal,outcome,CAST(result_bytes AS BLOB) \
             FROM chain_post_close_stage_results ORDER BY effect_ordinal",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn assert_concept_failure(error: &anyhow::Error, expected_reason: &str) {
    let failure = error
        .downcast_ref::<PreparationFailure>()
        .expect("ordinary concept errors must retain their protected observation");
    assert_eq!(failure.stage(), PreparationStage::Concepts);
    assert!(failure.reason().contains(expected_reason));
    assert!(!error.to_string().contains(expected_reason));
}

fn assert_storage_commit_cause(error: &anyhow::Error) {
    assert!(matches!(
        error.downcast_ref::<ChainPostCloseError>(),
        Some(ChainPostCloseError::StorageFailed {
            operation: "commit",
        })
    ));
}

#[tokio::test]
async fn single_user_local_all_cache_hits_preserve_the_complete_fixed_cache_image() {
    let mut fixture = V2BusinessFixture::new();
    install_v3(&mut fixture);
    fixture.execute(
        "INSERT INTO stock_concepts(code,concepts,updated_at) VALUES \
         ('TEST_CODE_HIT','[\"TEST_CODE_CACHED\"]','2026-07-21 14:00:00'), \
         ('TEST_CODE_MISSING_1','[\"TEST_CODE_CACHED_1\"]','2026-07-21 14:01:00'), \
         ('TEST_CODE_MISSING_2','[\"TEST_CODE_CACHED_2\"]','2026-07-21 14:02:00'), \
         ('TEST_CODE_UNRELATED','[\"TEST_CODE_UNRELATED_CONCEPT\"]','2026-07-21 14:03:00');",
    );
    let original_cache = stored_cache_rows_with_time(&fixture);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_ALL_CACHE_HITS"),
    )
    .unwrap();
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            context,
            fixed_input(stocks()),
            lease_request("TEST_CODE_ALL_HITS_OWNER", 1_000, 4_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let provider = BatchRawProvider::panic_if_called(database);
    let clock = ControlledClock::new(at(1_100));
    let error = run_public_batch_prepare(&mut local, lease, &provider, &clock, stocks()).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::ClusterConfiguration,
        })
    ));
    assert!(provider.calls().is_empty());
    let expected = HashMap::from([
        (
            "TEST_CODE_HIT".to_owned(),
            vec!["TEST_CODE_CACHED".to_owned()],
        ),
        (
            "TEST_CODE_MISSING_1".to_owned(),
            vec!["TEST_CODE_CACHED_1".to_owned()],
        ),
        (
            "TEST_CODE_MISSING_2".to_owned(),
            vec!["TEST_CODE_CACHED_2".to_owned()],
        ),
        (
            "TEST_CODE_UNRELATED".to_owned(),
            vec!["TEST_CODE_UNRELATED_CONCEPT".to_owned()],
        ),
    ]);
    let inspection = local.inspect_concept_batch(&intent_id).unwrap();
    assert_eq!(inspection.concepts(), &expected);
    assert!(inspection.effects().is_empty());
    assert!(inspection.applied_codes().is_empty());
    assert!(inspection.is_complete());
    drop(local);
    assert_eq!(fixture.count("chain_post_close_stage_begins"), 0);
    assert_eq!(fixture.count("chain_post_close_stage_results"), 0);
    assert_eq!(fixture.count("chain_post_close_concept_cache_writes"), 0);
    assert_eq!(stored_cache_rows_with_time(&fixture), original_cache);
}

#[tokio::test]
async fn single_user_local_eight_missing_keep_six_in_flight_and_persist_each_result_before_cache() {
    let mut fixture = V2BusinessFixture::new();
    install_v3(&mut fixture);
    seed_cache(&fixture);
    let input_stocks = batch_stocks(8);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_EIGHT_MISSING"),
    )
    .unwrap();
    let database = fixture.database();
    let observation = Connection::open_with_flags(
        &database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            context,
            fixed_input(input_stocks.clone()),
            lease_request("TEST_CODE_EIGHT_OWNER", 1_000, 8_000, None),
        )
        .unwrap();
    let provider = DeferredRawProvider::new(database);
    let clock = ControlledClock::new(at(1_100));
    let mut future = Box::pin(run_public_batch_prepare(
        &mut local,
        lease,
        &provider,
        &clock,
        input_stocks,
    ));
    let waker = futures::task::noop_waker();
    let mut context = std::task::Context::from_waker(&waker);
    assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    assert_eq!(provider.calls().len(), 6);
    assert_eq!(provider.max_active(), 6);
    assert_eq!(
        query_count(&observation, "chain_post_close_stage_begins"),
        6
    );
    assert_eq!(
        query_count(&observation, "chain_post_close_stage_results"),
        0
    );
    assert_eq!(
        query_count(&observation, "chain_post_close_concept_cache_writes"),
        0
    );
    assert!(batch_cache_rows(&observation).is_empty());

    let completion_order = [5usize, 1, 6, 0, 4, 2, 7, 3];
    for (completed, index) in completion_order.into_iter().enumerate() {
        let code = format!("TEST_CODE_BATCH_MISSING_{index}");
        provider.release(&code, DeferredReply::Returned(provider_raw(&code)));
        let poll = future.as_mut().poll(&mut context);
        if completed + 1 < completion_order.len() {
            assert!(matches!(poll, Poll::Pending));
            assert_eq!(
                query_count(&observation, "chain_post_close_stage_results"),
                i64::try_from(completed + 1).unwrap()
            );
            assert_eq!(
                query_count(&observation, "chain_post_close_concept_cache_writes"),
                0,
                "legacy cache application starts only after every provider completes"
            );
            assert!(
                batch_cache_rows(&observation).is_empty(),
                "business cache rows must not precede their application facts"
            );
        } else {
            let error = match poll {
                Poll::Ready(error) => error,
                Poll::Pending => panic!("last provider completion must finish concept batch"),
            };
            assert!(matches!(
                error.downcast_ref::<PreparationStop>(),
                Some(PreparationStop::StageNotMigrated {
                    next: UnmigratedStage::ClusterConfiguration,
                })
            ));
        }
    }
    assert_eq!(provider.max_active(), 6);
    assert_eq!(provider.calls().len(), 8);
    let expected_order = completion_order
        .into_iter()
        .map(|index| format!("TEST_CODE_BATCH_MISSING_{index}"))
        .collect::<Vec<_>>();
    assert_eq!(
        completion_codes(&observation, "chain_post_close_stage_results"),
        expected_order
    );
    assert_eq!(
        completion_codes(&observation, "chain_post_close_concept_cache_writes"),
        expected_order
    );
    assert_eq!(
        query_count(&observation, "chain_post_close_concept_cache_writes"),
        8
    );
    assert_eq!(batch_cache_rows(&observation).len(), 8);
    drop(future);
    drop(local);
    observation.close().unwrap();
}

async fn assert_ordinary_failure_collects_before_cache_prefix(
    run_id: &str,
    replies: [DeferredReply; 4],
    expected_error: &str,
) {
    let mut fixture = V2BusinessFixture::new();
    install_v3(&mut fixture);
    seed_cache(&fixture);
    let input_stocks = batch_stocks(4);
    let config = local_config(BUILD_A);
    let context =
        build_single_user_local_chain_post_close_context(&config, run_input(run_id)).unwrap();
    let database = fixture.database();
    let observation = Connection::open_with_flags(
        &database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            context,
            fixed_input(input_stocks.clone()),
            lease_request("TEST_CODE_ORDINARY_ERROR_OWNER", 1_000, 5_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let provider = DeferredRawProvider::new(database);
    let clock = ControlledClock::new(at(1_100));
    let mut future = Box::pin(run_public_batch_prepare(
        &mut local,
        lease,
        &provider,
        &clock,
        input_stocks.clone(),
    ));
    let waker = futures::task::noop_waker();
    let mut context = std::task::Context::from_waker(&waker);
    assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    assert_eq!(provider.calls().len(), 4);
    assert!(batch_cache_rows(&observation).is_empty());
    let expected_raw = replies
        .iter()
        .enumerate()
        .map(|(ordinal, reply)| match reply {
            DeferredReply::Returned(raw) => (
                i64::try_from(ordinal).unwrap(),
                "Returned".to_owned(),
                raw.as_bytes().to_vec(),
            ),
            DeferredReply::BusinessError(error) => (
                i64::try_from(ordinal).unwrap(),
                "BusinessError".to_owned(),
                error.as_bytes().to_vec(),
            ),
        })
        .collect::<Vec<_>>();

    for (index, reply) in replies.into_iter().enumerate() {
        let code = format!("TEST_CODE_BATCH_MISSING_{index}");
        provider.release(&code, reply);
        let poll = future.as_mut().poll(&mut context);
        if index < 3 {
            assert!(matches!(poll, Poll::Pending));
            assert_eq!(
                query_count(&observation, "chain_post_close_stage_results"),
                i64::try_from(index + 1).unwrap()
            );
            assert_eq!(
                query_count(&observation, "chain_post_close_concept_cache_writes"),
                0
            );
            assert!(batch_cache_rows(&observation).is_empty());
        } else {
            let error = match poll {
                Poll::Ready(error) => error,
                Poll::Pending => panic!("ordinary failures are evaluated after collection"),
            };
            assert_concept_failure(&error, expected_error);
        }
    }
    assert_eq!(provider.calls().len(), 4);
    assert_eq!(
        query_count(&observation, "chain_post_close_stage_results"),
        4
    );
    assert_eq!(ordered_raw_results(&observation), expected_raw);
    assert_eq!(
        completion_codes(&observation, "chain_post_close_concept_cache_writes"),
        vec!["TEST_CODE_BATCH_MISSING_0"]
    );
    let expected_cache_prefix = vec![(
        "TEST_CODE_BATCH_MISSING_0".to_owned(),
        "[\"TEST_CODE_CONCEPT_FOR_TEST_CODE_BATCH_MISSING_0\"]".to_owned(),
    )];
    assert_eq!(batch_cache_rows(&observation), expected_cache_prefix);
    drop(future);
    drop(local);
    observation.close().unwrap();

    fixture.reopen();
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let head = local.inspect_run(&intent_id).unwrap().head_version();
    let lease = local
        .resume_run(
            &intent_id,
            lease_request("TEST_CODE_ORDINARY_ERROR_REOPEN", 5_001, 9_000, Some(head)),
        )
        .unwrap();
    let provider = BatchRawProvider::panic_if_called(database);
    let clock = ControlledClock::new(at(5_100));
    let error = run_public_batch_prepare(&mut local, lease, &provider, &clock, input_stocks).await;
    assert_concept_failure(&error, expected_error);
    assert!(provider.calls().is_empty());
    drop(local);
    assert_eq!(fixture.count("chain_post_close_stage_results"), 4);
    assert_eq!(fixture.count("chain_post_close_concept_cache_writes"), 1);
    assert_eq!(
        batch_cache_rows(fixture.connection()),
        expected_cache_prefix
    );
}

#[tokio::test]
async fn single_user_local_ordinary_provider_and_empty_raw_errors_collect_before_cache_prefix() {
    assert_ordinary_failure_collects_before_cache_prefix(
        "TEST_CODE_RUN_BUSINESS_ERROR_BATCH",
        [
            DeferredReply::Returned(provider_raw("TEST_CODE_BATCH_MISSING_0")),
            DeferredReply::BusinessError("TEST_CODE_PROVIDER_FAILURE".to_owned()),
            DeferredReply::Returned(provider_raw("TEST_CODE_BATCH_MISSING_2")),
            DeferredReply::Returned(provider_raw("TEST_CODE_BATCH_MISSING_3")),
        ],
        "TEST_CODE_PROVIDER_FAILURE",
    )
    .await;
    assert_ordinary_failure_collects_before_cache_prefix(
        "TEST_CODE_RUN_EMPTY_RAW_BATCH",
        [
            DeferredReply::Returned(provider_raw("TEST_CODE_BATCH_MISSING_0")),
            DeferredReply::Returned(String::new()),
            DeferredReply::Returned(provider_raw("TEST_CODE_BATCH_MISSING_2")),
            DeferredReply::BusinessError("TEST_CODE_LATER_FAILURE".to_owned()),
        ],
        "板块 JSON 非法",
    )
    .await;
}

fn hold_read_lock(database: &std::path::Path, table: &str) -> Connection {
    let connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode.to_ascii_lowercase(), "delete");
    connection.execute_batch("BEGIN DEFERRED;").unwrap();
    assert!(query_count(&connection, table) >= 0);
    connection
}

fn release_read_lock(connection: Connection) {
    connection.execute_batch("ROLLBACK;").unwrap();
    connection.close().unwrap();
}

#[tokio::test]
async fn single_user_local_cancels_six_pending_calls_without_refill_or_replay() {
    let mut fixture = V2BusinessFixture::new();
    install_v3(&mut fixture);
    seed_cache(&fixture);
    let input_stocks = batch_stocks(8);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_BATCH_CANCELLED"),
    )
    .unwrap();
    let database = fixture.database();
    let observation = Connection::open_with_flags(
        &database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            context,
            fixed_input(input_stocks.clone()),
            lease_request("TEST_CODE_BATCH_CANCEL_OWNER", 1_000, 8_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let provider = DeferredRawProvider::new(database);
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .concept_batch_preparation_io(lease, &provider, &clock)
        .unwrap();
    {
        let mut future = Box::pin(prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            input_stocks.clone(),
            None,
            &mut io,
        ));
        let waker = futures::task::noop_waker();
        let mut context = std::task::Context::from_waker(&waker);
        assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
        assert_eq!(provider.calls().len(), 6);
        assert_eq!(provider.max_active(), 6);
    }
    let error = prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        input_stocks.clone(),
        None,
        &mut io,
    )
    .await
    .expect_err("cancelled batch adapter must stop");
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::ResultUnconfirmed { .. })
    ));
    drop(io);
    assert_eq!(provider.calls().len(), 6);
    assert_eq!(
        query_count(&observation, "chain_post_close_stage_begins"),
        6
    );
    assert_eq!(
        query_count(&observation, "chain_post_close_stage_results"),
        0
    );
    assert_eq!(
        query_count(&observation, "chain_post_close_concept_cache_writes"),
        0
    );
    assert!(batch_cache_rows(&observation).is_empty());
    observation.close().unwrap();
    drop(local);

    fixture.reopen();
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let head = local.inspect_run(&intent_id).unwrap().head_version();
    let lease = local
        .resume_run(
            &intent_id,
            lease_request("TEST_CODE_BATCH_CANCEL_REOPEN", 8_001, 12_000, Some(head)),
        )
        .unwrap();
    let provider = BatchRawProvider::panic_if_called(database);
    let clock = ControlledClock::new(at(8_100));
    let error = run_public_batch_prepare(&mut local, lease, &provider, &clock, input_stocks).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::IncompleteOnReopen { .. })
    ));
    assert!(provider.calls().is_empty());
}

#[tokio::test]
async fn single_user_local_result_commit_failure_cancels_other_inflight_calls() {
    let mut fixture = V2BusinessFixture::new();
    install_v3(&mut fixture);
    seed_cache(&fixture);
    let input_stocks = batch_stocks(8);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_BATCH_RESULT_COMMIT_FAILURE"),
    )
    .unwrap();
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            context,
            fixed_input(input_stocks.clone()),
            lease_request("TEST_CODE_BATCH_RESULT_FAULT_OWNER", 1_000, 8_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let provider = DeferredRawProvider::new(database.clone());
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .concept_batch_preparation_io(lease, &provider, &clock)
        .unwrap();
    let mut future = Box::pin(prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        input_stocks.clone(),
        None,
        &mut io,
    ));
    let waker = futures::task::noop_waker();
    let mut context = std::task::Context::from_waker(&waker);
    assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    assert_eq!(provider.calls().len(), 6);
    assert_eq!(provider.max_active(), 6);

    let read_lock = hold_read_lock(&database, "chain_post_close_stage_begins");
    let first_code = "TEST_CODE_BATCH_MISSING_0";
    provider.release(
        first_code,
        DeferredReply::Returned(provider_raw(first_code)),
    );
    let error = match future.as_mut().poll(&mut context) {
        Poll::Ready(result) => result.expect_err("result commit failure must stop"),
        Poll::Pending => panic!("result commit failure must stop the batch"),
    };
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::ResultUnconfirmed { .. })
    ));
    assert_storage_commit_cause(&error);
    drop(future);
    let error = prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        input_stocks.clone(),
        None,
        &mut io,
    )
    .await
    .expect_err("faulted adapter must remain cancelled");
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::ResultUnconfirmed { .. })
    ));
    drop(io);
    assert_eq!(provider.calls().len(), 6);
    assert_eq!(query_count(&read_lock, "chain_post_close_stage_begins"), 6);
    assert_eq!(query_count(&read_lock, "chain_post_close_stage_results"), 0);
    assert_eq!(
        query_count(&read_lock, "chain_post_close_concept_cache_writes"),
        0
    );
    assert!(batch_cache_rows(&read_lock).is_empty());
    release_read_lock(read_lock);
    drop(local);

    fixture.reopen();
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let head = local.inspect_run(&intent_id).unwrap().head_version();
    let lease = local
        .resume_run(
            &intent_id,
            lease_request(
                "TEST_CODE_BATCH_RESULT_FAULT_REOPEN",
                8_001,
                12_000,
                Some(head),
            ),
        )
        .unwrap();
    let provider = BatchRawProvider::panic_if_called(database);
    let clock = ControlledClock::new(at(8_100));
    let error = run_public_batch_prepare(&mut local, lease, &provider, &clock, input_stocks).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::IncompleteOnReopen { .. })
    ));
    assert!(provider.calls().is_empty());
}

struct CacheCommitFaultClock {
    now: i64,
    database: PathBuf,
    reader: RefCell<Option<Connection>>,
}

impl CacheCommitFaultClock {
    fn new(now: i64, database: PathBuf) -> Self {
        Self {
            now,
            database,
            reader: RefCell::new(None),
        }
    }

    fn release(&self) {
        release_read_lock(
            self.reader
                .borrow_mut()
                .take()
                .expect("cache commit fault must have armed a read lock"),
        );
    }
}

impl ConceptEffectClock for CacheCommitFaultClock {
    fn now(&self) -> UtcMicros {
        if self.reader.borrow().is_none() {
            let observation = Connection::open_with_flags(
                &self.database,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .unwrap();
            let ready_for_second_cache =
                query_count(&observation, "chain_post_close_stage_results") == 2
                    && query_count(&observation, "chain_post_close_concept_cache_writes") == 1;
            observation.close().unwrap();
            if ready_for_second_cache {
                *self.reader.borrow_mut() = Some(hold_read_lock(
                    &self.database,
                    "chain_post_close_concept_cache_writes",
                ));
            }
        }
        UtcMicros::try_new(self.now).unwrap()
    }
}

fn cache_rows_with_time(connection: &Connection) -> Vec<(String, String, String)> {
    connection
        .prepare(
            "SELECT code,concepts,updated_at FROM stock_concepts \
             WHERE code GLOB 'TEST_CODE_BATCH_MISSING_*' ORDER BY code",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn cache_fact_versions(connection: &Connection) -> Vec<(String, i64)> {
    connection
        .prepare(
            "SELECT code,run_version FROM chain_post_close_concept_cache_writes \
             ORDER BY run_version",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[tokio::test]
async fn single_user_local_cache_commit_failure_rolls_back_business_row_fact_and_head() {
    let mut fixture = V2BusinessFixture::new();
    install_v3(&mut fixture);
    seed_cache(&fixture);
    let input_stocks = batch_stocks(2);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_BATCH_CACHE_COMMIT_FAILURE"),
    )
    .unwrap();
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            context,
            fixed_input(input_stocks.clone()),
            lease_request("TEST_CODE_BATCH_CACHE_FAULT_OWNER", 1_000, 8_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let provider = DeferredRawProvider::new(database.clone());
    for index in 0..2 {
        let code = format!("TEST_CODE_BATCH_MISSING_{index}");
        provider.release(&code, DeferredReply::Returned(provider_raw(&code)));
    }
    let clock = CacheCommitFaultClock::new(at(1_100), database);
    let error =
        run_public_batch_prepare(&mut local, lease, &provider, &clock, input_stocks.clone()).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::AuthorityRejected { .. })
    ));
    assert_storage_commit_cause(&error);
    clock.release();
    assert_eq!(provider.calls().len(), 2);
    let head_after_failure = local.inspect_run(&intent_id).unwrap().head_version();
    assert_eq!(head_after_failure, 5);
    drop(local);
    assert_eq!(fixture.count("chain_post_close_stage_results"), 2);
    assert_eq!(fixture.count("chain_post_close_concept_cache_writes"), 1);
    let original_fact = cache_fact_versions(fixture.connection());
    assert_eq!(original_fact.len(), 1);
    let applied_code = original_fact[0].0.clone();
    let rows_after_failure = cache_rows_with_time(fixture.connection());
    assert_eq!(rows_after_failure.len(), 1);
    assert_eq!(rows_after_failure[0].0, applied_code);
    fixture
        .connection()
        .execute(
            "UPDATE stock_concepts SET concepts='[\"TEST_CODE_LIVE_MUTATION\"]', \
             updated_at='2026-07-21 15:59:59' WHERE code=?1",
            [&applied_code],
        )
        .unwrap();

    fixture.reopen();
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .resume_run(
            &intent_id,
            lease_request(
                "TEST_CODE_BATCH_CACHE_FAULT_REOPEN",
                8_001,
                12_000,
                Some(head_after_failure),
            ),
        )
        .unwrap();
    let provider = BatchRawProvider::panic_if_called(database);
    let clock = ControlledClock::new(at(8_100));
    let error = run_public_batch_prepare(&mut local, lease, &provider, &clock, input_stocks).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::ClusterConfiguration,
        })
    ));
    assert!(provider.calls().is_empty());
    let inspection = local.inspect_concept_batch(&intent_id).unwrap();
    assert!(inspection.is_complete());
    drop(local);
    assert_eq!(fixture.count("chain_post_close_concept_cache_writes"), 2);
    let cache_generations = fixture
        .connection()
        .prepare(
            "SELECT lease_generation FROM chain_post_close_concept_cache_writes \
             ORDER BY run_version",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<i64>>>()
        .unwrap();
    assert_eq!(cache_generations, [1, 2]);
    let final_facts = cache_fact_versions(fixture.connection());
    assert_eq!(final_facts[0], original_fact[0]);
    let final_rows = cache_rows_with_time(fixture.connection());
    assert_eq!(final_rows.len(), 2);
    let preserved = final_rows
        .iter()
        .find(|(code, _, _)| code == &applied_code)
        .unwrap();
    assert_eq!(preserved.1, "[\"TEST_CODE_LIVE_MUTATION\"]");
    assert_eq!(preserved.2, "2026-07-21 15:59:59");
    let recovered = final_rows
        .iter()
        .find(|(code, _, _)| code != &applied_code)
        .unwrap();
    assert_eq!(
        recovered.1,
        format!("[\"TEST_CODE_CONCEPT_FOR_{}\"]", recovered.0)
    );
}

fn begin_call(
    local: &mut super::super::LocalChainPostClose<'_>,
    lease: super::super::RunLease,
    ordinal: u64,
    code: &str,
    now: i64,
) -> (super::super::RunLease, super::super::ConceptProviderCall) {
    let request = ConceptProviderRequest::try_new(ordinal, code.to_owned()).unwrap();
    let (lease, admission) = local
        .begin_concept_provider(lease, request, UtcMicros::try_new(now).unwrap())
        .unwrap();
    let ConceptProviderAdmission::Call(call) = admission else {
        panic!("new partial effect must require a provider call");
    };
    (lease, call)
}

#[tokio::test]
async fn single_user_local_partial_batch_distinguishes_and_enforces_all_three_states() {
    let mut blocked_fixture = V2BusinessFixture::new();
    install_v3(&mut blocked_fixture);
    seed_cache(&blocked_fixture);
    let input_stocks = batch_stocks(3);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_BATCH_PARTIAL_BLOCKED"),
    )
    .unwrap();
    let database = blocked_fixture.database();
    let mut local = blocked_fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            context,
            fixed_input(input_stocks.clone()),
            lease_request("TEST_CODE_BATCH_PARTIAL_OWNER", 1_000, 8_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let code0 = "TEST_CODE_BATCH_MISSING_0";
    let (lease, call) = begin_call(&mut local, lease, 0, code0, at(1_100));
    let (lease, _) = local
        .record_concept_provider_result(
            lease,
            call,
            ConceptProviderRawResult::returned(provider_raw(code0)),
            UtcMicros::try_new(at(1_200)).unwrap(),
        )
        .unwrap();
    let (lease, _unconfirmed) =
        begin_call(&mut local, lease, 1, "TEST_CODE_BATCH_MISSING_1", at(1_300));
    let inspection = local.inspect_concept_batch(&intent_id).unwrap();
    let states = inspection
        .effects()
        .iter()
        .map(|effect| (effect.ordinal(), effect.code().to_owned(), effect.state()))
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        vec![
            (
                0,
                "TEST_CODE_BATCH_MISSING_0".to_owned(),
                super::super::ConceptEffectRecoveryState::Confirmed,
            ),
            (
                1,
                "TEST_CODE_BATCH_MISSING_1".to_owned(),
                super::super::ConceptEffectRecoveryState::BegunUnconfirmed,
            ),
            (
                2,
                "TEST_CODE_BATCH_MISSING_2".to_owned(),
                super::super::ConceptEffectRecoveryState::NeverStarted,
            ),
        ]
    );
    assert!(!inspection.is_complete());
    let provider = BatchRawProvider::panic_if_called(database);
    let clock = ControlledClock::new(at(1_400));
    let error = run_public_batch_prepare(&mut local, lease, &provider, &clock, input_stocks).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::IncompleteOnReopen { .. })
    ));
    assert!(provider.calls().is_empty());
    drop(local);
    assert_eq!(blocked_fixture.count("chain_post_close_stage_begins"), 2);

    let mut resumable_fixture = V2BusinessFixture::new();
    install_v3(&mut resumable_fixture);
    seed_cache(&resumable_fixture);
    let input_stocks = batch_stocks(3);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_BATCH_PARTIAL_RESUMABLE"),
    )
    .unwrap();
    let database = resumable_fixture.database();
    let mut local = resumable_fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            context,
            fixed_input(input_stocks.clone()),
            lease_request("TEST_CODE_BATCH_PARTIAL_RESUME_OWNER", 1_000, 8_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let (lease, call) = begin_call(&mut local, lease, 0, code0, at(1_100));
    let (lease, _) = local
        .record_concept_provider_result(
            lease,
            call,
            ConceptProviderRawResult::returned(provider_raw(code0)),
            UtcMicros::try_new(at(1_200)).unwrap(),
        )
        .unwrap();
    let provider = DeferredRawProvider::new(database);
    for index in 1..3 {
        let code = format!("TEST_CODE_BATCH_MISSING_{index}");
        provider.release(&code, DeferredReply::Returned(provider_raw(&code)));
    }
    let clock = ControlledClock::new(at(1_300));
    let error = run_public_batch_prepare(&mut local, lease, &provider, &clock, input_stocks).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::ClusterConfiguration,
        })
    ));
    assert_eq!(
        provider.calls(),
        vec!["TEST_CODE_BATCH_MISSING_1", "TEST_CODE_BATCH_MISSING_2"]
    );
    let inspection = local.inspect_concept_batch(&intent_id).unwrap();
    assert!(inspection.is_complete());
    assert!(inspection
        .effects()
        .iter()
        .all(|effect| { effect.state() == super::super::ConceptEffectRecoveryState::Confirmed }));
    assert_eq!(inspection.applied_codes().len(), 3);
}

async fn completed_batch_fixture(
    run_id: &str,
) -> (V2BusinessFixture, LocalChainPostCloseConfig, IntentId) {
    let mut fixture = V2BusinessFixture::new();
    install_v3(&mut fixture);
    seed_cache(&fixture);
    let config = local_config(BUILD_A);
    let context =
        build_single_user_local_chain_post_close_context(&config, run_input(run_id)).unwrap();
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            context,
            fixed_input(stocks()),
            lease_request("TEST_CODE_BATCH_RELATION_OWNER", 1_000, 8_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let provider = BatchRawProvider::returning(database);
    let clock = ControlledClock::new(at(1_100));
    let error = run_public_batch_prepare(&mut local, lease, &provider, &clock, stocks()).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::ClusterConfiguration,
        })
    ));
    assert!(local
        .inspect_concept_batch(&intent_id)
        .unwrap()
        .is_complete());
    drop(local);
    (fixture, config, intent_id)
}

async fn assert_completed_batch_corruption_rejected(run_id: &str, update: &str) {
    let (mut fixture, config, intent_id) = completed_batch_fixture(run_id).await;
    mutate_behind_guard(
        &fixture,
        "chain_post_close_concept_cache_writes_update",
        update,
    );
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    assert_eq!(
        local.inspect_concept_batch(&intent_id).err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
}

#[tokio::test]
async fn single_user_local_inspect_rejects_cache_code_and_raw_swapped_between_ordinals() {
    let concepts_1 = serde_json::to_vec(&vec!["TEST_CODE_CONCEPT_1"]).unwrap();
    let concepts_2 =
        serde_json::to_vec(&vec!["TEST_CODE_CONCEPT_2_A", "TEST_CODE_CONCEPT_2_B"]).unwrap();
    let update = format!(
        "UPDATE chain_post_close_concept_cache_writes \
         SET code='TEST_CODE_SWAP_TEMP' WHERE effect_ordinal=0; \
         UPDATE chain_post_close_concept_cache_writes SET \
           code='TEST_CODE_MISSING_1', \
           concepts_bytes=CAST('{}' AS BLOB),concepts_length={},concepts_sha256='{}' \
         WHERE effect_ordinal=1; \
         UPDATE chain_post_close_concept_cache_writes SET \
           code='TEST_CODE_MISSING_2', \
           concepts_bytes=CAST('{}' AS BLOB),concepts_length={},concepts_sha256='{}' \
         WHERE effect_ordinal=0;",
        std::str::from_utf8(&concepts_1).unwrap(),
        concepts_1.len(),
        raw_digest(&concepts_1).as_str(),
        std::str::from_utf8(&concepts_2).unwrap(),
        concepts_2.len(),
        raw_digest(&concepts_2).as_str(),
    );
    assert_completed_batch_corruption_rejected("TEST_CODE_RUN_BATCH_CORRUPT_CODE_RAW", &update)
        .await;
}

#[tokio::test]
async fn single_user_local_inspect_rejects_cache_version_reused_from_another_result() {
    assert_completed_batch_corruption_rejected(
        "TEST_CODE_RUN_BATCH_CORRUPT_CROSS_VERSION",
        "UPDATE chain_post_close_concept_cache_writes SET run_version=( \
             SELECT run_version FROM chain_post_close_stage_results \
             WHERE effect_kind='ConceptProvider' AND effect_ordinal=1 \
         ) WHERE effect_ordinal=0;",
    )
    .await;
}

#[tokio::test]
async fn single_user_local_inspect_rejects_cache_owner_contradiction_in_same_generation() {
    assert_completed_batch_corruption_rejected(
        "TEST_CODE_RUN_BATCH_CORRUPT_OWNER",
        "UPDATE chain_post_close_concept_cache_writes \
         SET lease_owner='TEST_CODE_FORGED_OWNER' WHERE effect_ordinal=0;",
    )
    .await;
}

#[tokio::test]
async fn single_user_local_inspect_rejects_cache_generation_beyond_current_run() {
    assert_completed_batch_corruption_rejected(
        "TEST_CODE_RUN_BATCH_CORRUPT_GENERATION",
        "UPDATE chain_post_close_concept_cache_writes \
         SET lease_generation=lease_generation+1 WHERE effect_ordinal=0;",
    )
    .await;
}

#[tokio::test]
async fn single_user_local_inspect_rejects_cache_time_before_provider_result() {
    assert_completed_batch_corruption_rejected(
        "TEST_CODE_RUN_BATCH_CORRUPT_TIME",
        "UPDATE chain_post_close_concept_cache_writes \
         SET written_at=0 WHERE effect_ordinal=0;",
    )
    .await;
}

#[tokio::test]
async fn single_user_local_cache_sql_failure_rolls_back_only_the_target_application() {
    let mut fixture = V2BusinessFixture::new();
    install_v3(&mut fixture);
    seed_cache(&fixture);
    fixture.execute(
        "CREATE TRIGGER TEST_CODE_FAIL_SECOND_CACHE \
         BEFORE INSERT ON stock_concepts \
         WHEN NEW.code='TEST_CODE_BATCH_MISSING_1' \
         BEGIN SELECT RAISE(ABORT,'TEST_CODE_CACHE_WRITE_FAILURE'); END;",
    );
    let input_stocks = batch_stocks(2);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_BATCH_CACHE_SQL_FAILURE"),
    )
    .unwrap();
    let database = fixture.database();
    let observation = Connection::open_with_flags(
        &database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            context,
            fixed_input(input_stocks.clone()),
            lease_request("TEST_CODE_BATCH_CACHE_SQL_OWNER", 1_000, 8_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let provider = DeferredRawProvider::new(database);
    let clock = ControlledClock::new(at(1_100));
    let mut future = Box::pin(run_public_batch_prepare(
        &mut local,
        lease,
        &provider,
        &clock,
        input_stocks.clone(),
    ));
    let waker = futures::task::noop_waker();
    let mut context = std::task::Context::from_waker(&waker);
    assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    for index in 0..2 {
        let code = format!("TEST_CODE_BATCH_MISSING_{index}");
        provider.release(&code, DeferredReply::Returned(provider_raw(&code)));
        let poll = future.as_mut().poll(&mut context);
        if index == 0 {
            assert!(matches!(poll, Poll::Pending));
        } else {
            let error = match poll {
                Poll::Ready(error) => error,
                Poll::Pending => panic!("cache SQL failure must stop after collection"),
            };
            assert!(matches!(
                error.downcast_ref::<PreparationStop>(),
                Some(PreparationStop::AuthorityRejected { .. })
            ));
            assert!(matches!(
                error.downcast_ref::<ChainPostCloseError>(),
                Some(ChainPostCloseError::StorageFailed {
                    operation: "cache write",
                })
            ));
        }
    }
    assert_eq!(
        query_count(&observation, "chain_post_close_stage_results"),
        2
    );
    assert_eq!(
        query_count(&observation, "chain_post_close_concept_cache_writes"),
        1
    );
    assert_eq!(
        batch_cache_rows(&observation),
        vec![(
            "TEST_CODE_BATCH_MISSING_0".to_owned(),
            "[\"TEST_CODE_CONCEPT_FOR_TEST_CODE_BATCH_MISSING_0\"]".to_owned(),
        )]
    );
    drop(future);
    let head = local.inspect_run(&intent_id).unwrap().head_version();
    assert_eq!(head, 5);
    drop(local);
    observation.close().unwrap();
    fixture.execute("DROP TRIGGER TEST_CODE_FAIL_SECOND_CACHE;");
    fixture.execute(
        "UPDATE stock_concepts SET concepts='[\"TEST_CODE_SQL_LIVE_MUTATION\"]', \
         updated_at='2026-07-21 15:58:58' WHERE code='TEST_CODE_BATCH_MISSING_0';",
    );

    fixture.reopen();
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .resume_run(
            &intent_id,
            lease_request(
                "TEST_CODE_BATCH_CACHE_SQL_REOPEN",
                8_001,
                12_000,
                Some(head),
            ),
        )
        .unwrap();
    let provider = BatchRawProvider::panic_if_called(database);
    let clock = ControlledClock::new(at(8_100));
    let error = run_public_batch_prepare(&mut local, lease, &provider, &clock, input_stocks).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::ClusterConfiguration,
        })
    ));
    assert!(provider.calls().is_empty());
    drop(local);
    assert_eq!(fixture.count("chain_post_close_stage_results"), 2);
    assert_eq!(fixture.count("chain_post_close_concept_cache_writes"), 2);
    assert_eq!(
        cache_rows_with_time(fixture.connection()),
        vec![
            (
                "TEST_CODE_BATCH_MISSING_0".to_owned(),
                "[\"TEST_CODE_SQL_LIVE_MUTATION\"]".to_owned(),
                "2026-07-21 15:58:58".to_owned(),
            ),
            (
                "TEST_CODE_BATCH_MISSING_1".to_owned(),
                "[\"TEST_CODE_CONCEPT_FOR_TEST_CODE_BATCH_MISSING_1\"]".to_owned(),
                "2026-07-21 15:31:00".to_owned(),
            ),
        ]
    );
}
