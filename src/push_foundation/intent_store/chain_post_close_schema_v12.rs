//! Exact v12 migration and catalog attestation; no provider or execution authority.
use super::*;

pub(super) const SQL: &str = include_str!("chain_post_close.v12.sql");
const SHA256: &str = "2a1a6969708b8df9feacb20b4474039e3b04d3c95ae7f849926defb1377021af";
// Independently measured from v1-v12 frozen DDL; autoindexes are excluded.
const OBJECTS: usize = 241;

#[derive(Clone)]
pub(super) struct V12Bundle {
    pub(super) digest: Sha256Digest,
    pub(super) definitions: Vec<Definition>,
}
static V12: OnceLock<V12Bundle> = OnceLock::new();

impl V12Bundle {
    pub(super) fn verified() -> Result<Self, ChainPostCloseError> {
        cached_bundle(&V12, || {
            let digest = raw_digest(SQL.as_bytes());
            if digest.as_str() != SHA256 {
                return Err(ChainPostCloseError::BundleRejected);
            }
            let _ = v11::V11Bundle::verified()?;
            let reference =
                Connection::open_in_memory().map_err(|_| ChainPostCloseError::BundleRejected)?;
            install_v10_bundle_audit_stubs(&reference)
                .map_err(|_| ChainPostCloseError::BundleRejected)?;
            for ddl in [
                DDL,
                V2_DDL,
                V3_DDL,
                V4_DDL,
                V5_DDL,
                V6_DDL,
                V7_DDL,
                V8_DDL,
                V9_DDL,
                V10_DDL,
                v11::SQL,
                SQL,
            ] {
                reference
                    .execute_batch(ddl)
                    .map_err(|_| ChainPostCloseError::BundleRejected)?;
            }
            let definitions = v11::owned_catalog(&reference)?;
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

fn predecessors(
    bundle: &V12Bundle,
) -> Result<Vec<(i64, Sha256Digest, Vec<RegisteredDefinition>)>, ChainPostCloseError> {
    let prior = v11::V11Bundle::verified()?;
    let mut metadata = v11::predecessor_metadata()?;
    for (version, digest, definitions) in [
        (11, &prior.digest, &prior.definitions),
        (12, &bundle.digest, &bundle.definitions),
    ] {
        metadata.push((
            version,
            digest.clone(),
            definitions
                .iter()
                .map(|(name, kind, _, bytes)| (name.clone(), kind.clone(), bytes.clone()))
                .collect(),
        ));
    }
    Ok(metadata)
}

/// Registered metadata of every layout up to and including v12, for later layouts.
pub(super) fn predecessor_metadata(
) -> Result<Vec<(i64, Sha256Digest, Vec<RegisteredDefinition>)>, ChainPostCloseError> {
    predecessors(&V12Bundle::verified()?)
}

pub(super) fn verify_installed(
    connection: &Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let bundle = V12Bundle::verified()?;
    let v1_bundle = Bundle::verified()?;
    verify_v1_metadata(connection, &v1_bundle)?;
    if latest_layout(connection)? != Some(12) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    let actual = v11::owned_catalog(connection)?;
    if actual != bundle.definitions {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let predecessor_metadata = predecessors(&bundle)?;
    v11::verify_layout_metadata(connection, predecessor_metadata)?;
    verify_v7_fact_shape(connection)?;
    Ok(ChainPostCloseSchemaReceipt {
        ddl_sha256: bundle.digest,
        schema_version: 12,
    })
}

pub(super) fn run(
    connection: &mut Connection,
    migrate: bool,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let bundle = V12Bundle::verified()?;
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
        .map_err(|_| storage("v12 begin"))?;
    let result = (|| {
        attest_bundled_connection(&transaction).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        verify_safeguards(&transaction, 1)?;
        restore_query_only(&transaction, original)?;
        if migrate {
            // No ddl is applied until the exact v11 catalog, metadata and every
            // old fact / audit have been checked in this same write transaction.
            v11::verify_installed(&transaction)?;
            super::super::validate_migration_audit_facts(&transaction)?;
            super::super::validate_all_runs_at_layout(&transaction, 11)?;
            transaction
                .execute_batch(SQL)
                .map_err(|_| storage("v12 ddl"))?;
            for (name, kind, _, definition) in &bundle.definitions {
                let definition = std::str::from_utf8(definition)
                    .map_err(|_| ChainPostCloseError::BundleRejected)?;
                transaction.execute("INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) VALUES(12,?1,?2,?3)",
                    params![name,kind,definition]).map_err(|_| storage("v12 registry"))?;
            }
            transaction.execute("INSERT INTO chain_post_close_layouts(layout_version,predecessor_layout_version,predecessor_bundle_sha256,artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256) VALUES(12,11,?1,1,1,1,'chain-post-close-layout-v12',?2)",
                params![v11::V11Bundle::verified()?.digest.as_str(),bundle.digest.as_str()])
                .map_err(|_| storage("v12 seal"))?;
        }
        let receipt = verify_installed(&transaction)?;
        super::super::validate_migration_audit_facts(&transaction)?;
        super::super::validate_all_runs_at_layout(&transaction, 12)?;
        Ok(receipt)
    })();
    let result = match result {
        Ok(receipt) => transaction
            .commit()
            .map(|()| receipt)
            .map_err(|_| storage("v12 commit")),
        Err(error) => match transaction.rollback() {
            Ok(()) => Err(error),
            Err(_) => Err(storage("v12 rollback")),
        },
    };
    if !connection.is_autocommit() {
        if connection.execute_batch("ROLLBACK;").is_err() || !connection.is_autocommit() {
            restore_query_only(connection, 1)?;
            return Err(storage("v12 cleanup"));
        }
    }
    restore_query_only(connection, original)?;
    result
}
