use super::*;

fn install_v6(fixture: &mut V2BusinessFixture) {
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
    install_br159_in_owned_database(fixture);
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
}

fn full_catalog(fixture: &V2BusinessFixture) -> Vec<Vec<rusqlite::types::Value>> {
    rows(
        fixture.connection(),
        "SELECT name,type,tbl_name,CAST(sql AS BLOB) \
         FROM main.sqlite_schema ORDER BY type,name",
        4,
    )
}

#[test]
fn warm_v6_reference_still_rejects_live_foreign_attachments_without_repair() {
    for (kind, sql) in [
        (
            "index",
            "CREATE INDEX TEST_CODE_FOREIGN_ERROR_INDEX \
             ON chain_post_close_board_error_materials(run_id);",
        ),
        (
            "trigger",
            "CREATE TRIGGER TEST_CODE_FOREIGN_STATUS_TRIGGER \
             BEFORE INSERT ON chain_post_close_board_status_materials \
             BEGIN SELECT 1; END;",
        ),
    ] {
        let mut damaged = V2BusinessFixture::new();
        install_v6(&mut damaged);
        assert_eq!(
            damaged
                .chain_post_close()
                .verify_schema()
                .unwrap()
                .schema_version(),
            6
        );
        let pristine_catalog = full_catalog(&damaged);
        damaged.execute(sql);
        let damaged_catalog = full_catalog(&damaged);
        assert_ne!(damaged_catalog, pristine_catalog);
        assert_eq!(
            damaged.chain_post_close().verify_schema().err(),
            Some(ChainPostCloseError::SchemaRejected),
            "warm verifier must reject foreign {kind}"
        );
        assert_eq!(full_catalog(&damaged), damaged_catalog);
        damaged.reopen();
        assert_eq!(
            damaged.chain_post_close().verify_schema().err(),
            Some(ChainPostCloseError::SchemaRejected),
            "foreign {kind} must remain rejected after reopen"
        );
        assert_eq!(full_catalog(&damaged), damaged_catalog);
    }

    let mut healthy = V2BusinessFixture::new();
    install_v6(&mut healthy);
    let healthy_catalog = full_catalog(&healthy);
    assert_eq!(
        healthy
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        6
    );
    healthy.reopen();
    assert_eq!(
        healthy
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        6
    );
    assert_eq!(full_catalog(&healthy), healthy_catalog);
}
