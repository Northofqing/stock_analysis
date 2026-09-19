use super::*;

#[path = "chain_post_close_concept_rpc_tests.rs"]
mod concept_rpc_tests;
#[path = "chain_post_close_schema_cache_tests.rs"]
mod schema_cache_tests;
#[path = "chain_post_close_success_provider_tests.rs"]
mod success_provider_tests;
#[path = "chain_post_close_terminal_error_tests.rs"]
mod terminal_error_tests;
#[path = "chain_post_close_v6_legacy_migration_tests.rs"]
mod v6_legacy_migration_tests;
use crate::data_gateway::{grpc_source::GrpcSource, BoardKind};
use crate::database::data_acquisition_audit::{
    install_acquisition_schema_for_test, read_acquisition_in_transaction,
};
use crate::grpc_client::client::board_loopback_fixture::{
    clone_with_invalid_instance_bearer, spawn_board_loopback, CONCEPT_DIRECTORY_BYTES,
    INDUSTRY_DIRECTORY_BYTES,
};
use crate::grpc_client::errors::GrpcError;
use crate::pipeline::chain_analysis::preparation::{
    FixedClusterConfiguration, PreparationFailure, PreparationStage, SourceStatus,
};
use diesel::{Connection as _, SqliteConnection};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;

use rusqlite::OpenFlags;

// SHA-256(BR159_DATA_GATEWAY_REQUEST_V1\0board-directory\0{kind}:10000).
const BOARD_REQUEST_HASHES: [&str; 2] = [
    "a60e8f768348b8c1cffcc9127857b61f109c7e5fc60f55fbddda98169d1d420f",
    "fe96ce4cfcabd4b4ca6fd5589792b7b1cb990ef3f445bdf688c6993950b0f6e5",
];
const BOARD_BATCH_IDS: [&str; 2] = ["TEST_CODE_INDUSTRY_BATCH", "TEST_CODE_CONCEPT_BATCH"];

fn install_br159_in_owned_database(fixture: &mut V2BusinessFixture) {
    let store = fixture.store.take().expect("TEST_CODE owned intent store");
    store
        .connection
        .close()
        .expect("TEST_CODE close store before audit installation");
    let database = fixture.database();
    let path = database
        .to_str()
        .expect("TEST_CODE UTF-8 owned business database");
    let mut connection =
        SqliteConnection::establish(path).expect("TEST_CODE owned Diesel schema connection");
    install_acquisition_schema_for_test(&mut connection)
        .expect("TEST_CODE install original BR159 schema");
    drop(connection);
    fixture.store =
        Some(BusinessIntentStore::open(&database).expect("TEST_CODE reopen intent store"));
}

fn rows(connection: &Connection, sql: &str, width: usize) -> Vec<Vec<rusqlite::types::Value>> {
    let mut statement = connection.prepare(sql).expect("TEST_CODE board snapshot");
    let mapped = statement
        .query_map([], |row| {
            (0..width)
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
        })
        .expect("TEST_CODE board snapshot query");
    mapped
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("TEST_CODE board snapshot rows")
}

fn audit_snapshot(fixture: &V2BusinessFixture) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    vec![
        rows(
            fixture.connection(),
            "SELECT * FROM data_acquisition_audit ORDER BY id",
            16,
        ),
        rows(
            fixture.connection(),
            "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id",
            4,
        ),
    ]
}

fn all_rows(connection: &Connection, sql: &str) -> Vec<Vec<rusqlite::types::Value>> {
    let mut statement = connection
        .prepare(sql)
        .expect("TEST_CODE board full fact snapshot");
    let width = statement.column_count();
    let mapped = statement
        .query_map([], |row| {
            (0..width)
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
        })
        .expect("TEST_CODE board full fact snapshot query");
    mapped
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("TEST_CODE board full fact snapshot rows")
}

#[derive(Clone, Debug, PartialEq)]
struct BoardDurableFacts {
    begins: Vec<Vec<rusqlite::types::Value>>,
    results: Vec<Vec<rusqlite::types::Value>>,
    finals: Vec<Vec<rusqlite::types::Value>>,
    directories: Vec<Vec<rusqlite::types::Value>>,
    selections: Vec<Vec<rusqlite::types::Value>>,
    audits: Vec<Vec<rusqlite::types::Value>>,
    audit_chain: Vec<Vec<rusqlite::types::Value>>,
}

fn board_durable_facts(connection: &Connection) -> BoardDurableFacts {
    BoardDurableFacts {
        begins: all_rows(
            connection,
            "SELECT * FROM chain_post_close_board_attempt_begins \
             ORDER BY CASE kind WHEN 'Industry' THEN 0 ELSE 1 END,attempt_ordinal",
        ),
        results: all_rows(
            connection,
            "SELECT * FROM chain_post_close_board_attempt_results \
             ORDER BY CASE kind WHEN 'Industry' THEN 0 ELSE 1 END,attempt_ordinal",
        ),
        finals: all_rows(
            connection,
            "SELECT * FROM chain_post_close_board_kind_finals \
             ORDER BY CASE kind WHEN 'Industry' THEN 0 ELSE 1 END",
        ),
        directories: all_rows(
            connection,
            "SELECT * FROM chain_post_close_board_directory_materials ORDER BY intent_id",
        ),
        selections: all_rows(
            connection,
            "SELECT * FROM chain_post_close_board_selections ORDER BY intent_id,cluster_ordinal",
        ),
        audits: all_rows(
            connection,
            "SELECT * FROM data_acquisition_audit ORDER BY id",
        ),
        audit_chain: all_rows(
            connection,
            "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id",
        ),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConfirmedRetryFact {
    request_id: String,
    request_bytes: Vec<u8>,
    request_digest: String,
    begin_version: i64,
    result_version: i64,
    result_bytes: Vec<u8>,
    result_digest: String,
}

fn confirmed_retry_fact(connection: &Connection) -> ConfirmedRetryFact {
    connection
        .query_row(
            "SELECT begin.request_id,begin.request_bytes,begin.request_sha256,begin.run_version, \
                    result.run_version,result.result_bytes,result.result_sha256 \
             FROM chain_post_close_board_attempt_begins AS begin \
             JOIN chain_post_close_board_attempt_results AS result \
               ON result.intent_id=begin.intent_id AND result.kind=begin.kind \
              AND result.attempt_ordinal=begin.attempt_ordinal \
             WHERE begin.kind='Industry' AND begin.attempt_ordinal=1 \
               AND result.continuation='Retry'",
            [],
            |row| {
                Ok(ConfirmedRetryFact {
                    request_id: row.get(0)?,
                    request_bytes: row.get(1)?,
                    request_digest: row.get(2)?,
                    begin_version: row.get(3)?,
                    result_version: row.get(4)?,
                    result_bytes: row.get(5)?,
                    result_digest: row.get(6)?,
                })
            },
        )
        .expect("TEST_CODE confirmed Industry retry fact")
}

fn only_first_retry_is_committed(connection: &Connection) -> bool {
    let result = connection.query_row(
        "SELECT EXISTS( \
                 SELECT 1 FROM chain_post_close_board_attempt_results \
                 WHERE kind='Industry' AND attempt_ordinal=1 AND wire_outcome='Status' \
                   AND continuation='Retry' AND retry_decision='RetryBackoff' AND backoff_ms=1000 \
             ) AND NOT EXISTS( \
                 SELECT 1 FROM chain_post_close_board_attempt_begins \
                 WHERE kind='Industry' AND attempt_ordinal=2 \
             ) AND NOT EXISTS(SELECT 1 FROM chain_post_close_board_kind_finals)",
        [],
        |row| row.get(0),
    );
    match result {
        Ok(ready) => ready,
        Err(rusqlite::Error::SqliteFailure(error, _))
            if matches!(
                error.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) =>
        {
            false
        }
        Err(error) => panic!("TEST_CODE inspect committed retry: {error}"),
    }
}

fn run_head(connection: &Connection) -> u64 {
    let value = connection
        .query_row(
            "SELECT head_version FROM chain_post_close_runs",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("TEST_CODE board run head");
    u64::try_from(value).expect("TEST_CODE non-negative board run head")
}

struct BoardFinalCommitClock {
    now: i64,
    reader: RefCell<Option<Connection>>,
    armed: Cell<bool>,
}

impl BoardFinalCommitClock {
    fn new(now: i64, database: PathBuf) -> Self {
        let reader = Connection::open_with_flags(
            database,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .expect("TEST_CODE open pre-existing board commit reader");
        reader
            .busy_timeout(std::time::Duration::ZERO)
            .expect("TEST_CODE board commit reader zero busy timeout");
        let journal_mode: String = reader
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("TEST_CODE board commit reader journal mode");
        assert_eq!(journal_mode.to_ascii_lowercase(), "delete");
        Self {
            now,
            reader: RefCell::new(Some(reader)),
            armed: Cell::new(false),
        }
    }

    fn is_armed(&self) -> bool {
        self.armed.get()
    }

    fn facts(&self) -> BoardDurableFacts {
        let reader = self.reader.borrow();
        board_durable_facts(
            reader
                .as_ref()
                .expect("TEST_CODE board commit reader remains open"),
        )
    }

    fn head(&self) -> u64 {
        let reader = self.reader.borrow();
        let head = reader
            .as_ref()
            .expect("TEST_CODE board commit reader remains open")
            .query_row(
                "SELECT head_version FROM chain_post_close_runs",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("TEST_CODE board commit reader head");
        u64::try_from(head).expect("TEST_CODE non-negative board head")
    }

    fn release(&self) {
        let reader = self
            .reader
            .borrow_mut()
            .take()
            .expect("TEST_CODE board commit fault must retain reader");
        reader
            .execute_batch("ROLLBACK;")
            .expect("TEST_CODE release board commit read lock");
        reader.close().expect("TEST_CODE close board commit reader");
    }
}

impl ConceptEffectClock for BoardFinalCommitClock {
    fn now(&self) -> UtcMicros {
        if !self.armed.get() {
            let reader = self.reader.borrow();
            let reader = reader
                .as_ref()
                .expect("TEST_CODE board commit reader remains open");
            let concept_result_ready = reader
                .query_row(
                    "SELECT EXISTS( \
                       SELECT 1 FROM chain_post_close_board_attempt_results \
                       WHERE kind='Concept' AND continuation='Terminal' \
                     ) AND NOT EXISTS( \
                       SELECT 1 FROM chain_post_close_board_kind_finals WHERE kind='Concept' \
                     )",
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .expect("TEST_CODE locate confirmed Concept response");
            if concept_result_ready {
                reader
                    .execute_batch("BEGIN DEFERRED;")
                    .expect("TEST_CODE begin board commit read lock");
                reader
                    .query_row(
                        "SELECT count(*) FROM chain_post_close_board_attempt_results",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .expect("TEST_CODE establish board commit read lock");
                self.armed.set(true);
            }
        }
        UtcMicros::try_new(self.now).expect("TEST_CODE board commit clock")
    }
}

#[derive(Clone, Copy)]
enum BoardAttemptCommitPoint {
    Begin,
    Result,
}

struct BoardAttemptCommitClock {
    now: i64,
    point: BoardAttemptCommitPoint,
    reader: RefCell<Option<Connection>>,
    armed: Cell<bool>,
    locked_head: Cell<Option<u64>>,
    locked_facts: RefCell<Option<BoardDurableFacts>>,
}

impl BoardAttemptCommitClock {
    fn new(now: i64, database: PathBuf, point: BoardAttemptCommitPoint) -> Self {
        let reader = Connection::open_with_flags(
            database,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .expect("TEST_CODE open board attempt commit reader");
        reader
            .busy_timeout(std::time::Duration::ZERO)
            .expect("TEST_CODE board attempt reader zero busy timeout");
        let journal_mode: String = reader
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("TEST_CODE board attempt reader journal mode");
        assert_eq!(journal_mode.to_ascii_lowercase(), "delete");
        Self {
            now,
            point,
            reader: RefCell::new(Some(reader)),
            armed: Cell::new(false),
            locked_head: Cell::new(None),
            locked_facts: RefCell::new(None),
        }
    }

    fn is_armed(&self) -> bool {
        self.armed.get()
    }

    fn head(&self) -> u64 {
        let reader = self.reader.borrow();
        run_head(
            reader
                .as_ref()
                .expect("TEST_CODE board attempt reader remains open"),
        )
    }

    fn facts(&self) -> BoardDurableFacts {
        let reader = self.reader.borrow();
        board_durable_facts(
            reader
                .as_ref()
                .expect("TEST_CODE board attempt reader remains open"),
        )
    }

    fn locked_head(&self) -> u64 {
        self.locked_head
            .get()
            .expect("TEST_CODE board attempt clock captured head")
    }

    fn locked_facts(&self) -> BoardDurableFacts {
        self.locked_facts
            .borrow()
            .as_ref()
            .expect("TEST_CODE board attempt clock captured facts")
            .clone()
    }

    fn release(&self) {
        let reader = self
            .reader
            .borrow_mut()
            .take()
            .expect("TEST_CODE board attempt fault retains reader");
        reader
            .execute_batch("ROLLBACK;")
            .expect("TEST_CODE release board attempt commit lock");
        reader
            .close()
            .expect("TEST_CODE close board attempt commit reader");
    }
}

impl ConceptEffectClock for BoardAttemptCommitClock {
    fn now(&self) -> UtcMicros {
        if !self.armed.get() {
            let reader = self.reader.borrow();
            let reader = reader
                .as_ref()
                .expect("TEST_CODE board attempt reader remains open");
            let ready = match self.point {
                BoardAttemptCommitPoint::Begin => reader
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM chain_post_close_chain_daily_applications) \
                           AND NOT EXISTS(SELECT 1 FROM chain_post_close_board_attempt_begins)",
                        [],
                        |row| row.get::<_, bool>(0),
                    )
                    .expect("TEST_CODE locate first board begin boundary"),
                BoardAttemptCommitPoint::Result => reader
                    .query_row(
                        "SELECT EXISTS( \
                             SELECT 1 FROM chain_post_close_board_attempt_begins \
                             WHERE kind='Industry' AND attempt_ordinal=1 \
                           ) AND NOT EXISTS(SELECT 1 FROM chain_post_close_board_attempt_results)",
                        [],
                        |row| row.get::<_, bool>(0),
                    )
                    .expect("TEST_CODE locate first board result boundary"),
            };
            if ready {
                self.locked_head.set(Some(run_head(reader)));
                *self.locked_facts.borrow_mut() = Some(board_durable_facts(reader));
                reader
                    .execute_batch("BEGIN DEFERRED;")
                    .expect("TEST_CODE begin board attempt commit read lock");
                reader
                    .query_row("SELECT count(*) FROM chain_post_close_runs", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .expect("TEST_CODE establish board attempt commit read lock");
                self.armed.set(true);
            }
        }
        UtcMicros::try_new(self.now).expect("TEST_CODE board attempt commit clock")
    }
}

fn assert_candidates_completed_before_positions(
    error: &anyhow::Error,
    expected_directory: &BTreeMap<String, String>,
    expected_selected: &BTreeMap<String, String>,
) {
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::Positions,
        })
    ));
    let failure = error
        .downcast_ref::<PreparationFailure>()
        .expect("TEST_CODE positions stop retains preparation observations");
    assert_eq!(failure.stage(), PreparationStage::Positions);
    assert_eq!(
        failure.completed_stages().last(),
        Some(&PreparationStage::Candidates)
    );
    let candidate_source = failure
        .candidate_sources()
        .get(cluster_tests::MAIN_CONCEPT)
        .expect("TEST_CODE main cluster candidate source");
    assert_eq!(candidate_source.status(), &SourceStatus::Unavailable);
    assert!(candidate_source
        .reason()
        .expect("TEST_CODE unsupported candidate reason")
        .contains("unsupported"));
    assert_eq!(failure.board_directory(), expected_directory);
    assert_eq!(failure.candidate_board_codes(), expected_selected);
}

#[tokio::test]
async fn board_directory_attempts_and_final_audits_recover_without_rpc_or_rewrite() {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v2_to_v3()
            .expect("TEST_CODE migrate owned board fixture to v3")
            .schema_version(),
        3
    );
    cluster_tests::install_business_rows(&fixture);
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v3_to_v4()
            .expect("TEST_CODE migrate owned board fixture to v4")
            .schema_version(),
        4
    );
    install_br159_in_owned_database(&mut fixture);
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v4_to_v5()
            .expect("TEST_CODE migrate owned board fixture to v5")
            .schema_version(),
        5
    );

    let (client, server) = spawn_board_loopback().await;
    let source = GrpcSource::from_board_loopback_test_client(client);
    let stocks = cluster_tests::cluster_stocks();
    let expected_directory = BTreeMap::from([
        (
            "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
            "TEST_CODE_BOARD_MAIN".to_owned(),
        ),
        (
            "TEST_CODE_CONCEPT_ONLY".to_owned(),
            "TEST_CODE_BOARD_CONCEPT_ONLY".to_owned(),
        ),
        (
            "TEST_CODE_INDUSTRY_ONLY".to_owned(),
            "TEST_CODE_BOARD_INDUSTRY_ONLY".to_owned(),
        ),
    ]);
    let expected_selected = BTreeMap::from([(
        cluster_tests::MAIN_CONCEPT.to_owned(),
        "TEST_CODE_BOARD_MAIN".to_owned(),
    )]);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_BOARD_DIRECTORY"),
    )
    .expect("TEST_CODE build board run context");
    let mut local = fixture
        .store
        .as_mut()
        .expect("TEST_CODE board store")
        .single_user_local_chain_post_close(&config)
        .expect("TEST_CODE local board facade");
    let lease = local
        .acquire_run(
            context,
            fixed_input(stocks.clone()),
            lease_request("TEST_CODE_BOARD_OWNER_A", 1_000, 5_000, None),
        )
        .expect("TEST_CODE acquire board run");
    let intent_id = lease.intent_id().clone();
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .board_preparation_io(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .expect("TEST_CODE construct board preparation adapter");
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).expect("TEST_CODE business date"),
            stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE first board prepare timeout")
    .expect_err("TEST_CODE v5 must stop before unmigrated positions");
    assert_candidates_completed_before_positions(&error, &expected_directory, &expected_selected);
    assert_eq!(provider.calls.get(), 0);
    drop(io);

    let observation = server.snapshot();
    assert_eq!(observation.requests.len(), 3);
    assert_eq!(
        observation
            .requests
            .iter()
            .map(|request| request.kind.as_str())
            .collect::<Vec<_>>(),
        ["Industry", "Industry", "Concept"]
    );
    assert!(observation
        .requests
        .iter()
        .all(|request| request.authorized));
    assert!(observation.requests.iter().all(|request| {
        request.limit == 10_000
            && request.protocol_version == 1
            && request.payload_schema == "board.directory"
            && request.payload_schema_version == 1
            && request.payload_content_type == "application/json; charset=utf-8"
            && !request.allow_unadmitted
    }));
    assert_eq!(
        observation.requests[0].request_id,
        observation.requests[1].request_id
    );
    assert_ne!(
        observation.requests[1].request_id,
        observation.requests[2].request_id
    );
    assert_eq!(observation.non_board_requests, 0);

    let recovery = local
        .inspect_board_directory(&intent_id)
        .expect("TEST_CODE inspect durable board facts");
    assert_eq!(recovery.attempts().len(), 3);
    for attempt in recovery.attempts() {
        assert!(attempt.is_confirmed());
    }
    assert_eq!(recovery.attempts()[0].kind(), BoardKind::Industry);
    assert_eq!(recovery.attempts()[0].attempt_ordinal(), 1);
    assert_eq!(
        recovery.attempts()[0].request_id(),
        recovery.attempts()[1].request_id()
    );
    assert!(recovery.attempts()[0].payload_bytes().is_none());
    assert_eq!(
        recovery.attempts()[0].error_detail_bytes(),
        Some(observation.retry_error_detail.as_slice())
    );
    assert_eq!(recovery.attempts()[1].kind(), BoardKind::Industry);
    assert_eq!(recovery.attempts()[1].attempt_ordinal(), 2);
    assert_eq!(
        recovery.attempts()[1].payload_bytes(),
        Some(INDUSTRY_DIRECTORY_BYTES)
    );
    assert_eq!(recovery.attempts()[2].kind(), BoardKind::Concept);
    assert_eq!(recovery.attempts()[2].attempt_ordinal(), 1);
    assert_eq!(
        recovery.attempts()[2].payload_bytes(),
        Some(CONCEPT_DIRECTORY_BYTES)
    );

    assert_eq!(recovery.board_directory(), &expected_directory);
    assert_eq!(recovery.selected_board_codes(), &expected_selected);
    assert_eq!(recovery.directories().len(), 2);
    assert_eq!(recovery.directories()[0].kind(), BoardKind::Industry);
    assert_eq!(recovery.directories()[1].kind(), BoardKind::Concept);
    let receipts = recovery
        .directories()
        .iter()
        .map(|directory| directory.receipt().clone())
        .collect::<Vec<_>>();
    assert_eq!(receipts[0].audit_id, 1);
    assert_eq!(receipts[0].previous_outcome, None);
    assert_eq!(receipts[0].current_outcome, "available");
    assert_eq!(receipts[1].audit_id, 2);
    assert_eq!(receipts[1].previous_outcome.as_deref(), Some("available"));
    assert_eq!(receipts[1].current_outcome, "available");

    let attempt_fact_bytes = recovery
        .attempts()
        .iter()
        .map(|attempt| attempt.fact_bytes().to_vec())
        .collect::<Vec<_>>();
    let directory_fact_bytes = recovery
        .directories()
        .iter()
        .map(|directory| directory.fact_bytes().to_vec())
        .collect::<Vec<_>>();
    let selection_fact_bytes = recovery.selection_fact_bytes().to_vec();
    let head = local
        .inspect_run(&intent_id)
        .expect("TEST_CODE inspect board run")
        .head_version();
    drop(recovery);
    drop(local);

    assert_eq!(fixture.count("data_acquisition_audit"), 2);
    assert_eq!(fixture.count("data_acquisition_audit_chain"), 2);
    let transaction = fixture
        .connection()
        .unchecked_transaction()
        .expect("TEST_CODE board audit read transaction");
    for (index, receipt) in receipts.iter().enumerate() {
        let verified = read_acquisition_in_transaction(&transaction, receipt)
            .expect("TEST_CODE original BR159 reader accepts board receipt");
        let record = verified.record();
        assert_eq!(record.capability, "board-directory");
        assert_eq!(record.provider, "Tdx");
        assert_eq!(record.source, "TEST_CODE_LOOPBACK_BOARD_SOURCE");
        assert_eq!(record.request_hash, BOARD_REQUEST_HASHES[index]);
        assert_eq!(record.source_at, Some("2026-07-21T15:30:00+08:00"));
        assert_eq!(record.observed_at, "2026-07-21T15:31:00+08:00");
        assert_eq!(record.outcome, "available");
        assert_eq!(record.request_count, 1);
        assert_eq!(record.accepted_count, 2);
        assert_eq!(record.rejected_count, 0);
        assert_eq!(record.reason_code, "accepted");
        assert!(!record.retryable);
        assert_eq!(record.batch_id, Some(BOARD_BATCH_IDS[index]));
    }
    transaction
        .rollback()
        .expect("TEST_CODE end board audit read transaction");
    let audit_rows = audit_snapshot(&fixture);

    fixture.reopen();
    let mut local = fixture
        .store
        .as_mut()
        .expect("TEST_CODE reopened board store")
        .single_user_local_chain_post_close(&config)
        .expect("TEST_CODE reopened local board facade");
    let lease = local
        .resume_run(
            &intent_id,
            lease_request("TEST_CODE_BOARD_OWNER_B", 5_001, 9_000, Some(head)),
        )
        .expect("TEST_CODE resume board run");
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let clock = ControlledClock::new(at(5_100));
    let mut io = local
        .board_preparation_io(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .expect("TEST_CODE construct reopened board adapter");
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).expect("TEST_CODE reopened business date"),
            stocks,
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE reopened board prepare timeout")
    .expect_err("TEST_CODE reopened v5 must stop before positions");
    assert_candidates_completed_before_positions(&error, &expected_directory, &expected_selected);
    assert_eq!(provider.calls.get(), 0);
    drop(io);
    let recovery = local
        .inspect_board_directory(&intent_id)
        .expect("TEST_CODE inspect reopened board facts");
    assert_eq!(recovery.board_directory(), &expected_directory);
    assert_eq!(recovery.selected_board_codes(), &expected_selected);
    assert_eq!(
        recovery
            .attempts()
            .iter()
            .map(|attempt| attempt.fact_bytes().to_vec())
            .collect::<Vec<_>>(),
        attempt_fact_bytes
    );
    assert_eq!(
        recovery
            .directories()
            .iter()
            .map(|directory| directory.fact_bytes().to_vec())
            .collect::<Vec<_>>(),
        directory_fact_bytes
    );
    assert_eq!(recovery.selection_fact_bytes(), selection_fact_bytes);
    assert_eq!(
        recovery
            .directories()
            .iter()
            .map(|directory| directory.receipt())
            .collect::<Vec<_>>(),
        receipts.iter().collect::<Vec<_>>()
    );
    drop(recovery);
    drop(local);
    assert_eq!(audit_snapshot(&fixture), audit_rows);

    let observation = server.finish().await;
    assert_eq!(observation.requests.len(), 3);
    assert_eq!(observation.non_board_requests, 0);
}

#[tokio::test]
async fn confirmed_board_response_recovers_after_final_audit_commit_failure_without_rpc() {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    fixture
        .chain_post_close()
        .migrate_schema_v2_to_v3()
        .expect("TEST_CODE migrate commit fault fixture to v3");
    cluster_tests::install_business_rows(&fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v3_to_v4()
        .expect("TEST_CODE migrate commit fault fixture to v4");
    install_br159_in_owned_database(&mut fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v4_to_v5()
        .expect("TEST_CODE migrate commit fault fixture to v5");
    let journal_mode: String = fixture
        .connection()
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .expect("TEST_CODE board commit fault DELETE journal");
    assert_eq!(journal_mode.to_ascii_lowercase(), "delete");
    fixture
        .connection()
        .busy_timeout(std::time::Duration::ZERO)
        .expect("TEST_CODE board writer zero busy timeout");

    let database = fixture.database();
    let clock = BoardFinalCommitClock::new(at(1_100), database.clone());
    let (client, server) = spawn_board_loopback().await;
    let source = GrpcSource::from_board_loopback_test_client(client);
    let stocks = cluster_tests::cluster_stocks();
    let expected_directory = BTreeMap::from([
        (
            "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
            "TEST_CODE_BOARD_MAIN".to_owned(),
        ),
        (
            "TEST_CODE_CONCEPT_ONLY".to_owned(),
            "TEST_CODE_BOARD_CONCEPT_ONLY".to_owned(),
        ),
        (
            "TEST_CODE_INDUSTRY_ONLY".to_owned(),
            "TEST_CODE_BOARD_INDUSTRY_ONLY".to_owned(),
        ),
    ]);
    let expected_selected = BTreeMap::from([(
        cluster_tests::MAIN_CONCEPT.to_owned(),
        "TEST_CODE_BOARD_MAIN".to_owned(),
    )]);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_BOARD_FINAL_COMMIT_FAILURE"),
    )
    .expect("TEST_CODE build board commit fault context");
    let mut local = fixture
        .store
        .as_mut()
        .expect("TEST_CODE board commit fault store")
        .single_user_local_chain_post_close(&config)
        .expect("TEST_CODE local board commit fault facade");
    let lease = local
        .acquire_run(
            context,
            fixed_input(stocks.clone()),
            lease_request("TEST_CODE_BOARD_COMMIT_OWNER_A", 1_000, 5_000, None),
        )
        .expect("TEST_CODE acquire board commit fault run");
    let intent_id = lease.intent_id().clone();
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let mut io = local
        .board_preparation_io(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .expect("TEST_CODE construct board commit fault adapter");
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).expect("TEST_CODE commit fault business date"),
            stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE board final commit fault timeout")
    .expect_err("TEST_CODE board final commit must stop preparation");
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::ResultUnconfirmed { .. })
    ));
    assert!(matches!(
        error.downcast_ref::<ChainPostCloseError>(),
        Some(ChainPostCloseError::StorageFailed {
            operation: "commit",
        })
    ));
    assert!(clock.is_armed());
    assert_eq!(provider.calls.get(), 0);
    drop(io);

    let failed_facts = clock.facts();
    let failed_head = clock.head();
    assert_eq!(failed_facts.begins.len(), 3);
    assert_eq!(failed_facts.results.len(), 3);
    assert_eq!(failed_facts.finals.len(), 1);
    assert_eq!(failed_facts.audits.len(), 1);
    assert_eq!(failed_facts.audit_chain.len(), 1);
    assert!(failed_facts.directories.is_empty());
    assert!(failed_facts.selections.is_empty());
    let observation = server.snapshot();
    assert_eq!(observation.requests.len(), 3);
    assert_eq!(observation.non_board_requests, 0);

    clock.release();
    assert_eq!(
        local
            .inspect_run(&intent_id)
            .expect("TEST_CODE inspect failed board run")
            .head_version(),
        failed_head
    );
    drop(local);
    assert_eq!(board_durable_facts(fixture.connection()), failed_facts);

    fixture.reopen();
    assert_eq!(board_durable_facts(fixture.connection()), failed_facts);
    let mut local = fixture
        .store
        .as_mut()
        .expect("TEST_CODE reopened board commit fault store")
        .single_user_local_chain_post_close(&config)
        .expect("TEST_CODE reopened board commit fault facade");
    let lease = local
        .resume_run(
            &intent_id,
            lease_request(
                "TEST_CODE_BOARD_COMMIT_OWNER_B",
                5_001,
                9_000,
                Some(failed_head),
            ),
        )
        .expect("TEST_CODE resume board commit fault run");
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let recovery_clock = ControlledClock::new(at(5_100));
    let mut io = local
        .board_preparation_io(
            lease,
            &provider,
            &recovery_clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .expect("TEST_CODE construct recovered board adapter");
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).expect("TEST_CODE recovery business date"),
            stocks,
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE recovered board prepare timeout")
    .expect_err("TEST_CODE recovered board prepare stops at positions");
    assert_candidates_completed_before_positions(&error, &expected_directory, &expected_selected);
    assert_eq!(provider.calls.get(), 0);
    drop(io);

    let recovery = local
        .inspect_board_directory(&intent_id)
        .expect("TEST_CODE inspect recovered board directory");
    assert_eq!(recovery.attempts().len(), 3);
    assert!(recovery
        .attempts()
        .iter()
        .all(|attempt| attempt.is_confirmed()));
    assert_eq!(recovery.directories().len(), 2);
    assert_eq!(recovery.board_directory(), &expected_directory);
    assert_eq!(recovery.selected_board_codes(), &expected_selected);
    let receipts = recovery
        .directories()
        .iter()
        .map(|directory| directory.receipt().clone())
        .collect::<Vec<_>>();
    drop(recovery);
    drop(local);

    let recovered_facts = board_durable_facts(fixture.connection());
    assert_eq!(recovered_facts.begins, failed_facts.begins);
    assert_eq!(recovered_facts.results, failed_facts.results);
    assert_eq!(recovered_facts.finals.len(), 2);
    assert_eq!(recovered_facts.finals[0], failed_facts.finals[0]);
    assert_eq!(recovered_facts.audits.len(), 2);
    assert_eq!(recovered_facts.audits[0], failed_facts.audits[0]);
    assert_eq!(recovered_facts.audit_chain.len(), 2);
    assert_eq!(recovered_facts.audit_chain[0], failed_facts.audit_chain[0]);
    assert_eq!(recovered_facts.directories.len(), 1);
    assert_eq!(recovered_facts.selections.len(), 1);
    let transaction = fixture
        .connection()
        .unchecked_transaction()
        .expect("TEST_CODE recovered board audit transaction");
    assert_eq!(receipts[0].audit_id, 1);
    assert_eq!(receipts[0].previous_outcome, None);
    assert_eq!(receipts[0].current_outcome, "available");
    assert_eq!(receipts[1].audit_id, 2);
    assert_eq!(receipts[1].previous_outcome.as_deref(), Some("available"));
    assert_eq!(receipts[1].current_outcome, "available");
    for (index, receipt) in receipts.iter().enumerate() {
        let verified = read_acquisition_in_transaction(&transaction, receipt)
            .expect("TEST_CODE original reader verifies recovered board receipt");
        let record = verified.record();
        assert_eq!(record.capability, "board-directory");
        assert_eq!(record.provider, "Tdx");
        assert_eq!(record.source, "TEST_CODE_LOOPBACK_BOARD_SOURCE");
        assert_eq!(record.request_hash, BOARD_REQUEST_HASHES[index]);
        assert_eq!(record.source_at, Some("2026-07-21T15:30:00+08:00"));
        assert_eq!(record.observed_at, "2026-07-21T15:31:00+08:00");
        assert_eq!(record.outcome, "available");
        assert_eq!(record.request_count, 1);
        assert_eq!(record.accepted_count, 2);
        assert_eq!(record.rejected_count, 0);
        assert_eq!(record.reason_code, "accepted");
        assert!(!record.retryable);
        assert_eq!(record.batch_id, Some(BOARD_BATCH_IDS[index]));
    }
    transaction
        .rollback()
        .expect("TEST_CODE end recovered board audit transaction");

    let observation = server.finish().await;
    assert_eq!(observation.requests.len(), 3);
    assert_eq!(observation.non_board_requests, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn confirmed_board_retry_resumes_original_request_after_cancel_and_reopen() {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    fixture
        .chain_post_close()
        .migrate_schema_v2_to_v3()
        .expect("TEST_CODE migrate retry-cancel fixture to v3");
    cluster_tests::install_business_rows(&fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v3_to_v4()
        .expect("TEST_CODE migrate retry-cancel fixture to v4");
    install_br159_in_owned_database(&mut fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v4_to_v5()
        .expect("TEST_CODE migrate retry-cancel fixture to v5");

    let database = fixture.database();
    let observer = Connection::open_with_flags(
        &database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("TEST_CODE open retry-cancel observer");
    observer
        .busy_timeout(std::time::Duration::ZERO)
        .expect("TEST_CODE retry-cancel observer zero timeout");
    let (client, server) = spawn_board_loopback().await;
    let source = GrpcSource::from_board_loopback_test_client(client);
    tokio::time::pause();

    let stocks = cluster_tests::cluster_stocks();
    let expected_directory = BTreeMap::from([
        (
            "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
            "TEST_CODE_BOARD_MAIN".to_owned(),
        ),
        (
            "TEST_CODE_CONCEPT_ONLY".to_owned(),
            "TEST_CODE_BOARD_CONCEPT_ONLY".to_owned(),
        ),
        (
            "TEST_CODE_INDUSTRY_ONLY".to_owned(),
            "TEST_CODE_BOARD_INDUSTRY_ONLY".to_owned(),
        ),
    ]);
    let expected_selected = BTreeMap::from([(
        cluster_tests::MAIN_CONCEPT.to_owned(),
        "TEST_CODE_BOARD_MAIN".to_owned(),
    )]);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_BOARD_RETRY_CANCEL"),
    )
    .expect("TEST_CODE build retry-cancel context");
    let mut local = fixture
        .store
        .as_mut()
        .expect("TEST_CODE retry-cancel store")
        .single_user_local_chain_post_close(&config)
        .expect("TEST_CODE retry-cancel facade");
    let lease = local
        .acquire_run(
            context,
            fixed_input(stocks.clone()),
            lease_request("TEST_CODE_BOARD_RETRY_OWNER_A", 1_000, 5_000, None),
        )
        .expect("TEST_CODE acquire retry-cancel run");
    let intent_id = lease.intent_id().clone();
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .board_preparation_io(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .expect("TEST_CODE construct retry-cancel adapter");
    let mut first_prepare = Box::pin(prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).expect("TEST_CODE retry-cancel business date"),
        stocks.clone(),
        None,
        &mut io,
    ));
    let commit_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !only_first_retry_is_committed(&observer) {
        assert!(
            std::time::Instant::now() < commit_deadline,
            "TEST_CODE timed out waiting for committed Industry retry"
        );
        tokio::select! {
            biased;
            _ = &mut first_prepare => {
                panic!("TEST_CODE first prepare completed before retry cancellation")
            }
            _ = tokio::task::yield_now() => {}
        }
    }
    drop(first_prepare);
    drop(io);
    assert_eq!(provider.calls.get(), 0);

    let first_observation = server.snapshot();
    assert_eq!(first_observation.requests.len(), 1);
    assert_eq!(first_observation.requests[0].kind, "Industry");
    assert_eq!(first_observation.non_board_requests, 0);
    let first_facts = board_durable_facts(&observer);
    assert_eq!(first_facts.begins.len(), 1);
    assert_eq!(first_facts.results.len(), 1);
    assert!(first_facts.finals.is_empty());
    assert!(first_facts.audits.is_empty());
    assert!(first_facts.audit_chain.is_empty());
    assert!(first_facts.directories.is_empty());
    assert!(first_facts.selections.is_empty());
    let first_retry = confirmed_retry_fact(&observer);
    assert_eq!(
        first_retry.request_digest,
        raw_digest(&first_retry.request_bytes).as_str()
    );
    assert_eq!(
        first_retry.result_digest,
        raw_digest(&first_retry.result_bytes).as_str()
    );
    let retry_material: serde_json::Value = serde_json::from_slice(&first_retry.result_bytes)
        .expect("TEST_CODE decode saved retry material");
    assert_eq!(retry_material["response_wire"], serde_json::Value::Null);
    assert_eq!(retry_material["status_code"], 14);
    assert_eq!(
        retry_material["status_details"],
        serde_json::json!(first_observation.retry_error_detail.clone())
    );
    assert_eq!(
        retry_material["status_error_detail_trailer"]["Bytes"],
        serde_json::json!(first_observation.retry_error_detail.clone())
    );
    assert_eq!(retry_material["retry_decision"], "RetryBackoff");
    assert_eq!(retry_material["continuation"], "Retry");
    assert_eq!(retry_material["backoff_ms"], 1_000);
    let cancelled_head = run_head(&observer);
    drop(local);
    observer
        .close()
        .expect("TEST_CODE close retry-cancel observer");

    fixture.reopen();
    assert_eq!(board_durable_facts(fixture.connection()), first_facts);
    assert_eq!(confirmed_retry_fact(fixture.connection()), first_retry);
    let recovery_observer = Connection::open_with_flags(
        &database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("TEST_CODE open retry recovery observer");
    recovery_observer
        .busy_timeout(std::time::Duration::ZERO)
        .expect("TEST_CODE retry recovery observer zero timeout");
    let mut local = fixture
        .store
        .as_mut()
        .expect("TEST_CODE reopened retry store")
        .single_user_local_chain_post_close(&config)
        .expect("TEST_CODE reopened retry facade");
    let lease = local
        .resume_run(
            &intent_id,
            lease_request(
                "TEST_CODE_BOARD_RETRY_OWNER_B",
                5_001,
                2_005_001,
                Some(cancelled_head),
            ),
        )
        .expect("TEST_CODE resume cancelled retry run");
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let recovery_clock = ControlledClock::new(at(5_100));
    let mut io = local
        .board_preparation_io(
            lease,
            &provider,
            &recovery_clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .expect("TEST_CODE construct retry recovery adapter");
    let mut recovered_prepare = Box::pin(prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).expect("TEST_CODE retry recovery business date"),
        stocks,
        None,
        &mut io,
    ));
    let initial_poll = tokio::select! {
        biased;
        result = &mut recovered_prepare => Some(result),
        _ = tokio::task::yield_now() => None,
    };
    assert!(
        initial_poll.is_none(),
        "TEST_CODE confirmed retry must first wait its saved backoff"
    );

    tokio::time::advance(std::time::Duration::from_millis(999)).await;
    // UTC lease time and Tokio's monotonic timer are controlled independently.
    recovery_clock.now.set(at(5_100 + 999_000));
    let before_backoff_deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
    while std::time::Instant::now() < before_backoff_deadline {
        tokio::select! {
            biased;
            _ = &mut recovered_prepare => {
                panic!("TEST_CODE retry resumed before 1000ms backoff")
            }
            _ = tokio::task::yield_now() => {}
        }
        assert!(only_first_retry_is_committed(&recovery_observer));
        assert_eq!(server.snapshot().requests.len(), 1);
    }

    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    recovery_clock.now.set(at(5_100 + 1_000_000));
    let mut completed = tokio::select! {
        biased;
        result = &mut recovered_prepare => Some(result),
        _ = tokio::task::yield_now() => None,
    };
    if completed.is_none() {
        // Tokio timer precision makes the observable boundary [1000ms, 1001ms].
        tokio::time::advance(std::time::Duration::from_millis(1)).await;
        recovery_clock.now.set(at(5_100 + 1_001_000));
    }
    let completion_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while completed.is_none() {
        assert!(
            std::time::Instant::now() < completion_deadline,
            "TEST_CODE timed out completing recovered retry"
        );
        completed = tokio::select! {
            biased;
            result = &mut recovered_prepare => Some(result),
            _ = tokio::task::yield_now() => None,
        };
    }
    let error = completed
        .expect("TEST_CODE recovered retry completion")
        .expect_err("TEST_CODE recovered retry stops at positions");
    assert_candidates_completed_before_positions(&error, &expected_directory, &expected_selected);
    drop(recovered_prepare);
    drop(io);
    assert_eq!(provider.calls.get(), 0);

    let recovery = local
        .inspect_board_directory(&intent_id)
        .expect("TEST_CODE inspect recovered retry board facts");
    assert_eq!(recovery.attempts().len(), 3);
    assert_eq!(recovery.attempts()[0].kind(), BoardKind::Industry);
    assert_eq!(recovery.attempts()[0].attempt_ordinal(), 1);
    assert_eq!(recovery.attempts()[1].kind(), BoardKind::Industry);
    assert_eq!(recovery.attempts()[1].attempt_ordinal(), 2);
    assert_eq!(recovery.attempts()[2].kind(), BoardKind::Concept);
    assert_eq!(recovery.attempts()[2].attempt_ordinal(), 1);
    assert!(recovery
        .attempts()
        .iter()
        .all(|attempt| attempt.is_confirmed()));
    assert_eq!(recovery.board_directory(), &expected_directory);
    assert_eq!(recovery.selected_board_codes(), &expected_selected);
    let receipts = recovery
        .directories()
        .iter()
        .map(|directory| directory.receipt().clone())
        .collect::<Vec<_>>();
    assert_eq!(receipts.len(), 2);
    drop(recovery);
    drop(local);

    let recovered_facts = board_durable_facts(&recovery_observer);
    assert_eq!(recovered_facts.begins.len(), 3);
    assert_eq!(recovered_facts.results.len(), 3);
    assert_eq!(recovered_facts.finals.len(), 2);
    assert_eq!(recovered_facts.audits.len(), 2);
    assert_eq!(recovered_facts.audit_chain.len(), 2);
    assert_eq!(recovered_facts.directories.len(), 1);
    assert_eq!(recovered_facts.selections.len(), 1);
    assert_eq!(recovered_facts.begins[0], first_facts.begins[0]);
    assert_eq!(recovered_facts.results[0], first_facts.results[0]);
    assert_eq!(confirmed_retry_fact(&recovery_observer), first_retry);
    let second_begin = recovery_observer
        .query_row(
            "SELECT previous_result_run_version,previous_result_sha256,lease_owner, \
                    lease_generation,request_bytes,request_sha256 \
             FROM chain_post_close_board_attempt_begins \
             WHERE kind='Industry' AND attempt_ordinal=2",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .expect("TEST_CODE recovered second Industry begin");
    assert_eq!(second_begin.0, first_retry.result_version);
    assert_eq!(second_begin.1, first_retry.result_digest);
    assert_eq!(second_begin.2, "TEST_CODE_BOARD_RETRY_OWNER_B");
    assert_eq!(second_begin.3, 2);
    assert_eq!(second_begin.4, first_retry.request_bytes);
    assert_eq!(second_begin.5, first_retry.request_digest);
    let second_result_authority = recovery_observer
        .query_row(
            "SELECT lease_owner,lease_generation FROM chain_post_close_board_attempt_results \
             WHERE kind='Industry' AND attempt_ordinal=2",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .expect("TEST_CODE recovered second Industry result authority");
    assert_eq!(
        second_result_authority,
        ("TEST_CODE_BOARD_RETRY_OWNER_B".to_owned(), 2)
    );

    let transaction = recovery_observer
        .unchecked_transaction()
        .expect("TEST_CODE recovered retry audit transaction");
    for (index, receipt) in receipts.iter().enumerate() {
        assert_eq!(receipt.audit_id, i64::try_from(index + 1).unwrap());
        assert_eq!(
            receipt.previous_outcome.as_deref(),
            if index == 0 { None } else { Some("available") }
        );
        assert_eq!(receipt.current_outcome, "available");
        let verified = read_acquisition_in_transaction(&transaction, receipt)
            .expect("TEST_CODE original reader verifies recovered retry receipt");
        let record = verified.record();
        assert_eq!(record.capability, "board-directory");
        assert_eq!(record.provider, "Tdx");
        assert_eq!(record.source, "TEST_CODE_LOOPBACK_BOARD_SOURCE");
        assert_eq!(record.request_hash, BOARD_REQUEST_HASHES[index]);
        assert_eq!(record.source_at, Some("2026-07-21T15:30:00+08:00"));
        assert_eq!(record.observed_at, "2026-07-21T15:31:00+08:00");
        assert_eq!(record.outcome, "available");
        assert_eq!(record.request_count, 1);
        assert_eq!(record.accepted_count, 2);
        assert_eq!(record.rejected_count, 0);
        assert_eq!(record.reason_code, "accepted");
        assert!(!record.retryable);
        assert_eq!(record.batch_id, Some(BOARD_BATCH_IDS[index]));
    }
    transaction
        .rollback()
        .expect("TEST_CODE end recovered retry audit transaction");

    let final_observation = server.snapshot();
    assert_eq!(final_observation.requests.len(), 3);
    assert_eq!(
        final_observation
            .requests
            .iter()
            .map(|request| request.kind.as_str())
            .collect::<Vec<_>>(),
        ["Industry", "Industry", "Concept"]
    );
    assert_eq!(
        final_observation.requests[0].request_id,
        final_observation.requests[1].request_id
    );
    assert_eq!(
        final_observation.requests[0].request_id,
        first_retry.request_id
    );
    assert_ne!(
        final_observation.requests[1].request_id,
        final_observation.requests[2].request_id
    );
    assert!(final_observation.requests.iter().all(|request| {
        request.authorized
            && request.limit == 10_000
            && request.protocol_version == 1
            && request.payload_schema == "board.directory"
            && request.payload_schema_version == 1
            && request.payload_content_type == "application/json; charset=utf-8"
            && !request.allow_unadmitted
    }));
    assert_eq!(final_observation.non_board_requests, 0);
    recovery_observer
        .close()
        .expect("TEST_CODE close retry recovery observer");
    tokio::time::resume();
    let finished = server.finish().await;
    assert_eq!(finished.requests, final_observation.requests);
    assert_eq!(finished.non_board_requests, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn confirmed_board_retry_rejects_invalid_resume_credentials_before_new_effects() {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    fixture
        .chain_post_close()
        .migrate_schema_v2_to_v3()
        .expect("TEST_CODE migrate invalid-auth fixture to v3");
    cluster_tests::install_business_rows(&fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v3_to_v4()
        .expect("TEST_CODE migrate invalid-auth fixture to v4");
    install_br159_in_owned_database(&mut fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v4_to_v5()
        .expect("TEST_CODE migrate invalid-auth fixture to v5");

    let database = fixture.database();
    let observer = Connection::open_with_flags(
        &database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("TEST_CODE open invalid-auth observer");
    observer
        .busy_timeout(std::time::Duration::ZERO)
        .expect("TEST_CODE invalid-auth observer zero timeout");
    let (client, server) = spawn_board_loopback().await;
    let invalid_client = clone_with_invalid_instance_bearer(&client);
    let source = GrpcSource::from_board_loopback_test_client(client);
    tokio::time::pause();

    let stocks = cluster_tests::cluster_stocks();
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_BOARD_RETRY_INVALID_AUTH"),
    )
    .expect("TEST_CODE build invalid-auth context");
    let mut local = fixture
        .store
        .as_mut()
        .expect("TEST_CODE invalid-auth store")
        .single_user_local_chain_post_close(&config)
        .expect("TEST_CODE invalid-auth facade");
    let lease = local
        .acquire_run(
            context,
            fixed_input(stocks.clone()),
            lease_request("TEST_CODE_BOARD_AUTH_OWNER_A", 1_000, 5_000, None),
        )
        .expect("TEST_CODE acquire invalid-auth run");
    let intent_id = lease.intent_id().clone();
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .board_preparation_io(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .expect("TEST_CODE construct invalid-auth initial adapter");
    let mut first_prepare = Box::pin(prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).expect("TEST_CODE invalid-auth business date"),
        stocks.clone(),
        None,
        &mut io,
    ));
    let commit_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !only_first_retry_is_committed(&observer) {
        assert!(
            std::time::Instant::now() < commit_deadline,
            "TEST_CODE timed out waiting for invalid-auth Retry fact"
        );
        tokio::select! {
            biased;
            _ = &mut first_prepare => {
                panic!("TEST_CODE initial prepare completed before invalid-auth cancellation")
            }
            _ = tokio::task::yield_now() => {}
        }
    }
    drop(first_prepare);
    drop(io);
    assert_eq!(provider.calls.get(), 0);

    let first_observation = server.snapshot();
    assert_eq!(first_observation.requests.len(), 1);
    assert_eq!(first_observation.requests[0].kind, "Industry");
    assert_eq!(first_observation.non_board_requests, 0);
    let first_facts = board_durable_facts(&observer);
    assert_eq!(first_facts.begins.len(), 1);
    assert_eq!(first_facts.results.len(), 1);
    assert!(first_facts.finals.is_empty());
    assert!(first_facts.audits.is_empty());
    assert!(first_facts.audit_chain.is_empty());
    assert!(first_facts.directories.is_empty());
    assert!(first_facts.selections.is_empty());
    let first_retry = confirmed_retry_fact(&observer);
    assert_eq!(
        first_retry.request_digest,
        raw_digest(&first_retry.request_bytes).as_str()
    );
    assert_eq!(
        first_retry.result_digest,
        raw_digest(&first_retry.result_bytes).as_str()
    );
    let retry_material: serde_json::Value = serde_json::from_slice(&first_retry.result_bytes)
        .expect("TEST_CODE decode invalid-auth Retry material");
    assert_eq!(retry_material["response_wire"], serde_json::Value::Null);
    assert_eq!(retry_material["status_code"], 14);
    assert_eq!(
        retry_material["status_details"],
        serde_json::json!(first_observation.retry_error_detail.clone())
    );
    assert_eq!(
        retry_material["status_error_detail_trailer"]["Bytes"],
        serde_json::json!(first_observation.retry_error_detail.clone())
    );
    assert_eq!(retry_material["retry_decision"], "RetryBackoff");
    assert_eq!(retry_material["continuation"], "Retry");
    assert_eq!(retry_material["backoff_ms"], 1_000);
    let cancelled_head = run_head(&observer);
    drop(local);
    drop(source);
    observer
        .close()
        .expect("TEST_CODE close initial invalid-auth observer");
    tokio::time::resume();

    fixture.reopen();
    assert_eq!(board_durable_facts(fixture.connection()), first_facts);
    assert_eq!(confirmed_retry_fact(fixture.connection()), first_retry);
    let recovery_observer = Connection::open_with_flags(
        &database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("TEST_CODE open invalid-auth recovery observer");
    recovery_observer
        .busy_timeout(std::time::Duration::ZERO)
        .expect("TEST_CODE invalid-auth recovery observer zero timeout");
    let invalid_source = GrpcSource::from_board_loopback_test_client(invalid_client);
    let mut local = fixture
        .store
        .as_mut()
        .expect("TEST_CODE reopened invalid-auth store")
        .single_user_local_chain_post_close(&config)
        .expect("TEST_CODE reopened invalid-auth facade");
    let lease = local
        .resume_run(
            &intent_id,
            lease_request(
                "TEST_CODE_BOARD_AUTH_OWNER_B",
                5_001,
                2_005_001,
                Some(cancelled_head),
            ),
        )
        .expect("TEST_CODE resume invalid-auth run");
    let resumed_head = run_head(&recovery_observer);
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let recovery_clock = ControlledClock::new(at(5_100));
    let mut io = local
        .board_preparation_io(
            lease,
            &provider,
            &recovery_clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &invalid_source,
        )
        .expect("TEST_CODE construct invalid-auth recovery adapter");
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).expect("TEST_CODE invalid-auth recovery date"),
            stocks,
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE invalid-auth recovery timeout")
    .expect_err("TEST_CODE invalid resume credentials must hard-stop");
    drop(io);

    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::AuthorityRejected { intent_id: actual })
            if actual.as_str() == intent_id.as_str()
    ));
    assert!(matches!(
        error.downcast_ref::<GrpcError>(),
        Some(GrpcError::Unauthenticated { .. })
    ));
    let failure = error
        .downcast_ref::<PreparationFailure>()
        .expect("TEST_CODE invalid-auth stop retains preparation observations");
    assert_eq!(failure.stage(), PreparationStage::Candidates);
    assert_eq!(
        failure.completed_stages().last(),
        Some(&PreparationStage::ClusterWritesAndLifecycle)
    );
    assert_eq!(provider.calls.get(), 0);
    assert_eq!(run_head(&recovery_observer), resumed_head);
    assert_eq!(board_durable_facts(&recovery_observer), first_facts);
    assert_eq!(confirmed_retry_fact(&recovery_observer), first_retry);

    let final_observation = server.snapshot();
    assert_eq!(final_observation.requests.len(), 1);
    assert_eq!(final_observation.requests[0].kind, "Industry");
    assert_eq!(final_observation.non_board_requests, 0);
    drop(local);
    recovery_observer
        .close()
        .expect("TEST_CODE close invalid-auth recovery observer");
    let finished = server.finish().await;
    assert_eq!(finished.requests, final_observation.requests);
    assert_eq!(finished.non_board_requests, 0);
}

#[tokio::test]
async fn board_begin_commit_failure_stops_before_rpc_and_reopens_without_phantom_attempt() {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    fixture
        .chain_post_close()
        .migrate_schema_v2_to_v3()
        .expect("TEST_CODE migrate begin-commit fixture to v3");
    cluster_tests::install_business_rows(&fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v3_to_v4()
        .expect("TEST_CODE migrate begin-commit fixture to v4");
    install_br159_in_owned_database(&mut fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v4_to_v5()
        .expect("TEST_CODE migrate begin-commit fixture to v5");
    let journal_mode: String = fixture
        .connection()
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .expect("TEST_CODE begin-commit DELETE journal");
    assert_eq!(journal_mode.to_ascii_lowercase(), "delete");
    fixture
        .connection()
        .busy_timeout(std::time::Duration::ZERO)
        .expect("TEST_CODE begin-commit writer zero timeout");

    let database = fixture.database();
    let clock =
        BoardAttemptCommitClock::new(at(1_100), database.clone(), BoardAttemptCommitPoint::Begin);
    let (client, server) = spawn_board_loopback().await;
    let source = GrpcSource::from_board_loopback_test_client(client);
    let stocks = cluster_tests::cluster_stocks();
    let expected_directory = BTreeMap::from([
        (
            "TEST_CODE_CLUSTER_A_MAIN".to_owned(),
            "TEST_CODE_BOARD_MAIN".to_owned(),
        ),
        (
            "TEST_CODE_CONCEPT_ONLY".to_owned(),
            "TEST_CODE_BOARD_CONCEPT_ONLY".to_owned(),
        ),
        (
            "TEST_CODE_INDUSTRY_ONLY".to_owned(),
            "TEST_CODE_BOARD_INDUSTRY_ONLY".to_owned(),
        ),
    ]);
    let expected_selected = BTreeMap::from([(
        cluster_tests::MAIN_CONCEPT.to_owned(),
        "TEST_CODE_BOARD_MAIN".to_owned(),
    )]);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_BOARD_BEGIN_COMMIT"),
    )
    .expect("TEST_CODE build begin-commit context");
    let mut local = fixture
        .store
        .as_mut()
        .expect("TEST_CODE begin-commit store")
        .single_user_local_chain_post_close(&config)
        .expect("TEST_CODE begin-commit facade");
    let lease = local
        .acquire_run(
            context,
            fixed_input(stocks.clone()),
            lease_request("TEST_CODE_BOARD_BEGIN_OWNER_A", 1_000, 5_000, None),
        )
        .expect("TEST_CODE acquire begin-commit run");
    let intent_id = lease.intent_id().clone();
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let mut io = local
        .board_preparation_io(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .expect("TEST_CODE construct begin-commit adapter");
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).expect("TEST_CODE begin-commit business date"),
            stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE begin-commit prepare timeout")
    .expect_err("TEST_CODE first board begin COMMIT must stop");
    assert!(matches!(
        error.downcast_ref::<ChainPostCloseError>(),
        Some(ChainPostCloseError::StorageFailed {
            operation: "commit",
        })
    ));
    assert!(clock.is_armed());
    let failure = error
        .downcast_ref::<PreparationFailure>()
        .expect("TEST_CODE begin-commit stop retains preparation observations");
    assert_eq!(failure.stage(), PreparationStage::Candidates);
    assert_eq!(
        failure.completed_stages().last(),
        Some(&PreparationStage::ClusterWritesAndLifecycle)
    );
    assert_eq!(provider.calls.get(), 0);
    drop(io);

    let failed_facts = clock.facts();
    let failed_head = clock.head();
    assert_eq!(failed_facts, clock.locked_facts());
    assert_eq!(failed_head, clock.locked_head());
    assert!(failed_facts.begins.is_empty());
    assert!(failed_facts.results.is_empty());
    assert!(failed_facts.finals.is_empty());
    assert!(failed_facts.audits.is_empty());
    assert!(failed_facts.audit_chain.is_empty());
    assert!(failed_facts.directories.is_empty());
    assert!(failed_facts.selections.is_empty());
    let failed_observation = server.snapshot();
    assert!(failed_observation.requests.is_empty());
    assert_eq!(failed_observation.non_board_requests, 0);
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::AuthorityRejected { intent_id: actual })
            if actual.as_str() == intent_id.as_str()
    ));

    clock.release();
    assert_eq!(
        local
            .inspect_run(&intent_id)
            .expect("TEST_CODE inspect begin-commit run")
            .head_version(),
        failed_head
    );
    drop(local);
    assert_eq!(board_durable_facts(fixture.connection()), failed_facts);
    fixture.reopen();
    assert_eq!(board_durable_facts(fixture.connection()), failed_facts);

    let mut local = fixture
        .store
        .as_mut()
        .expect("TEST_CODE reopened begin-commit store")
        .single_user_local_chain_post_close(&config)
        .expect("TEST_CODE reopened begin-commit facade");
    let lease = local
        .resume_run(
            &intent_id,
            lease_request(
                "TEST_CODE_BOARD_BEGIN_OWNER_B",
                5_001,
                2_005_001,
                Some(failed_head),
            ),
        )
        .expect("TEST_CODE resume begin-commit run");
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let recovery_clock = ControlledClock::new(at(5_100));
    let mut io = local
        .board_preparation_io(
            lease,
            &provider,
            &recovery_clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .expect("TEST_CODE construct recovered begin adapter");
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).expect("TEST_CODE recovered begin business date"),
            stocks,
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE recovered begin prepare timeout")
    .expect_err("TEST_CODE recovered begin prepare stops at positions");
    assert_candidates_completed_before_positions(&error, &expected_directory, &expected_selected);
    assert_eq!(provider.calls.get(), 0);
    drop(io);

    let recovery = local
        .inspect_board_directory(&intent_id)
        .expect("TEST_CODE inspect recovered begin board directory");
    assert_eq!(recovery.attempts().len(), 3);
    assert_eq!(recovery.directories().len(), 2);
    assert_eq!(recovery.board_directory(), &expected_directory);
    assert_eq!(recovery.selected_board_codes(), &expected_selected);
    let receipts = recovery
        .directories()
        .iter()
        .map(|directory| directory.receipt().clone())
        .collect::<Vec<_>>();
    drop(recovery);
    drop(local);
    let recovered_facts = board_durable_facts(fixture.connection());
    assert_eq!(recovered_facts.begins.len(), 3);
    assert_eq!(recovered_facts.results.len(), 3);
    assert_eq!(recovered_facts.finals.len(), 2);
    assert_eq!(recovered_facts.audits.len(), 2);
    assert_eq!(recovered_facts.audit_chain.len(), 2);
    assert_eq!(recovered_facts.directories.len(), 1);
    assert_eq!(recovered_facts.selections.len(), 1);
    let transaction = fixture
        .connection()
        .unchecked_transaction()
        .expect("TEST_CODE recovered begin audit transaction");
    for (index, receipt) in receipts.iter().enumerate() {
        assert_eq!(receipt.audit_id, i64::try_from(index + 1).unwrap());
        assert_eq!(
            receipt.previous_outcome.as_deref(),
            if index == 0 { None } else { Some("available") }
        );
        assert_eq!(receipt.current_outcome, "available");
        let verified = read_acquisition_in_transaction(&transaction, receipt)
            .expect("TEST_CODE original reader verifies recovered begin receipt");
        let record = verified.record();
        assert_eq!(record.capability, "board-directory");
        assert_eq!(record.provider, "Tdx");
        assert_eq!(record.source, "TEST_CODE_LOOPBACK_BOARD_SOURCE");
        assert_eq!(record.request_hash, BOARD_REQUEST_HASHES[index]);
        assert_eq!(record.source_at, Some("2026-07-21T15:30:00+08:00"));
        assert_eq!(record.observed_at, "2026-07-21T15:31:00+08:00");
        assert_eq!(record.outcome, "available");
        assert_eq!(record.request_count, 1);
        assert_eq!(record.accepted_count, 2);
        assert_eq!(record.rejected_count, 0);
        assert_eq!(record.reason_code, "accepted");
        assert!(!record.retryable);
        assert_eq!(record.batch_id, Some(BOARD_BATCH_IDS[index]));
    }
    transaction
        .rollback()
        .expect("TEST_CODE end recovered begin audit transaction");
    let observation = server.finish().await;
    assert_eq!(
        observation
            .requests
            .iter()
            .map(|request| request.kind.as_str())
            .collect::<Vec<_>>(),
        ["Industry", "Industry", "Concept"]
    );
    assert_eq!(observation.non_board_requests, 0);
}

#[tokio::test]
async fn board_result_commit_failure_preserves_unconfirmed_attempt_and_never_replays() {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    fixture
        .chain_post_close()
        .migrate_schema_v2_to_v3()
        .expect("TEST_CODE migrate result-commit fixture to v3");
    cluster_tests::install_business_rows(&fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v3_to_v4()
        .expect("TEST_CODE migrate result-commit fixture to v4");
    install_br159_in_owned_database(&mut fixture);
    fixture
        .chain_post_close()
        .migrate_schema_v4_to_v5()
        .expect("TEST_CODE migrate result-commit fixture to v5");
    let journal_mode: String = fixture
        .connection()
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .expect("TEST_CODE result-commit DELETE journal");
    assert_eq!(journal_mode.to_ascii_lowercase(), "delete");
    fixture
        .connection()
        .busy_timeout(std::time::Duration::ZERO)
        .expect("TEST_CODE result-commit writer zero timeout");

    let database = fixture.database();
    let clock =
        BoardAttemptCommitClock::new(at(1_100), database.clone(), BoardAttemptCommitPoint::Result);
    let (client, server) = spawn_board_loopback().await;
    let source = GrpcSource::from_board_loopback_test_client(client);
    let stocks = cluster_tests::cluster_stocks();
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_BOARD_RESULT_COMMIT"),
    )
    .expect("TEST_CODE build result-commit context");
    let mut local = fixture
        .store
        .as_mut()
        .expect("TEST_CODE result-commit store")
        .single_user_local_chain_post_close(&config)
        .expect("TEST_CODE result-commit facade");
    let lease = local
        .acquire_run(
            context,
            fixed_input(stocks.clone()),
            lease_request("TEST_CODE_BOARD_RESULT_OWNER_A", 1_000, 5_000, None),
        )
        .expect("TEST_CODE acquire result-commit run");
    let intent_id = lease.intent_id().clone();
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let mut io = local
        .board_preparation_io(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .expect("TEST_CODE construct result-commit adapter");
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).expect("TEST_CODE result-commit business date"),
            stocks.clone(),
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE result-commit prepare timeout")
    .expect_err("TEST_CODE first board result COMMIT must stop");
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::ResultUnconfirmed { intent_id: actual })
            if actual.as_str() == intent_id.as_str()
    ));
    assert!(matches!(
        error.downcast_ref::<ChainPostCloseError>(),
        Some(ChainPostCloseError::StorageFailed {
            operation: "commit",
        })
    ));
    let failure = error
        .downcast_ref::<PreparationFailure>()
        .expect("TEST_CODE result-commit stop retains preparation observations");
    assert_eq!(failure.stage(), PreparationStage::Candidates);
    assert_eq!(
        failure.completed_stages().last(),
        Some(&PreparationStage::ClusterWritesAndLifecycle)
    );
    assert!(clock.is_armed());
    assert_eq!(provider.calls.get(), 0);
    drop(io);

    let failed_facts = clock.facts();
    let failed_head = clock.head();
    assert_eq!(failed_facts, clock.locked_facts());
    assert_eq!(failed_head, clock.locked_head());
    assert_eq!(failed_facts.begins.len(), 1);
    assert!(failed_facts.results.is_empty());
    assert!(failed_facts.finals.is_empty());
    assert!(failed_facts.audits.is_empty());
    assert!(failed_facts.audit_chain.is_empty());
    assert!(failed_facts.directories.is_empty());
    assert!(failed_facts.selections.is_empty());
    let failed_observation = server.snapshot();
    assert_eq!(failed_observation.requests.len(), 1);
    assert_eq!(failed_observation.requests[0].kind, "Industry");
    assert_eq!(failed_observation.non_board_requests, 0);

    clock.release();
    assert_eq!(
        local
            .inspect_run(&intent_id)
            .expect("TEST_CODE inspect result-commit run")
            .head_version(),
        failed_head
    );
    drop(local);
    assert_eq!(board_durable_facts(fixture.connection()), failed_facts);
    fixture.reopen();
    assert_eq!(board_durable_facts(fixture.connection()), failed_facts);

    let mut local = fixture
        .store
        .as_mut()
        .expect("TEST_CODE reopened result-commit store")
        .single_user_local_chain_post_close(&config)
        .expect("TEST_CODE reopened result-commit facade");
    let lease = local
        .resume_run(
            &intent_id,
            lease_request(
                "TEST_CODE_BOARD_RESULT_OWNER_B",
                5_001,
                2_005_001,
                Some(failed_head),
            ),
        )
        .expect("TEST_CODE resume result-commit run");
    let resumed_head = local
        .inspect_run(&intent_id)
        .expect("TEST_CODE inspect resumed result-commit run")
        .head_version();
    let provider = cluster_tests::PanicRawProvider {
        calls: Cell::new(0),
    };
    let recovery_clock = ControlledClock::new(at(5_100));
    let mut io = local
        .board_preparation_io(
            lease,
            &provider,
            &recovery_clock,
            FixedClusterConfiguration::resolve(Some("2")),
            &source,
        )
        .expect("TEST_CODE construct reopened result adapter");
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).expect("TEST_CODE reopened result business date"),
            stocks,
            None,
            &mut io,
        ),
    )
    .await
    .expect("TEST_CODE reopened result prepare timeout")
    .expect_err("TEST_CODE unconfirmed result must stop after reopen");
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::IncompleteOnReopen { intent_id: actual })
            if actual.as_str() == intent_id.as_str()
    ));
    let failure = error
        .downcast_ref::<PreparationFailure>()
        .expect("TEST_CODE incomplete result retains preparation observations");
    assert_eq!(failure.stage(), PreparationStage::Candidates);
    assert_eq!(
        failure.completed_stages().last(),
        Some(&PreparationStage::ClusterWritesAndLifecycle)
    );
    assert_eq!(provider.calls.get(), 0);
    drop(io);
    assert_eq!(
        local
            .inspect_run(&intent_id)
            .expect("TEST_CODE inspect stopped result recovery")
            .head_version(),
        resumed_head
    );
    drop(local);
    assert_eq!(board_durable_facts(fixture.connection()), failed_facts);
    let final_observation = server.finish().await;
    assert_eq!(final_observation.requests, failed_observation.requests);
    assert_eq!(final_observation.non_board_requests, 0);
}
