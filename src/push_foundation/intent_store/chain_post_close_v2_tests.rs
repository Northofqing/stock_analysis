use std::cell::{Cell, RefCell};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::task::{Context, Poll};

use chrono::{DateTime, NaiveDate};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use tempfile::TempDir;

use super::{
    ChainPostClose, ChainPostCloseError, ConceptProviderAdmission, ConceptProviderRawResult,
    ConceptProviderRequest, FixedChainPreparationInput, RunLeaseRequest,
};
use crate::data_gateway::BatchEvidence;
use crate::market_data::TopStock;
use crate::market_domain::ProviderId;
use crate::monitor::push_job::{
    build_single_user_local_chain_post_close_context, raw_digest, BusinessDate, CalendarDate,
    GitSha40, IntentId, LocalChainPostCloseConfig, LocalChainPostCloseRunInput, MachineCatalog,
    Namespace, PhaseEpic, RunId, Sha256Digest, UtcMicros,
};
use crate::pipeline::chain_analysis::preparation::{
    prepare_chain_analysis_with_io, ConceptEffectClock, ConceptProviderRawIo, PreparationFailure,
    PreparationStop, SourceObservation, SourceStatus, UnmigratedStage,
};
use crate::push_foundation::{BusinessIntentStore, FoundationSchemaMigration, LeaseOwnerId};

#[path = "chain_post_close_board_tests.rs"]
mod board_tests;
#[path = "chain_post_close_cluster_tests.rs"]
mod cluster_tests;
#[path = "chain_post_close_concept_batch_tests.rs"]
mod concept_batch_tests;
#[path = "chain_post_close_positions_tests.rs"]
mod positions_tests;
#[path = "chain_post_close_v3_migration_tests.rs"]
mod v3_migration_tests;

const BUSINESS_DATE: &str = "2026-07-21";
const CAPTURED_AT: &str = "2026-07-21T15:31:00+08:00";
const CACHE_CUTOFF: &str = "2026-07-14 15:31:00";
const BUILD_A: &str = "0123456789abcdef0123456789abcdef01234567";
const BUILD_B: &str = "89abcdef0123456789abcdef0123456789abcdef";
const VALID_RAW: &str = "{\"all_boards\":[\"TEST_CODE_CONCEPT\"]}";

struct V2BusinessFixture {
    store: Option<BusinessIntentStore>,
    directory: TempDir,
}

impl V2BusinessFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let database = directory
            .path()
            .canonicalize()
            .unwrap()
            .join("business.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA application_id=1413829460; \
                 PRAGMA user_version=73; \
                 CREATE TABLE stock_concepts ( \
                     code TEXT PRIMARY KEY, \
                     concepts TEXT NOT NULL, \
                     updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP \
                 );",
            )
            .unwrap();
        let foundation = FoundationSchemaMigration::bundled().unwrap();
        assert_eq!(
            foundation.ddl_sha256().as_str(),
            "4bac8e58caa2f5d2362137b5e96dd087649044f45484a1284dbd7e1fd7baa953"
        );
        let sql = foundation.ddl_bytes().strip_prefix(b".bail on\n").unwrap();
        connection
            .execute_batch(std::str::from_utf8(sql).unwrap())
            .unwrap();
        connection.close().unwrap();
        let store = BusinessIntentStore::open(&database).unwrap();
        Self {
            store: Some(store),
            directory,
        }
    }

    fn database(&self) -> PathBuf {
        self.directory
            .path()
            .canonicalize()
            .unwrap()
            .join("business.sqlite")
    }

    fn chain_post_close(&mut self) -> ChainPostClose<'_> {
        ChainPostClose {
            store: self.store.as_mut().unwrap(),
        }
    }

    fn install_v2(&mut self) {
        let v1 = self.chain_post_close().install_schema().unwrap();
        assert_eq!(v1.schema_version(), 1);
        let v2 = self.chain_post_close().migrate_schema_v1_to_v2().unwrap();
        assert_eq!(v2.schema_version(), 2);
        assert_eq!(v2.input_codec_version(), 1);
        assert_eq!(v2.stage_codec_version(), 1);
    }

    fn reopen(&mut self) {
        let store = self.store.take().unwrap();
        store.connection.close().unwrap();
        self.store = Some(BusinessIntentStore::open(&self.database()).unwrap());
    }

    fn connection(&self) -> &Connection {
        &self.store.as_ref().unwrap().connection
    }

    fn execute(&self, sql: &str) {
        self.connection().execute_batch(sql).unwrap();
    }

    fn count(&self, table: &str) -> i64 {
        self.connection()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn v1_metadata(
        &self,
    ) -> (
        Vec<(i64, i64, String, String)>,
        Vec<(String, String, Vec<u8>)>,
    ) {
        let headers = self
            .connection()
            .prepare(
                "SELECT schema_version,artifact_codec_version,description,bundle_sha256 \
                 FROM chain_post_close_schema ORDER BY schema_version",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let objects = self
            .connection()
            .prepare(
                "SELECT name,object_type,CAST(definition AS BLOB) \
                 FROM chain_post_close_objects ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        (headers, objects)
    }

    fn foundation_catalog(&self) -> Vec<(String, String, Vec<u8>, Vec<u8>)> {
        self.connection()
            .prepare(
                "SELECT r.name,r.object_type,CAST(r.definition AS BLOB),CAST(s.sql AS BLOB) \
                 FROM push_foundation_objects r JOIN sqlite_schema s \
                 ON s.name=r.name AND s.type=r.object_type ORDER BY r.name",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    fn stored_context(&self, intent_id: &IntentId) -> (Vec<u8>, i64, String) {
        self.connection()
            .query_row(
                "SELECT CAST(context_bytes AS BLOB),context_length,run_context_sha256 \
                 FROM chain_post_close_runs WHERE intent_id=?1",
                [intent_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
    }

    fn stored_input(&self, intent_id: &IntentId) -> (Vec<u8>, i64, String) {
        self.connection()
            .query_row(
                "SELECT CAST(input_bytes AS BLOB),input_length,input_sha256 \
                 FROM chain_post_close_runs WHERE intent_id=?1",
                [intent_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
    }

    fn stored_result(&self, intent_id: &IntentId) -> Option<(String, Vec<u8>)> {
        self.connection()
            .query_row(
                "SELECT outcome,CAST(result_bytes AS BLOB) \
                 FROM chain_post_close_stage_results \
                 WHERE intent_id=?1 AND effect_kind='ConceptProvider' AND effect_ordinal=0",
                [intent_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .unwrap()
    }
}

fn local_config(build: &str) -> LocalChainPostCloseConfig {
    LocalChainPostCloseConfig::try_new(
        MachineCatalog::bundled().unwrap().catalog_sha256().clone(),
        GitSha40::parse(build).unwrap(),
        7,
    )
    .unwrap()
}

fn captured_at() -> UtcMicros {
    UtcMicros::try_new(
        DateTime::parse_from_rfc3339(CAPTURED_AT)
            .unwrap()
            .timestamp_micros(),
    )
    .unwrap()
}

fn at(offset: i64) -> i64 {
    captured_at().get() + offset
}

fn run_input(run_id: &str) -> LocalChainPostCloseRunInput {
    LocalChainPostCloseRunInput::try_new(
        RunId::try_new(run_id.to_owned()).unwrap(),
        CalendarDate::parse(BUSINESS_DATE).unwrap(),
        BusinessDate::parse(BUSINESS_DATE).unwrap(),
        captured_at(),
    )
    .unwrap()
}

fn source(status: SourceStatus, batch: &str) -> SourceObservation {
    source_named(status, batch, "TEST_CODE_LOCAL_SOURCE")
}

fn source_named(status: SourceStatus, batch: &str, source_name: &str) -> SourceObservation {
    SourceObservation::from_batch_for_request(
        status,
        BatchEvidence {
            provider: ProviderId::Custom,
            source: source_name.to_owned(),
            source_at: Some("2026-07-21T15:30:00+08:00".to_owned()),
            observed_at: CAPTURED_AT.to_owned(),
            batch_id: batch.to_owned(),
        },
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        CAPTURED_AT.to_owned(),
    )
    .unwrap()
}

fn stocks() -> Vec<TopStock> {
    [
        "TEST_CODE_HIT",
        "TEST_CODE_MISSING_1",
        "TEST_CODE_MISSING_2",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, code)| TopStock {
        code: code.to_owned(),
        name: format!("TEST_CODE_NAME_{index}"),
        change_pct: 10.0 - index as f64 / 10.0,
        price: 10.0 + index as f64,
        ..TopStock::default()
    })
    .collect()
}

fn fixed_input(stocks: Vec<TopStock>) -> FixedChainPreparationInput {
    FixedChainPreparationInput::try_new(
        BusinessDate::parse(BUSINESS_DATE).unwrap(),
        stocks,
        None,
        source(SourceStatus::Available, "TEST_CODE_LIMIT_UP_BATCH"),
        source(SourceStatus::VerifiedEmpty, "TEST_CODE_MACRO_EMPTY_BATCH"),
    )
    .unwrap()
}

fn lease_request(
    owner: &str,
    now: i64,
    until: i64,
    expected_head_version: Option<u64>,
) -> RunLeaseRequest {
    RunLeaseRequest::try_new(
        LeaseOwnerId::try_new(owner.to_owned()).unwrap(),
        UtcMicros::try_new(at(now)).unwrap(),
        UtcMicros::try_new(at(until)).unwrap(),
        expected_head_version,
    )
    .unwrap()
}

fn seed_cache(fixture: &V2BusinessFixture) {
    fixture.execute(
        "INSERT INTO stock_concepts(code,concepts,updated_at) VALUES \
         ('TEST_CODE_HIT','[\"TEST_CODE_CACHED\"]','2026-07-21 14:00:00'), \
         ('TEST_CODE_EXPIRED','[\"TEST_CODE_OLD\"]','2026-07-01 14:00:00');",
    );
}

#[test]
fn single_user_local_v2_preserves_v1_and_recovers_original_context_and_input() {
    let mut fixture = V2BusinessFixture::new();
    fixture.chain_post_close().install_schema().unwrap();
    let v1_metadata = fixture.v1_metadata();
    let foundation = fixture.foundation_catalog();
    let migrated = fixture
        .chain_post_close()
        .migrate_schema_v1_to_v2()
        .unwrap();
    assert_eq!(migrated.schema_version(), 2);
    assert_eq!(fixture.count("chain_post_close_layout_objects"), 27);
    assert_eq!(fixture.v1_metadata(), v1_metadata);
    assert_eq!(fixture.foundation_catalog(), foundation);
    let verified = fixture.chain_post_close().verify_schema().unwrap();
    assert_eq!(verified.schema_version(), 2);
    assert_eq!(verified.ddl_sha256(), migrated.ddl_sha256());
    assert_eq!(
        fixture.chain_post_close().verify_schema_v1_reader(),
        Err(ChainPostCloseError::SchemaRejected)
    );
    fixture.reopen();
    let reopened = fixture.chain_post_close().verify_schema().unwrap();
    assert_eq!(reopened.schema_version(), 2);
    assert_eq!(reopened.ddl_sha256(), migrated.ddl_sha256());
    seed_cache(&fixture);

    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_CONTEXT_A"),
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
            fixed_input(stocks()),
            lease_request("TEST_CODE_OWNER_A", 1_000, 2_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    drop(local);

    let context_row = fixture.stored_context(&intent_id);
    let input_row = fixture.stored_input(&intent_id);
    assert_eq!(context_row.1, i64::try_from(context_row.0.len()).unwrap());
    assert_eq!(input_row.1, i64::try_from(input_row.0.len()).unwrap());
    assert_eq!(raw_digest(&context_row.0).as_str(), context_row.2);
    assert_eq!(raw_digest(&input_row.0).as_str(), input_row.2);
    assert!(context_row
        .0
        .windows(BUILD_A.len())
        .any(|bytes| bytes == BUILD_A.as_bytes()));
    assert!(input_row
        .0
        .windows(CACHE_CUTOFF.len())
        .any(|bytes| bytes == CACHE_CUTOFF.as_bytes()));

    fixture.reopen();
    let changed_current_config = local_config(BUILD_B);
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&changed_current_config)
        .unwrap();
    let recovered = local.inspect_run(&intent_id).unwrap();
    assert_eq!(
        recovered.context().run_id().as_str(),
        "TEST_CODE_RUN_CONTEXT_A"
    );
    assert_eq!(recovered.context().namespace(), &Namespace::Production);
    assert_eq!(
        recovered.context().unit_id().as_str(),
        "MU-chain-post-close"
    );
    assert_eq!(recovered.context().phase(), PhaseEpic::Postclose);
    assert_eq!(recovered.context().business_date().as_str(), BUSINESS_DATE);
    assert_eq!(recovered.context().calendar_date().as_str(), BUSINESS_DATE);
    assert_eq!(recovered.context().build_commit().as_str(), BUILD_A);
    assert_eq!(recovered.context().activation_generation(), 7);
    assert_eq!(recovered.context().captured_business_time(), captured_at());
    assert_eq!(
        recovered.context().catalog_sha256(),
        MachineCatalog::bundled().unwrap().catalog_sha256()
    );
    assert_eq!(
        recovered.context().canonical_sha256().as_str(),
        context_row.2
    );
    assert_eq!(recovered.fixed_input().cache_cutoff(), CACHE_CUTOFF);
    assert_eq!(recovered.fixed_input().cache_rows().len(), 1);
    assert_eq!(
        recovered.fixed_input().cache_rows()[0].code(),
        "TEST_CODE_HIT"
    );
    drop(local);
    assert_eq!(fixture.stored_context(&intent_id), context_row);
    assert_eq!(fixture.stored_input(&intent_id), input_row);
    assert_eq!(fixture.v1_metadata(), v1_metadata);
    assert_eq!(fixture.foundation_catalog(), foundation);
    assert_eq!(
        fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        2
    );

    fixture.execute(
        "CREATE INDEX TEST_CODE_FOREIGN_RUN_INDEX \
         ON chain_post_close_runs(run_id);",
    );
    assert_eq!(
        fixture.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::SchemaRejected)
    );
}

#[test]
fn single_user_local_rejects_bad_config_source_cache_and_occurrence_conflicts() {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    seed_cache(&fixture);
    let bad_config = LocalChainPostCloseConfig::try_new(
        Sha256Digest::parse("catalog_sha256", &"f".repeat(64)).unwrap(),
        GitSha40::parse(BUILD_A).unwrap(),
        7,
    )
    .unwrap();
    assert!(matches!(
        fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&bad_config),
        Err(ChainPostCloseError::InvalidConfiguration { .. })
    ));
    assert_eq!(fixture.count("chain_post_close_runs"), 0);

    assert!(FixedChainPreparationInput::try_new(
        BusinessDate::parse(BUSINESS_DATE).unwrap(),
        stocks(),
        None,
        SourceObservation::unknown(),
        source(SourceStatus::VerifiedEmpty, "TEST_CODE_MACRO_EMPTY_BATCH"),
    )
    .is_err());
    assert_eq!(fixture.count("chain_post_close_runs"), 0);

    fixture.execute(
        "INSERT INTO stock_concepts(code,concepts,updated_at) VALUES \
         ('TEST_CODE_BAD_CACHE','not-json','2026-07-21 14:00:00');",
    );
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_CONFLICT_A"),
    )
    .unwrap();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    assert!(matches!(
        local.acquire_run(
            context,
            fixed_input(stocks()),
            lease_request("TEST_CODE_OWNER_A", 1_000, 2_000, None),
        ),
        Err(ChainPostCloseError::InvalidInput { .. })
    ));
    drop(local);
    assert_eq!(fixture.count("chain_post_close_runs"), 0);
    fixture.execute("DELETE FROM stock_concepts WHERE code='TEST_CODE_BAD_CACHE';");

    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_CONFLICT_A"),
    )
    .unwrap();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    local
        .acquire_run(
            context,
            fixed_input(stocks()),
            lease_request("TEST_CODE_OWNER_A", 1_000, 2_000, None),
        )
        .unwrap();
    let different_run = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_CONFLICT_B"),
    )
    .unwrap();
    assert!(matches!(
        local.acquire_run(
            different_run,
            fixed_input(stocks()),
            lease_request("TEST_CODE_OWNER_B", 1_100, 2_100, None),
        ),
        Err(ChainPostCloseError::RunConflict { .. })
    ));
    let mut changed_stocks = stocks();
    changed_stocks[0].name = "TEST_CODE_CHANGED_INPUT".to_owned();
    let same_run = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_CONFLICT_A"),
    )
    .unwrap();
    assert!(matches!(
        local.acquire_run(
            same_run,
            fixed_input(changed_stocks),
            lease_request("TEST_CODE_OWNER_B", 1_100, 2_100, None),
        ),
        Err(ChainPostCloseError::RunConflict { .. })
    ));
    drop(local);
    assert_eq!(fixture.count("chain_post_close_runs"), 1);
    assert_eq!(fixture.count("chain_post_close_stage_begins"), 0);
}

#[test]
fn single_user_local_lease_cas_rejects_duplicate_and_stale_capabilities() {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    seed_cache(&fixture);
    let config = local_config(BUILD_A);
    let context =
        build_single_user_local_chain_post_close_context(&config, run_input("TEST_CODE_RUN_LEASE"))
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
            fixed_input(stocks()),
            lease_request("TEST_CODE_OWNER_A", 1_000, 2_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    assert!(matches!(
        local.resume_run(
            &intent_id,
            lease_request("TEST_CODE_OWNER_B", 1_500, 2_500, Some(0)),
        ),
        Err(ChainPostCloseError::LeaseHeld { .. })
    ));
    let (lease, admission) = local
        .begin_concept_provider(
            lease,
            ConceptProviderRequest::try_new(0, "TEST_CODE_MISSING_1".to_owned()).unwrap(),
            UtcMicros::try_new(at(1_600)).unwrap(),
        )
        .unwrap();
    let call = match admission {
        ConceptProviderAdmission::Call(call) => call,
        ConceptProviderAdmission::Replay(_) => panic!("first begin cannot replay"),
    };
    assert_eq!(lease.head_version(), 1);

    let generation_two = local
        .resume_run(
            &intent_id,
            lease_request("TEST_CODE_OWNER_B", 2_001, 3_000, Some(1)),
        )
        .unwrap();
    assert_eq!(generation_two.generation(), 2);
    assert_eq!(generation_two.head_version(), 2);
    assert!(matches!(
        local.resume_run(
            &intent_id,
            lease_request("TEST_CODE_OWNER_C", 2_002, 3_001, Some(1)),
        ),
        Err(ChainPostCloseError::StaleLease { .. })
    ));
    let begins_before = local.inspect_run(&intent_id).unwrap().begins().len();
    assert!(matches!(
        local.record_concept_provider_result(
            lease,
            call,
            ConceptProviderRawResult::returned(VALID_RAW.to_owned()),
            UtcMicros::try_new(at(2_100)).unwrap(),
        ),
        Err(ChainPostCloseError::StaleLease { .. })
    ));
    let recovered = local.inspect_run(&intent_id).unwrap();
    assert_eq!(recovered.begins().len(), begins_before);
    assert!(recovered.results().is_empty());
    assert_eq!(recovered.lease_generation(), 2);
    assert_eq!(recovered.head_version(), 2);
    drop(local);

    let mut expired_begin = V2BusinessFixture::new();
    expired_begin.install_v2();
    seed_cache(&expired_begin);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_EXPIRED_BEGIN"),
    )
    .unwrap();
    let mut local = expired_begin
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            context,
            fixed_input(stocks()),
            lease_request("TEST_CODE_OWNER_EXPIRED_BEGIN", 10, 100, None),
        )
        .unwrap();
    assert!(matches!(
        local.begin_concept_provider(
            lease,
            ConceptProviderRequest::try_new(0, "TEST_CODE_MISSING_1".to_owned()).unwrap(),
            UtcMicros::try_new(at(101)).unwrap(),
        ),
        Err(ChainPostCloseError::LeaseExpired { .. })
    ));
    drop(local);
    assert_eq!(expired_begin.count("chain_post_close_stage_begins"), 0);

    let mut expired_result = V2BusinessFixture::new();
    expired_result.install_v2();
    seed_cache(&expired_result);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_EXPIRED_RESULT"),
    )
    .unwrap();
    let mut local = expired_result
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            context,
            fixed_input(stocks()),
            lease_request("TEST_CODE_OWNER_EXPIRED_RESULT", 10, 100, None),
        )
        .unwrap();
    let (lease, admission) = local
        .begin_concept_provider(
            lease,
            ConceptProviderRequest::try_new(0, "TEST_CODE_MISSING_1".to_owned()).unwrap(),
            UtcMicros::try_new(at(50)).unwrap(),
        )
        .unwrap();
    let call = match admission {
        ConceptProviderAdmission::Call(call) => call,
        ConceptProviderAdmission::Replay(_) => panic!("first begin cannot replay"),
    };
    assert!(matches!(
        local.record_concept_provider_result(
            lease,
            call,
            ConceptProviderRawResult::returned(VALID_RAW.to_owned()),
            UtcMicros::try_new(at(101)).unwrap(),
        ),
        Err(ChainPostCloseError::LeaseExpired { .. })
    ));
    drop(local);
    assert_eq!(expired_result.count("chain_post_close_stage_begins"), 1);
    assert_eq!(expired_result.count("chain_post_close_stage_results"), 0);
}

enum ProviderReply {
    Returned(String),
    BusinessError(String),
}

enum ProviderObservation {
    ConfirmBeginThenRelease,
    HoldReadLockForResultCommit,
    PanicIfCalled,
}

struct RecordingRawProvider {
    database: PathBuf,
    reply: ProviderReply,
    observation: ProviderObservation,
    calls: RefCell<Vec<String>>,
    held_reader: RefCell<Option<Connection>>,
    advance_clock: Option<(Rc<Cell<i64>>, i64)>,
}

impl RecordingRawProvider {
    fn new(database: &Path, reply: ProviderReply, observation: ProviderObservation) -> Self {
        Self {
            database: database.to_path_buf(),
            reply,
            observation,
            calls: RefCell::new(Vec::new()),
            held_reader: RefCell::new(None),
            advance_clock: None,
        }
    }

    fn advancing_clock(mut self, clock: &ControlledClock, next: i64) -> Self {
        self.advance_clock = Some((Rc::clone(&clock.now), next));
        self
    }

    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }

    fn release_read_lock(&self) {
        if let Some(connection) = self.held_reader.borrow_mut().take() {
            connection.execute_batch("ROLLBACK;").unwrap();
            connection.close().unwrap();
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ConceptProviderRawIo for RecordingRawProvider {
    async fn call_raw(&self, code: &str) -> std::result::Result<String, String> {
        if matches!(self.observation, ProviderObservation::PanicIfCalled) {
            panic!("recovery must not call the concept provider")
        }
        self.calls.borrow_mut().push(code.to_owned());
        let connection = Connection::open_with_flags(
            &self.database,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        connection.execute_batch("BEGIN DEFERRED;").unwrap();
        let begin_rows: i64 = connection
            .query_row(
                "SELECT count(*) FROM chain_post_close_stage_begins \
                 WHERE effect_kind='ConceptProvider' AND effect_ordinal=0 AND effect_key=?1",
                [code],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(begin_rows, 1, "provider may run only after durable begin");
        match self.observation {
            ProviderObservation::ConfirmBeginThenRelease => {
                connection.execute_batch("ROLLBACK;").unwrap();
                connection.close().unwrap();
            }
            ProviderObservation::HoldReadLockForResultCommit => {
                *self.held_reader.borrow_mut() = Some(connection);
            }
            ProviderObservation::PanicIfCalled => unreachable!(),
        }
        if let Some((clock, next)) = &self.advance_clock {
            clock.set(*next);
        }
        match &self.reply {
            ProviderReply::Returned(raw) => Ok(raw.clone()),
            ProviderReply::BusinessError(error) => Err(error.clone()),
        }
    }
}

#[derive(Clone)]
struct ControlledClock {
    now: Rc<Cell<i64>>,
}

impl ControlledClock {
    fn new(now: i64) -> Self {
        Self {
            now: Rc::new(Cell::new(now)),
        }
    }
}

impl ConceptEffectClock for ControlledClock {
    fn now(&self) -> UtcMicros {
        UtcMicros::try_new(self.now.get()).unwrap()
    }
}

async fn run_public_prepare(
    local: &mut super::LocalChainPostClose<'_>,
    lease: super::RunLease,
    provider: &mut RecordingRawProvider,
    clock: &ControlledClock,
) -> anyhow::Error {
    let mut io = local
        .first_concept_preparation_io(lease, provider, clock)
        .unwrap();
    prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        stocks(),
        None,
        &mut io,
    )
    .await
    .expect_err("the first slice must stop before an unmigrated effect")
}

#[tokio::test]
async fn single_user_local_first_concept_result_replays_without_provider_or_cache_reread() {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    seed_cache(&fixture);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_REPLAY"),
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
            lease_request("TEST_CODE_OWNER_A", 1_000, 2_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let mut provider = RecordingRawProvider::new(
        &database,
        ProviderReply::Returned(VALID_RAW.to_owned()),
        ProviderObservation::ConfirmBeginThenRelease,
    );
    let clock = ControlledClock::new(at(1_100));
    let error = run_public_prepare(&mut local, lease, &mut provider, &clock).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::ConceptCacheWrite,
            ..
        })
    ));
    assert_eq!(provider.calls(), vec!["TEST_CODE_MISSING_1"]);
    drop(local);
    assert_eq!(
        fixture.stored_result(&intent_id),
        Some(("Returned".to_owned(), VALID_RAW.as_bytes().to_vec()))
    );

    fixture.execute(
        "INSERT INTO stock_concepts(code,concepts,updated_at) VALUES \
         ('TEST_CODE_MISSING_1','[\"TEST_CODE_NEW_CACHE\"]','2026-07-21 15:40:00');",
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
            lease_request("TEST_CODE_OWNER_B", 2_001, 3_000, Some(head)),
        )
        .unwrap();
    let mut replay_provider = RecordingRawProvider::new(
        &database,
        ProviderReply::Returned("TEST_CODE_MUST_NOT_BE_USED".to_owned()),
        ProviderObservation::PanicIfCalled,
    );
    let clock = ControlledClock::new(at(2_100));
    let error = run_public_prepare(&mut local, lease, &mut replay_provider, &clock).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::ConceptCacheWrite,
            ..
        })
    ));
    assert!(replay_provider.calls().is_empty());
    drop(local);
    assert_eq!(
        fixture.stored_result(&intent_id),
        Some(("Returned".to_owned(), VALID_RAW.as_bytes().to_vec()))
    );

    for (run, reply, expected_outcome) in [
        (
            "TEST_CODE_RUN_EMPTY_RETURN",
            ProviderReply::Returned(String::new()),
            "Returned",
        ),
        (
            "TEST_CODE_RUN_EMPTY_ERROR",
            ProviderReply::BusinessError(String::new()),
            "BusinessError",
        ),
    ] {
        let mut fixture = V2BusinessFixture::new();
        fixture.install_v2();
        seed_cache(&fixture);
        let context =
            build_single_user_local_chain_post_close_context(&config, run_input(run)).unwrap();
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
                lease_request("TEST_CODE_OWNER_EMPTY", 1_000, 2_000, None),
            )
            .unwrap();
        let intent_id = lease.intent_id().clone();
        let mut provider = RecordingRawProvider::new(
            &database,
            reply,
            ProviderObservation::ConfirmBeginThenRelease,
        );
        let clock = ControlledClock::new(at(1_100));
        let error = run_public_prepare(&mut local, lease, &mut provider, &clock).await;
        assert!(error.downcast_ref::<PreparationFailure>().is_some());
        assert!(error.downcast_ref::<PreparationStop>().is_none());
        assert_eq!(provider.calls(), vec!["TEST_CODE_MISSING_1"]);
        drop(local);
        assert_eq!(
            fixture.stored_result(&intent_id),
            Some((expected_outcome.to_owned(), Vec::new()))
        );
        assert_eq!(fixture.count("chain_post_close_stage_begins"), 1);
        assert_eq!(fixture.count("chain_post_close_stage_results"), 1);

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
                lease_request("TEST_CODE_OWNER_EMPTY_REPLAY", 2_001, 3_000, Some(head)),
            )
            .unwrap();
        let mut replay_provider = RecordingRawProvider::new(
            &database,
            ProviderReply::Returned("TEST_CODE_MUST_NOT_BE_USED".to_owned()),
            ProviderObservation::PanicIfCalled,
        );
        let clock = ControlledClock::new(at(2_100));
        let error = run_public_prepare(&mut local, lease, &mut replay_provider, &clock).await;
        assert!(error.downcast_ref::<PreparationFailure>().is_some());
        assert!(replay_provider.calls().is_empty());
        drop(local);
        assert_eq!(
            fixture.stored_result(&intent_id),
            Some((expected_outcome.to_owned(), Vec::new()))
        );
    }
}

#[tokio::test]
async fn single_user_local_unconfirmed_concept_result_stops_and_never_replays() {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    seed_cache(&fixture);
    let journal_mode: String = fixture
        .connection()
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode, "delete");
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_UNCONFIRMED"),
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
            lease_request("TEST_CODE_OWNER_A", 1_000, 2_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let mut provider = RecordingRawProvider::new(
        &database,
        ProviderReply::Returned(VALID_RAW.to_owned()),
        ProviderObservation::HoldReadLockForResultCommit,
    );
    let clock = ControlledClock::new(at(1_100));
    let error = run_public_prepare(&mut local, lease, &mut provider, &clock).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::ResultUnconfirmed { .. })
    ));
    assert_eq!(provider.calls(), vec!["TEST_CODE_MISSING_1"]);
    drop(local);
    provider.release_read_lock();
    assert_eq!(fixture.count("chain_post_close_stage_begins"), 1);
    assert_eq!(fixture.count("chain_post_close_stage_results"), 0);

    fixture.reopen();
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let recovery = local.inspect_run(&intent_id).unwrap();
    assert_eq!(recovery.begins().len(), 1);
    assert!(recovery.results().is_empty());
    let lease = local
        .resume_run(
            &intent_id,
            lease_request(
                "TEST_CODE_OWNER_B",
                2_001,
                3_000,
                Some(recovery.head_version()),
            ),
        )
        .unwrap();
    let mut replay_provider = RecordingRawProvider::new(
        &database,
        ProviderReply::Returned("TEST_CODE_MUST_NOT_BE_USED".to_owned()),
        ProviderObservation::PanicIfCalled,
    );
    let clock = ControlledClock::new(at(2_100));
    let error = run_public_prepare(&mut local, lease, &mut replay_provider, &clock).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::IncompleteOnReopen { .. })
    ));
    assert!(replay_provider.calls().is_empty());
    drop(local);

    let mut expired = V2BusinessFixture::new();
    expired.install_v2();
    seed_cache(&expired);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_PROVIDER_CROSSES_LEASE"),
    )
    .unwrap();
    let database = expired.database();
    let mut local = expired
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            context,
            fixed_input(stocks()),
            lease_request("TEST_CODE_OWNER_EXPIRING", 10, 100, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let clock = ControlledClock::new(at(50));
    let mut provider = RecordingRawProvider::new(
        &database,
        ProviderReply::Returned(VALID_RAW.to_owned()),
        ProviderObservation::ConfirmBeginThenRelease,
    )
    .advancing_clock(&clock, at(101));
    let error = run_public_prepare(&mut local, lease, &mut provider, &clock).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::AuthorityRejected { .. })
    ));
    assert_eq!(provider.calls(), vec!["TEST_CODE_MISSING_1"]);
    drop(local);
    assert_eq!(expired.count("chain_post_close_stage_begins"), 1);
    assert_eq!(expired.count("chain_post_close_stage_results"), 0);
    assert_eq!(expired.stored_result(&intent_id), None);
}

#[test]
fn single_user_local_codec_accepts_normal_sources_and_compares_source_identity() {
    let long_macro = "TEST_CODE_MACRO_NEWS\n".repeat(80);
    assert!(FixedChainPreparationInput::try_new(
        BusinessDate::parse(BUSINESS_DATE).unwrap(),
        stocks(),
        Some(long_macro),
        source(SourceStatus::Available, "TEST_CODE_LIMIT_UP_BATCH"),
        source(SourceStatus::Available, "TEST_CODE_MACRO_BATCH"),
    )
    .is_ok());
    assert!(FixedChainPreparationInput::try_new(
        BusinessDate::parse(BUSINESS_DATE).unwrap(),
        Vec::new(),
        None,
        source(
            SourceStatus::VerifiedEmpty,
            "TEST_CODE_LIMIT_UP_EMPTY_BATCH",
        ),
        SourceObservation::unavailable("TEST_CODE_MACRO_UNAVAILABLE".to_owned()),
    )
    .is_ok());
    assert!(FixedChainPreparationInput::try_new(
        BusinessDate::parse(BUSINESS_DATE).unwrap(),
        Vec::new(),
        None,
        source(SourceStatus::Available, "TEST_CODE_LIMIT_UP_BATCH"),
        source(SourceStatus::VerifiedEmpty, "TEST_CODE_MACRO_EMPTY_BATCH"),
    )
    .is_err());

    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    seed_cache(&fixture);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_SOURCE_CONFLICT"),
    )
    .unwrap();
    let first = FixedChainPreparationInput::try_new(
        BusinessDate::parse(BUSINESS_DATE).unwrap(),
        stocks(),
        None,
        source_named(
            SourceStatus::Available,
            "TEST_CODE_LIMIT_UP_BATCH",
            "TEST_CODE_SOURCE_A",
        ),
        source(SourceStatus::VerifiedEmpty, "TEST_CODE_MACRO_EMPTY_BATCH"),
    )
    .unwrap();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    local
        .acquire_run(
            context,
            first,
            lease_request("TEST_CODE_OWNER_SOURCE_A", 1_000, 2_000, None),
        )
        .unwrap();
    let same_context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_SOURCE_CONFLICT"),
    )
    .unwrap();
    let changed_source = FixedChainPreparationInput::try_new(
        BusinessDate::parse(BUSINESS_DATE).unwrap(),
        stocks(),
        None,
        source_named(
            SourceStatus::Available,
            "TEST_CODE_LIMIT_UP_BATCH",
            "TEST_CODE_SOURCE_B",
        ),
        source(SourceStatus::VerifiedEmpty, "TEST_CODE_MACRO_EMPTY_BATCH"),
    )
    .unwrap();
    assert!(matches!(
        local.acquire_run(
            same_context,
            changed_source,
            lease_request("TEST_CODE_OWNER_SOURCE_B", 1_100, 2_100, None),
        ),
        Err(ChainPostCloseError::RunConflict { .. })
    ));
}

enum PublicInputMutation {
    Stock,
    Date,
    Macro,
}

async fn assert_public_input_rejected_before_begin(mutation: PublicInputMutation, run_id: &str) {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
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
            lease_request("TEST_CODE_OWNER_INPUT", 1_000, 2_000, None),
        )
        .unwrap();
    let mut provider = RecordingRawProvider::new(
        &database,
        ProviderReply::Returned(VALID_RAW.to_owned()),
        ProviderObservation::PanicIfCalled,
    );
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .first_concept_preparation_io(lease, &mut provider, &clock)
        .unwrap();
    let mut actual_stocks = stocks();
    let mut actual_date = NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
    let mut actual_macro = None;
    match mutation {
        PublicInputMutation::Stock => {
            actual_stocks[0].name = "TEST_CODE_CHANGED_NAME".to_owned();
            actual_stocks[0].price += 1.0;
        }
        PublicInputMutation::Date => {
            actual_date = NaiveDate::from_ymd_opt(2026, 7, 22).unwrap();
        }
        PublicInputMutation::Macro => {
            actual_macro = Some("TEST_CODE_CHANGED_MACRO".to_owned());
        }
    }
    assert!(
        prepare_chain_analysis_with_io(actual_date, actual_stocks, actual_macro, &mut io)
            .await
            .is_err()
    );
    drop(io);
    assert!(provider.calls().is_empty());
    drop(local);
    assert_eq!(fixture.count("chain_post_close_stage_begins"), 0);
}

#[tokio::test]
async fn single_user_local_public_prepare_rejects_changed_complete_input_before_effect() {
    assert_public_input_rejected_before_begin(
        PublicInputMutation::Stock,
        "TEST_CODE_RUN_CHANGED_STOCK",
    )
    .await;
    assert_public_input_rejected_before_begin(
        PublicInputMutation::Date,
        "TEST_CODE_RUN_CHANGED_DATE",
    )
    .await;
    assert_public_input_rejected_before_begin(
        PublicInputMutation::Macro,
        "TEST_CODE_RUN_CHANGED_MACRO",
    )
    .await;
}

#[test]
fn single_user_local_effect_capabilities_cannot_cross_runs_or_requests() {
    let config = local_config(BUILD_A);
    let mut first = V2BusinessFixture::new();
    first.install_v2();
    seed_cache(&first);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_CAPABILITY_A"),
    )
    .unwrap();
    let mut local_a = first
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease_a = local_a
        .acquire_run(
            context,
            fixed_input(stocks()),
            lease_request("TEST_CODE_OWNER_CAPABILITY_A", 1_000, 2_000, None),
        )
        .unwrap();
    let (lease_a, admission) = local_a
        .begin_concept_provider(
            lease_a,
            ConceptProviderRequest::try_new(0, "TEST_CODE_MISSING_1".to_owned()).unwrap(),
            UtcMicros::try_new(at(1_100)).unwrap(),
        )
        .unwrap();
    let call_a = match admission {
        ConceptProviderAdmission::Call(call) => call,
        ConceptProviderAdmission::Replay(_) => panic!("first begin cannot replay"),
    };

    let mut second = V2BusinessFixture::new();
    second.install_v2();
    seed_cache(&second);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_CAPABILITY_B"),
    )
    .unwrap();
    let mut local_b = second
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease_b = local_b
        .acquire_run(
            context,
            fixed_input(stocks()),
            lease_request("TEST_CODE_OWNER_CAPABILITY_B", 1_000, 2_000, None),
        )
        .unwrap();
    let intent_b = lease_b.intent_id().clone();
    assert!(matches!(
        local_b.record_concept_provider_result(
            lease_b,
            call_a,
            ConceptProviderRawResult::returned(VALID_RAW.to_owned()),
            UtcMicros::try_new(at(1_200)).unwrap(),
        ),
        Err(ChainPostCloseError::StaleLease { .. })
    ));
    let recovery_b = local_b.inspect_run(&intent_b).unwrap();
    assert_eq!(recovery_b.head_version(), 0);
    assert!(recovery_b.begins().is_empty());
    assert!(recovery_b.results().is_empty());
    drop(local_b);

    assert!(matches!(
        local_a.begin_concept_provider(
            lease_a,
            ConceptProviderRequest::try_new(1, "TEST_CODE_MISSING_2".to_owned()).unwrap(),
            UtcMicros::try_new(at(1_200)).unwrap(),
        ),
        Err(ChainPostCloseError::InvalidInput { .. })
    ));
    drop(local_a);
    assert_eq!(first.count("chain_post_close_stage_begins"), 1);
    assert_eq!(first.count("chain_post_close_stage_results"), 0);
}

fn mutate_behind_guard(fixture: &V2BusinessFixture, trigger: &str, update: &str) {
    let definition: String = fixture
        .connection()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
            [trigger],
            |row| row.get(0),
        )
        .unwrap();
    fixture.execute(&format!("DROP TRIGGER {trigger};"));
    fixture.execute(update);
    fixture.execute(&definition);
}

fn persisted_fixture(run_id: &str) -> (V2BusinessFixture, LocalChainPostCloseConfig, IntentId) {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    seed_cache(&fixture);
    let config = local_config(BUILD_A);
    let context =
        build_single_user_local_chain_post_close_context(&config, run_input(run_id)).unwrap();
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
            lease_request("TEST_CODE_OWNER_INTEGRITY", 1_000, 2_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    drop(local);
    (fixture, config, intent_id)
}

#[test]
fn single_user_local_rejects_corrupt_context_input_and_result_facts() {
    let (mut context_fixture, config, intent_id) =
        persisted_fixture("TEST_CODE_RUN_CORRUPT_CONTEXT");
    mutate_behind_guard(
        &context_fixture,
        "chain_post_close_runs_update",
        "UPDATE chain_post_close_runs SET \
         context_bytes=CAST(context_bytes||x'00' AS BLOB), \
         context_length=context_length+1;",
    );
    let mut local = context_fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    assert_eq!(
        local.inspect_run(&intent_id).err(),
        Some(ChainPostCloseError::SchemaRejected)
    );

    let (mut input_fixture, config, intent_id) = persisted_fixture("TEST_CODE_RUN_CORRUPT_INPUT");
    mutate_behind_guard(
        &input_fixture,
        "chain_post_close_runs_update",
        &format!(
            "UPDATE chain_post_close_runs SET input_sha256='{}';",
            "f".repeat(64)
        ),
    );
    let mut local = input_fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    assert_eq!(
        local.inspect_run(&intent_id).err(),
        Some(ChainPostCloseError::SchemaRejected)
    );

    let (mut result_fixture, config, intent_id) = persisted_fixture("TEST_CODE_RUN_CORRUPT_RESULT");
    let mut local = result_fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let recovery = local.inspect_run(&intent_id).unwrap();
    let lease = local
        .resume_run(
            &intent_id,
            lease_request(
                "TEST_CODE_OWNER_INTEGRITY_RESULT",
                2_001,
                3_000,
                Some(recovery.head_version()),
            ),
        )
        .unwrap();
    let (lease, admission) = local
        .begin_concept_provider(
            lease,
            ConceptProviderRequest::try_new(0, "TEST_CODE_MISSING_1".to_owned()).unwrap(),
            UtcMicros::try_new(at(2_100)).unwrap(),
        )
        .unwrap();
    let call = match admission {
        ConceptProviderAdmission::Call(call) => call,
        ConceptProviderAdmission::Replay(_) => panic!("first begin cannot replay"),
    };
    local
        .record_concept_provider_result(
            lease,
            call,
            ConceptProviderRawResult::returned(VALID_RAW.to_owned()),
            UtcMicros::try_new(at(2_200)).unwrap(),
        )
        .unwrap();
    drop(local);
    mutate_behind_guard(
        &result_fixture,
        "chain_post_close_stage_results_update",
        &format!(
            "UPDATE chain_post_close_stage_results SET result_sha256='{}';",
            "f".repeat(64)
        ),
    );
    let mut local = result_fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    assert_eq!(
        local.inspect_run(&intent_id).err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
}

#[test]
fn single_user_local_finite_precision_input_is_recoverable_or_never_admitted() {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    seed_cache(&fixture);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_FINITE_PRECISION"),
    )
    .unwrap();
    let mut precise_stocks = stocks();
    precise_stocks[0].change_pct = 1.2345678901234567;
    precise_stocks[0].price = 1.2345678901234567;
    precise_stocks[0].volume_ratio = Some(1.2345678901234567);
    precise_stocks[0].main_net_yi = Some(1.2345678901234567);
    let expected_bits = (
        precise_stocks[0].change_pct.to_bits(),
        precise_stocks[0].price.to_bits(),
        precise_stocks[0].volume_ratio.unwrap().to_bits(),
        precise_stocks[0].main_net_yi.unwrap().to_bits(),
    );
    let input = fixed_input(precise_stocks);
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    match local.acquire_run(
        context,
        input,
        lease_request("TEST_CODE_OWNER_FINITE", 1_000, 2_000, None),
    ) {
        Ok(lease) => {
            let intent_id = lease.intent_id().clone();
            drop(local);
            fixture.reopen();
            let mut local = fixture
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&config)
                .unwrap();
            let recovery = local.inspect_run(&intent_id).unwrap();
            let stock = &recovery.fixed_input().stocks()[0];
            assert_eq!(
                (
                    stock.change_pct.to_bits(),
                    stock.price.to_bits(),
                    stock.volume_ratio.unwrap().to_bits(),
                    stock.main_net_yi.unwrap().to_bits(),
                ),
                expected_bits
            );
        }
        Err(ChainPostCloseError::InvalidInput { .. }) => {
            drop(local);
            assert_eq!(fixture.count("chain_post_close_runs"), 0);
        }
        Err(error) => panic!("unexpected finite input result: {error}"),
    }
}

#[test]
fn single_user_local_call_capability_binds_run_id_with_equal_other_fields() {
    let config = local_config(BUILD_A);
    let mut first = V2BusinessFixture::new();
    first.install_v2();
    seed_cache(&first);
    let mut second = V2BusinessFixture::new();
    second.install_v2();
    seed_cache(&second);

    let context_a = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_BINDING_A"),
    )
    .unwrap();
    let context_b = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_BINDING_B"),
    )
    .unwrap();
    let mut local_a = first
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let mut local_b = second
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease_a = local_a
        .acquire_run(
            context_a,
            fixed_input(stocks()),
            lease_request("TEST_CODE_OWNER_EQUAL", 1_000, 2_000, None),
        )
        .unwrap();
    let lease_b = local_b
        .acquire_run(
            context_b,
            fixed_input(stocks()),
            lease_request("TEST_CODE_OWNER_EQUAL", 1_000, 2_000, None),
        )
        .unwrap();
    assert_eq!(lease_a.intent_id(), lease_b.intent_id());
    let (lease_a, admission_a) = local_a
        .begin_concept_provider(
            lease_a,
            ConceptProviderRequest::try_new(0, "TEST_CODE_MISSING_1".to_owned()).unwrap(),
            UtcMicros::try_new(at(1_100)).unwrap(),
        )
        .unwrap();
    let call_a = match admission_a {
        ConceptProviderAdmission::Call(call) => call,
        ConceptProviderAdmission::Replay(_) => panic!("first begin cannot replay"),
    };
    let (lease_b, admission_b) = local_b
        .begin_concept_provider(
            lease_b,
            ConceptProviderRequest::try_new(0, "TEST_CODE_MISSING_1".to_owned()).unwrap(),
            UtcMicros::try_new(at(1_100)).unwrap(),
        )
        .unwrap();
    assert!(matches!(admission_b, ConceptProviderAdmission::Call(_)));
    assert_eq!(lease_a.head_version(), lease_b.head_version());
    let intent_b = lease_b.intent_id().clone();
    assert!(matches!(
        local_b.record_concept_provider_result(
            lease_b,
            call_a,
            ConceptProviderRawResult::returned(VALID_RAW.to_owned()),
            UtcMicros::try_new(at(1_200)).unwrap(),
        ),
        Err(ChainPostCloseError::StaleLease { .. })
    ));
    let recovered = local_b.inspect_run(&intent_b).unwrap();
    assert_eq!(recovered.head_version(), 1);
    assert_eq!(recovered.begins().len(), 1);
    assert!(recovered.results().is_empty());
    drop(local_a);
    drop(local_b);
    assert_eq!(first.count("chain_post_close_stage_results"), 0);
    assert_eq!(second.count("chain_post_close_stage_results"), 0);
}

struct PendingRawProvider {
    database: PathBuf,
    calls: RefCell<Vec<String>>,
}

#[async_trait::async_trait(?Send)]
impl ConceptProviderRawIo for PendingRawProvider {
    async fn call_raw(&self, code: &str) -> std::result::Result<String, String> {
        self.calls.borrow_mut().push(code.to_owned());
        let connection = Connection::open_with_flags(
            &self.database,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        connection.execute_batch("BEGIN DEFERRED;").unwrap();
        let begins: i64 = connection
            .query_row(
                "SELECT count(*) FROM chain_post_close_stage_begins \
                 WHERE effect_kind='ConceptProvider' AND effect_ordinal=0 AND effect_key=?1",
                [code],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(begins, 1);
        connection.execute_batch("ROLLBACK;").unwrap();
        connection.close().unwrap();
        std::future::pending().await
    }
}

#[tokio::test]
async fn single_user_local_cancelled_pending_provider_never_replays() {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    seed_cache(&fixture);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_CANCELLED_PENDING"),
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
            lease_request("TEST_CODE_OWNER_CANCEL", 1_000, 2_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let mut provider = PendingRawProvider {
        database,
        calls: RefCell::new(Vec::new()),
    };
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .first_concept_preparation_io(lease, &mut provider, &clock)
        .unwrap();
    {
        let mut future = Box::pin(prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks(),
            None,
            &mut io,
        ));
        let waker = futures::task::noop_waker();
        let mut context = Context::from_waker(&waker);
        assert!(matches!(
            Future::poll(future.as_mut(), &mut context),
            Poll::Pending
        ));
    }
    let error = prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        stocks(),
        None,
        &mut io,
    )
    .await
    .expect_err("cancelled adapter must stop");
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::ResultUnconfirmed { .. })
    ));
    drop(io);
    assert_eq!(provider.calls.borrow().as_slice(), ["TEST_CODE_MISSING_1"]);
    drop(local);
    assert_eq!(fixture.count("chain_post_close_stage_begins"), 1);
    assert_eq!(fixture.count("chain_post_close_stage_results"), 0);

    fixture.reopen();
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let recovery = local.inspect_run(&intent_id).unwrap();
    let lease = local
        .resume_run(
            &intent_id,
            lease_request(
                "TEST_CODE_OWNER_CANCEL_REOPEN",
                2_001,
                3_000,
                Some(recovery.head_version()),
            ),
        )
        .unwrap();
    let mut provider = RecordingRawProvider::new(
        &database,
        ProviderReply::Returned("TEST_CODE_MUST_NOT_BE_USED".to_owned()),
        ProviderObservation::PanicIfCalled,
    );
    let clock = ControlledClock::new(at(2_100));
    let error = run_public_prepare(&mut local, lease, &mut provider, &clock).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::IncompleteOnReopen { .. })
    ));
    assert!(provider.calls().is_empty());
}
