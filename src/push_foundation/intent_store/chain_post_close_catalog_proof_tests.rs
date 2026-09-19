use super::*;
use super::super::super::{check_lease, inspect_run_and_macro_with_catalog, schema};
use rusqlite::{params, TransactionBehavior};

const PROOF_OWNER: &str = "TEST_CODE_CATALOG_PROOF_OWNER";
const FUTURE_DIGEST_14: &str =
    "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const FUTURE_DIGEST: &str =
    "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

#[derive(Debug, PartialEq)]
struct CatalogSnapshot {
    schema_cookie: i64,
    layouts: Vec<Vec<Value>>,
    registry: Vec<Vec<Value>>,
    sqlite_catalog: Vec<Vec<Value>>,
}

#[derive(Debug, PartialEq)]
struct RunFactSnapshot {
    run: Vec<Vec<Value>>,
    begins: Vec<Vec<Value>>,
    legacy_qualifications: Vec<Vec<Value>>,
    finals: Vec<Vec<Value>>,
}

fn install_v12(fixture: &mut V2BusinessFixture) {
    fixture.install_v2();
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v2_to_v3()
            .unwrap()
            .schema_version(),
        3
    );
    cluster_tests::install_business_rows(fixture);
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v3_to_v4()
            .unwrap()
            .schema_version(),
        4
    );
    install_br159(fixture);
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v4_to_v5()
            .unwrap()
            .schema_version(),
        5
    );
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v5_to_v6()
            .unwrap()
            .schema_version(),
        6
    );
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v6_to_v7()
            .unwrap()
            .schema_version(),
        7
    );
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v7_to_v8()
            .unwrap()
            .schema_version(),
        8
    );
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v8_to_v9()
            .unwrap()
            .schema_version(),
        9
    );
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v9_to_v10()
            .unwrap()
            .schema_version(),
        10
    );
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v10_to_v11()
            .unwrap()
            .schema_version(),
        11
    );
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v11_to_v12()
            .unwrap()
            .schema_version(),
        12
    );
}

fn catalog_snapshot(connection: &Connection) -> CatalogSnapshot {
    CatalogSnapshot {
        schema_cookie: connection
            .query_row("PRAGMA main.schema_version", [], |row| row.get(0))
            .unwrap(),
        layouts: all_rows(
            connection,
            "SELECT * FROM chain_post_close_layouts ORDER BY layout_version",
        ),
        registry: all_rows(
            connection,
            "SELECT * FROM chain_post_close_layout_objects ORDER BY layout_version,name",
        ),
        sqlite_catalog: all_rows(
            connection,
            "SELECT type,name,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
             WHERE (lower(name) GLOB 'chain_post_close_*' \
                    OR lower(tbl_name) GLOB 'chain_post_close_*') \
               AND NOT(type='index' AND name GLOB 'sqlite_autoindex_*' AND sql IS NULL) \
             ORDER BY type,name",
        ),
    }
}

fn run_fact_snapshot(connection: &Connection, intent: &IntentId) -> RunFactSnapshot {
    let quoted = format!("'{}'", intent.as_str());
    RunFactSnapshot {
        run: all_rows(
            connection,
            &format!("SELECT * FROM chain_post_close_runs WHERE intent_id={quoted}"),
        ),
        begins: all_rows(
            connection,
            &format!("SELECT * FROM chain_post_close_stage_begins WHERE intent_id={quoted}"),
        ),
        legacy_qualifications: all_rows(
            connection,
            &format!(
                "SELECT * FROM chain_post_close_concept_rpc_legacy_outer_qualifications \
                 WHERE intent_id={quoted}"
            ),
        ),
        finals: all_rows(
            connection,
            &format!(
                "SELECT * FROM chain_post_close_concept_rpc_finals WHERE intent_id={quoted}"
            ),
        ),
    }
}

fn schema_cookie(connection: &Connection) -> i64 {
    connection
        .query_row("PRAGMA main.schema_version", [], |row| row.get(0))
        .unwrap()
}

fn run_row(connection: &Connection, intent: &IntentId) -> (String, i64, i64, i64, i64) {
    connection
        .query_row(
            "SELECT lease_owner,lease_generation,head_version,lease_until,updated_at \
             FROM chain_post_close_runs WHERE intent_id=?1",
            [intent.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap()
}

#[test]
fn single_user_local_v12_catalog_proof_binds_transaction_and_rejects_catalog_drift() {
    let mut fixture = V2BusinessFixture::new();
    install_v12(&mut fixture);
    let installed = catalog_snapshot(fixture.connection());
    let database = fixture.database();
    let mut second = BusinessIntentStore::open(&database).unwrap();

    let transaction_a = fixture
        .store
        .as_mut()
        .unwrap()
        .connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .unwrap();
    let proof_a = schema::verify_v12_transaction(&transaction_a).unwrap();
    assert_eq!(proof_a.check(&transaction_a), Ok(()));
    assert_eq!(schema::verify_v12_read_pass(&transaction_a, &proof_a), Ok(()));
    let transaction_b = second
        .connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .unwrap();
    let proof_b = schema::verify_v12_transaction(&transaction_b).unwrap();
    assert!(matches!(
        proof_a.check(&transaction_b),
        Err(ChainPostCloseError::SchemaRejected)
    ));
    assert_eq!(proof_b.check(&transaction_b), Ok(()));
    drop(proof_b);
    drop(proof_a);
    transaction_b.rollback().unwrap();
    transaction_a.rollback().unwrap();
    second.connection.close().unwrap();
    assert_eq!(catalog_snapshot(fixture.connection()), installed);

    let v12_registry_count: i64 = fixture
        .connection()
        .query_row(
            "SELECT count(*) FROM chain_post_close_layout_objects WHERE layout_version=12",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let transaction = fixture
        .store
        .as_mut()
        .unwrap()
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert_eq!(
        transaction
            .execute(
                "INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
                 SELECT 13,name,object_type,definition FROM chain_post_close_layout_objects \
                 WHERE layout_version=12",
                [],
            )
            .unwrap(),
        usize::try_from(v12_registry_count).unwrap()
    );
    assert_eq!(
        transaction
            .execute(
                "INSERT INTO chain_post_close_layouts( \
                 layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                 artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256) \
                 SELECT 13,12,bundle_sha256,1,1,1,'TEST_CODE_FUTURE_LAYOUT_13',?1 \
                 FROM chain_post_close_layouts WHERE layout_version=12",
                [FUTURE_DIGEST],
            )
            .unwrap(),
        1
    );
    // Layout 14 is the first version this build does not know.
    assert_eq!(
        transaction
            .execute(
                "INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
                 SELECT 14,name,object_type,definition FROM chain_post_close_layout_objects \
                 WHERE layout_version=13",
                [],
            )
            .unwrap(),
        usize::try_from(v12_registry_count).unwrap()
    );
    assert_eq!(
        transaction
            .execute(
                "INSERT INTO chain_post_close_layouts( \
                 layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                 artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256) \
                 SELECT 14,13,bundle_sha256,1,1,1,'TEST_CODE_FUTURE_LAYOUT_14',?1 \
                 FROM chain_post_close_layouts WHERE layout_version=13",
                [FUTURE_DIGEST_14],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        transaction
            .query_row(
                "SELECT MAX(layout_version) FROM chain_post_close_layouts",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        14
    );
    assert!(matches!(
        schema::verify_v12_transaction(&transaction),
        Err(ChainPostCloseError::UnsupportedVersion)
    ));
    transaction.rollback().unwrap();
    assert_eq!(catalog_snapshot(fixture.connection()), installed);

    let transaction = fixture
        .store
        .as_mut()
        .unwrap()
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    transaction
        .execute_batch("DROP TRIGGER chain_post_close_stage_begins_delete")
        .unwrap();
    assert_eq!(
        transaction
            .query_row(
                "SELECT count(*) FROM sqlite_schema \
                 WHERE type='trigger' AND name='chain_post_close_stage_begins_delete'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert!(matches!(
        schema::verify_v12_transaction(&transaction),
        Err(ChainPostCloseError::SchemaRejected)
    ));
    transaction.rollback().unwrap();
    assert_eq!(catalog_snapshot(fixture.connection()), installed);

    let transaction = fixture
        .store
        .as_mut()
        .unwrap()
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let proof = schema::verify_v12_transaction(&transaction).unwrap();
    assert_eq!(proof.check(&transaction), Ok(()));
    let cookie_before = schema_cookie(&transaction);
    transaction
        .execute_batch(
            "CREATE TABLE chain_post_close_TEST_CODE_PROOF_DDL(value INTEGER)",
        )
        .unwrap();
    assert_ne!(schema_cookie(&transaction), cookie_before);
    assert!(matches!(
        proof.check(&transaction),
        Err(ChainPostCloseError::SchemaRejected)
    ));
    drop(proof);
    transaction.rollback().unwrap();
    assert_eq!(catalog_snapshot(fixture.connection()), installed);

    let transaction = fixture
        .store
        .as_mut()
        .unwrap()
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let proof = schema::verify_v12_transaction(&transaction).unwrap();
    assert_eq!(proof.check(&transaction), Ok(()));
    let cookie_before = schema_cookie(&transaction);
    assert_eq!(
        transaction
            .execute(
                "INSERT INTO chain_post_close_layout_objects( \
                 layout_version,name,object_type,definition) \
                 VALUES(13,'TEST_CODE_PROOF_FUTURE','table', \
                 'CREATE TABLE TEST_CODE_PROOF_FUTURE(value INTEGER)')",
                [],
            )
            .unwrap(),
        1
    );
    assert_eq!(
        transaction
            .query_row(
                "SELECT MAX(layout_version) FROM chain_post_close_layouts",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        12
    );
    assert_eq!(schema_cookie(&transaction), cookie_before);
    assert_eq!(
        transaction
            .query_row(
                "SELECT count(*) FROM chain_post_close_layout_objects \
                 WHERE layout_version=13 AND name='TEST_CODE_PROOF_FUTURE'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert!(matches!(
        proof.check(&transaction),
        Err(ChainPostCloseError::SchemaRejected)
    ));
    drop(proof);
    transaction.rollback().unwrap();
    assert_eq!(catalog_snapshot(fixture.connection()), installed);
}

#[test]
fn single_user_local_v12_catalog_proof_rechecks_mutable_run_and_fact_state() {
    let mut fixture = V2BusinessFixture::new();
    install_v12(&mut fixture);
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_CATALOG_PROOF"),
    )
    .unwrap();
    let lease = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap()
        .acquire_run(
            context,
            fixed_input(stocks()),
            lease_request(PROOF_OWNER, 1_000_000, 60_000_000, None),
        )
        .unwrap();
    let intent = lease.intent_id().clone();
    let original_head = lease.head_version();
    let original = run_fact_snapshot(fixture.connection(), &intent);

    let transaction = fixture
        .store
        .as_mut()
        .unwrap()
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let proof = schema::verify_v12_transaction(&transaction).unwrap();
    let (baseline, macro_recovery) =
        inspect_run_and_macro_with_catalog(&transaction, &intent, &proof).unwrap();
    assert_eq!(baseline.head_version(), original_head);
    assert!(macro_recovery.is_none());
    let (owner, generation, head, lease_until, updated_at) = run_row(&transaction, &intent);
    assert_eq!(owner, PROOF_OWNER);
    assert_eq!(u64::try_from(generation).unwrap(), lease.generation());
    assert_eq!(u64::try_from(head).unwrap(), original_head);
    let next_head = head + 1;
    let next_updated_at = updated_at + 1;
    assert!(lease_until > next_updated_at);
    assert_eq!(
        transaction
            .execute(
                "UPDATE chain_post_close_runs SET head_version=?1,updated_at=?2 \
                 WHERE intent_id=?3 AND lease_owner=?4 AND lease_generation=?5 \
                 AND head_version=?6 AND lease_until>?2",
                params![
                    next_head,
                    next_updated_at,
                    intent.as_str(),
                    owner,
                    generation,
                    head,
                ],
            )
            .unwrap(),
        1
    );
    let (advanced, macro_recovery) =
        inspect_run_and_macro_with_catalog(&transaction, &intent, &proof).unwrap();
    assert_eq!(advanced.head_version(), u64::try_from(next_head).unwrap());
    assert!(macro_recovery.is_none());
    assert!(matches!(
        check_lease(
            &transaction,
            &lease,
            UtcMicros::try_new(next_updated_at).unwrap()
        ),
        Err(ChainPostCloseError::StaleLease { intent_id }) if intent_id == intent.as_str()
    ));
    drop(proof);
    transaction.rollback().unwrap();
    assert_eq!(run_fact_snapshot(fixture.connection(), &intent), original);

    let transaction = fixture
        .store
        .as_mut()
        .unwrap()
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let proof = schema::verify_v12_transaction(&transaction).unwrap();
    let cookie_before = schema_cookie(&transaction);
    let (owner, generation, head, lease_until, updated_at) = run_row(&transaction, &intent);
    let next_head = head + 1;
    let next_updated_at = updated_at + 1;
    assert!(lease_until > next_updated_at);
    assert_eq!(
        transaction
            .execute(
                "UPDATE chain_post_close_runs SET head_version=?1,updated_at=?2 \
                 WHERE intent_id=?3 AND lease_owner=?4 AND lease_generation=?5 \
                 AND head_version=?6 AND lease_until>?2",
                params![
                    next_head,
                    next_updated_at,
                    intent.as_str(),
                    owner,
                    generation,
                    head,
                ],
            )
            .unwrap(),
        1
    );
    let request = ConceptProviderRequest::try_new(
        0,
        "TEST_CODE_MISSING_1".to_owned(),
    )
    .unwrap();
    let request_code = request.code;
    let request_bytes = request_code.as_bytes();
    assert_eq!(
        transaction
            .execute(
                "INSERT INTO chain_post_close_stage_begins( \
                 intent_id,effect_kind,effect_ordinal,effect_key,request_codec_version, \
                 request_bytes,request_length,request_sha256,lease_owner,lease_generation, \
                 run_version,begun_at) \
                 VALUES(?1,'ConceptProvider',?2,?3,1,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    intent.as_str(),
                    request.ordinal,
                    request_code.as_str(),
                    request_bytes,
                    i64::try_from(request_bytes.len()).unwrap(),
                    raw_digest(request_bytes).as_str(),
                    owner,
                    generation,
                    next_head,
                    next_updated_at,
                ],
            )
            .unwrap(),
        1
    );
    assert_eq!(schema_cookie(&transaction), cookie_before);
    assert_eq!(
        transaction
            .query_row(
                "SELECT count(*) FROM chain_post_close_stage_begins \
                 WHERE intent_id=?1 AND effect_kind='ConceptProvider' AND effect_ordinal=0",
                [intent.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        transaction
            .query_row(
                "SELECT count(*) FROM chain_post_close_concept_rpc_legacy_outer_qualifications \
                 WHERE intent_id=?1 AND effect_kind='ConceptProvider' AND outer_ordinal=0",
                [intent.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert_eq!(
        transaction
            .query_row(
                "SELECT count(*) FROM chain_post_close_concept_rpc_finals \
                 WHERE intent_id=?1 AND outer_ordinal=0",
                [intent.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert_eq!(proof.check(&transaction), Ok(()));
    assert!(matches!(
        schema::verify_v12_read_pass(&transaction, &proof),
        Err(ChainPostCloseError::SchemaRejected)
    ));
    assert!(matches!(
        inspect_run_and_macro_with_catalog(&transaction, &intent, &proof),
        Err(ChainPostCloseError::SchemaRejected)
    ));
    drop(proof);
    transaction.rollback().unwrap();
    assert_eq!(run_fact_snapshot(fixture.connection(), &intent), original);
}
