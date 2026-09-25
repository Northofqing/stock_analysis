//! Exact v14 migration and catalog attestation; no provider or execution authority.
use super::*;

pub(super) const SQL: &str = include_str!("chain_post_close.v14.sql");
const SHA256: &str = "158041dc5c8a5f78f8b39e52b322c555dddc27e9eb17f62836c7f1eea99d0726";
// Independently measured from v1-v14 frozen DDL; autoindexes are excluded.
const OBJECTS: usize = 257;

#[derive(Clone)]
pub(super) struct V14Bundle {
    pub(super) digest: Sha256Digest,
    pub(super) definitions: Vec<Definition>,
}
static V14: OnceLock<V14Bundle> = OnceLock::new();

impl V14Bundle {
    pub(super) fn verified() -> Result<Self, ChainPostCloseError> {
        cached_bundle(&V14, || {
            let digest = raw_digest(SQL.as_bytes());
            if digest.as_str() != SHA256 {
                return Err(ChainPostCloseError::BundleRejected);
            }
            let _ = v13::V13Bundle::verified()?;
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
                v12::SQL,
                v13::SQL,
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
    bundle: &V14Bundle,
) -> Result<Vec<(i64, Sha256Digest, Vec<RegisteredDefinition>)>, ChainPostCloseError> {
    let mut metadata = v13::predecessor_metadata()?;
    metadata.push((
        14,
        bundle.digest.clone(),
        bundle
            .definitions
            .iter()
            .map(|(name, kind, _, bytes)| (name.clone(), kind.clone(), bytes.clone()))
            .collect(),
    ));
    Ok(metadata)
}

pub(super) fn verify_installed(
    connection: &Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let bundle = V14Bundle::verified()?;
    let v1_bundle = Bundle::verified()?;
    verify_v1_metadata(connection, &v1_bundle)?;
    if latest_layout(connection)? != Some(14) {
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
        schema_version: 14,
    })
}

pub(super) fn run(
    connection: &mut Connection,
    migrate: bool,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let bundle = V14Bundle::verified()?;
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
        .map_err(|_| storage("v14 begin"))?;
    let result = (|| {
        attest_bundled_connection(&transaction).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        verify_safeguards(&transaction, 1)?;
        restore_query_only(&transaction, original)?;
        if migrate {
            // No ddl is applied until the exact v13 catalog, metadata and every
            // old fact / audit have been checked in this same write transaction.
            v13::verify_installed(&transaction)?;
            super::super::validate_migration_audit_facts(&transaction)?;
            super::super::validate_all_runs_at_layout(&transaction, 13)?;
            transaction
                .execute_batch(SQL)
                .map_err(|_| storage("v14 ddl"))?;
            for (name, kind, _, definition) in &bundle.definitions {
                let definition = std::str::from_utf8(definition)
                    .map_err(|_| ChainPostCloseError::BundleRejected)?;
                transaction.execute("INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) VALUES(14,?1,?2,?3)",
                    params![name,kind,definition]).map_err(|_| storage("v14 registry"))?;
            }
            transaction.execute("INSERT INTO chain_post_close_layouts(layout_version,predecessor_layout_version,predecessor_bundle_sha256,artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256) VALUES(14,13,?1,1,1,1,'chain-post-close-layout-v14',?2)",
                params![v13::V13Bundle::verified()?.digest.as_str(),bundle.digest.as_str()])
                .map_err(|_| storage("v14 seal"))?;
        }
        let receipt = verify_installed(&transaction)?;
        super::super::validate_migration_audit_facts(&transaction)?;
        super::super::validate_all_runs_at_layout(&transaction, 14)?;
        Ok(receipt)
    })();
    let result = match result {
        Ok(receipt) => transaction
            .commit()
            .map(|()| receipt)
            .map_err(|_| storage("v14 commit")),
        Err(error) => match transaction.rollback() {
            Ok(()) => Err(error),
            Err(_) => Err(storage("v14 rollback")),
        },
    };
    if !connection.is_autocommit() {
        if connection.execute_batch("ROLLBACK;").is_err() || !connection.is_autocommit() {
            restore_query_only(connection, 1)?;
            return Err(storage("v14 cleanup"));
        }
    }
    restore_query_only(connection, original)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_v14_catalog_is_sealed() {
        V14Bundle::verified().unwrap();
    }
}
