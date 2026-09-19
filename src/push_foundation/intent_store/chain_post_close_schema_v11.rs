//! Controlled v11 extension. Prior bundles and their exact readers are unchanged.
use super::*;

pub(super) const SQL: &str = include_str!("chain_post_close.v11.sql");
const SHA256: &str = "8ee02c8ab5bb7e23ee7904f4db08ccc86b7f504a7ae86b37fc88c75d4a453faa";
const OBJECTS: usize = 225;

#[derive(Clone)]
pub(super) struct V11Bundle {
    pub(super) digest: Sha256Digest,
    pub(super) definitions: Vec<Definition>,
}
static V11: OnceLock<V11Bundle> = OnceLock::new();

pub(super) fn owned_catalog(connection: &Connection) -> Result<Vec<Definition>, ChainPostCloseError> {
    let mut statement=connection.prepare("SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema WHERE (lower(name) GLOB 'chain_post_close_*' OR lower(tbl_name) GLOB 'chain_post_close_*') AND NOT(type='index' AND name GLOB 'sqlite_autoindex_*' AND sql IS NULL) ORDER BY name")
        .map_err(|_|ChainPostCloseError::SchemaRejected)?;
    let definitions = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    Ok(definitions)
}

impl V11Bundle {
    pub(super) fn verified() -> Result<Self, ChainPostCloseError> {
        cached_bundle(&V11, || {
            let digest = raw_digest(SQL.as_bytes());
            if digest.as_str() != SHA256 {
                return Err(ChainPostCloseError::BundleRejected);
            }
            // Verify every frozen predecessor independently before evaluating SQL.
            let _ = predecessor_metadata()?;
            let reference =
                Connection::open_in_memory().map_err(|_| ChainPostCloseError::BundleRejected)?;
            install_v10_bundle_audit_stubs(&reference)
                .map_err(|_| ChainPostCloseError::BundleRejected)?;
            for sql in [
                DDL, V2_DDL, V3_DDL, V4_DDL, V5_DDL, V6_DDL, V7_DDL, V8_DDL, V9_DDL, V10_DDL, SQL,
            ] {
                reference
                    .execute_batch(sql)
                    .map_err(|_| ChainPostCloseError::BundleRejected)?;
            }
            let definitions = owned_catalog(&reference)?;
            if definitions.len() != OBJECTS {
                return Err(ChainPostCloseError::BundleRejected);
            }
            Ok(Self {
                digest,
                definitions,
            })
        })
    }
}

pub(super) fn predecessor_metadata(
) -> Result<Vec<(i64, Sha256Digest, Vec<RegisteredDefinition>)>, ChainPostCloseError> {
    let v1 = Bundle::verified()?;
    let v2 = V2Bundle::verified()?;
    let v3 = V3Bundle::verified()?;
    let v4 = V4Bundle::verified()?;
    let v5 = V5Bundle::verified()?;
    let v6 = V6Bundle::verified()?;
    let v7 = V7Bundle::verified()?;
    let v8 = V8Bundle::verified()?;
    let v9 = V9Bundle::verified()?;
    let v10 = V10Bundle::verified()?;
    Ok(vec![
        (1, v1.digest.clone(), v1.registered_definitions()),
        (2, v2.digest.clone(), v2.registered_definitions()),
        (3, v3.digest.clone(), v3.registered_definitions()),
        (4, v4.digest.clone(), v4.registered_definitions()),
        (5, v5.digest.clone(), v5.registered_definitions()),
        (6, v6.digest.clone(), v6.registered_definitions()),
        (7, v7.digest.clone(), v7.registered_definitions()),
        (8, v8.digest.clone(), v8.registered_definitions()),
        (9, v9.digest.clone(), v9.registered_definitions()),
        (10, v10.digest.clone(), v10.registered_definitions()),
    ])
}

pub(super) fn verify_installed(
    connection: &Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let bundle = V11Bundle::verified()?;
    verify_v1_metadata(connection, &Bundle::verified()?)?;
    if latest_layout(connection)? != Some(11) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    if owned_catalog(connection)? != bundle.definitions {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let mut predecessors = predecessor_metadata()?;
    predecessors.push((
        11,
        bundle.digest.clone(),
        bundle
            .definitions
            .iter()
            .map(|(name, kind, _, sql)| (name.clone(), kind.clone(), sql.clone()))
            .collect(),
    ));
    verify_layout_metadata(connection, predecessors)?;
    verify_v7_fact_shape(connection)?;
    Ok(ChainPostCloseSchemaReceipt {
        ddl_sha256: bundle.digest,
        schema_version: 11,
    })
}

/// Exact metadata comparison shared only by verified frozen layout bundles.
pub(super) fn verify_layout_metadata(
    connection: &Connection,
    predecessors: Vec<(i64, Sha256Digest, Vec<RegisteredDefinition>)>,
) -> Result<(), ChainPostCloseError> {
    let mut statement=connection.prepare("SELECT layout_version,predecessor_layout_version,predecessor_bundle_sha256,artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256 FROM chain_post_close_layouts ORDER BY layout_version")
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let layouts = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if layouts.len() + 1 != predecessors.len() {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    for (index, row) in layouts.iter().enumerate() {
        let (version, digest, _) = &predecessors[index + 1];
        if row.0 != *version
            || row.1 != version - 1
            || row.2 != predecessors[index].1.as_str()
            || (row.3, row.4, row.5) != (1, 1, 1)
            || row.6 != format!("chain-post-close-layout-v{version}")
            || row.7 != digest.as_str()
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
    }
    drop(statement);
    let expected: Vec<_> = predecessors
        .into_iter()
        .skip(1)
        .flat_map(|(version, _, definitions)| {
            definitions
                .into_iter()
                .map(move |(name, kind, sql)| (version, name, kind, sql))
        })
        .collect();
    let mut statement=connection.prepare("SELECT layout_version,name,object_type,CAST(definition AS BLOB) FROM chain_post_close_layout_objects ORDER BY layout_version,name")
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let actual = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if actual != expected {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}

pub(super) fn run(
    connection: &mut Connection,
    migrate: bool,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let bundle = V11Bundle::verified()?;
    if !connection.is_autocommit() {
        return Err(ChainPostCloseError::ConnectionSafeguardFailed);
    }
    let original = pragma(connection, "PRAGMA query_only")?;
    if !matches!(original, 0 | 1) || (migrate && original != 0) {
        return Err(ChainPostCloseError::ConnectionSafeguardFailed);
    }
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON; PRAGMA synchronous=FULL;",
        )
        .map_err(|_| ChainPostCloseError::ConnectionSafeguardFailed)?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|_| ChainPostCloseError::ConnectionSafeguardFailed)?;
    verify_safeguards(connection, original)?;
    let transaction = connection
        .transaction_with_behavior(if migrate {
            TransactionBehavior::Immediate
        } else {
            TransactionBehavior::Deferred
        })
        .map_err(|_| storage("v11 begin"))?;
    let result = (|| {
        attest_bundled_connection(&transaction).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        verify_safeguards(&transaction, 1)?;
        restore_query_only(&transaction, original)?;
        if migrate {
            if owned_catalog(&transaction)? != V10Bundle::verified()?.definitions {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            verify_v10_installed(
                &transaction,
                &Bundle::verified()?,
                &V2Bundle::verified()?,
                &V3Bundle::verified()?,
                &V4Bundle::verified()?,
                &V5Bundle::verified()?,
                &V6Bundle::verified()?,
                &V7Bundle::verified()?,
                &V8Bundle::verified()?,
                &V9Bundle::verified()?,
                &V10Bundle::verified()?,
            )?;
            super::super::validate_migration_audit_facts(&transaction)?;
            super::super::validate_all_runs_at_layout(&transaction, 10)?;
            transaction
                .execute_batch(SQL)
                .map_err(|_| storage("v11 ddl"))?;
            for (name, kind, _, definition) in &bundle.definitions {
                let definition = std::str::from_utf8(definition)
                    .map_err(|_| ChainPostCloseError::BundleRejected)?;
                transaction.execute("INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) VALUES(11,?1,?2,?3)",
                    params![name,kind,definition]).map_err(|_|storage("v11 registry"))?;
            }
            transaction.execute("INSERT INTO chain_post_close_layouts(layout_version,predecessor_layout_version,predecessor_bundle_sha256,artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256) VALUES(11,10,?1,1,1,1,'chain-post-close-layout-v11',?2)",
                params![V10Bundle::verified()?.digest.as_str(),bundle.digest.as_str()]).map_err(|_|storage("v11 seal"))?;
        }
        let receipt = verify_installed(&transaction)?;
        super::super::validate_migration_audit_facts(&transaction)?;
        super::super::validate_all_runs_at_layout(&transaction, 11)?;
        Ok(receipt)
    })();
    let result = match result {
        Ok(receipt) => transaction
            .commit()
            .map(|()| receipt)
            .map_err(|_| storage("v11 commit")),
        Err(error) => match transaction.rollback() {
            Ok(()) => Err(error),
            Err(_) => Err(storage("v11 rollback")),
        },
    };
    if !connection.is_autocommit() {
        if connection.execute_batch("ROLLBACK;").is_err() || !connection.is_autocommit() {
            restore_query_only(connection, 1)?;
            return Err(storage("v11 cleanup"));
        }
    }
    restore_query_only(connection, original)?;
    result
}
