//! Fixed schema installation and attestation on the parent's existing SQLite handle.

use std::sync::OnceLock;

use rusqlite::{params, Connection, TransactionBehavior};

use crate::monitor::push_job::{raw_digest, Sha256Digest};
use crate::push_foundation::migration::attest_bundled_connection;

use super::super::BUSY_TIMEOUT;
use super::{ChainPostCloseError, ChainPostCloseSchemaReceipt};

const DDL: &str = include_str!("chain_post_close.v1.sql");
const DDL_SHA256: &str = "cfaedcafa3bda35942404b874e954a3b88c764a1600e9060a496163721742cb5";
const OBJECT_COUNT: usize = 8;
const V2_DDL: &str = include_str!("chain_post_close.v2.sql");
const V2_DDL_SHA256: &str = "39280ac92da2068c37484cab83124f5bd6de8d220cbfe5fbd2f1427fe15b5608";
const V2_OBJECT_COUNT: usize = 27;
const V3_DDL: &str = include_str!("chain_post_close.v3.sql");
const V3_DDL_SHA256: &str = "165dbf8bbae2458d6616d6973722e4b056412f6777a9690a670878541cb18f75";
const V3_OBJECT_COUNT: usize = 31;
const V4_DDL: &str = include_str!("chain_post_close.v4.sql");
const V4_DDL_SHA256: &str = "476707c06cc32e9a7c7d5b9da249c9820af497606f71230afce9f09b9b2010be";
const V4_OBJECT_COUNT: usize = 43;
const V5_DDL: &str = include_str!("chain_post_close.v5.sql");
const V5_DDL_SHA256: &str = "b2c48142faf90409b7d54d028a2deecf7f62a9f3665c2087dfb3961e3f521918";
const V5_OBJECT_COUNT: usize = 63;
const V6_DDL: &str = include_str!("chain_post_close.v6.sql");
const V6_DDL_SHA256: &str = "ab520eb71a62ea476777ab525f1fb2f52823c0d035b7defcebc8834c0cea945e";
const V6_OBJECT_COUNT: usize = 71;
const V7_DDL: &str = include_str!("chain_post_close.v7.sql");
const V7_DDL_SHA256: &str = "3e5c0c8443c5fcc6ea3c99bd318134d86579aba2d16bc055e51036ad639509ca";
const V7_OBJECT_COUNT: usize = 99;
const V8_DDL: &str = include_str!("chain_post_close.v8.sql");
const V8_DDL_SHA256: &str = "64ef5d43911025558d92815a94e552f1fe21dabe6f756c885cff08cda697bed6";
const V8_OBJECT_COUNT: usize = 107;
const V9_DDL: &str = include_str!("chain_post_close.v9.sql");
const V9_DDL_SHA256: &str = "754b2aa8d4cffd9072c32e6ae14789c19f5093fdf6e7b0a08cbc20d244a498c8";
const V9_OBJECT_COUNT: usize = 135;
const V10_DDL: &str = include_str!("chain_post_close.v10.sql");
const V10_DDL_SHA256: &str = "1f66f45fb534da1fa7b77fedf161924b60a6e2b69ae1a7c4d1577ab04125ccd8";
const V10_OBJECT_COUNT: usize = 159;
#[path = "chain_post_close_schema_v11.rs"]
mod v11;
#[path = "chain_post_close_schema_v12.rs"]
mod v12;
#[path = "chain_post_close_schema_v13.rs"]
mod v13;

/// Newest sealed layout this build can attest and write.
pub(super) const CURRENT_LAYOUT: i64 = 13;

// Include foreign-named objects attached to our tables, not just our own name prefix.
const CATALOG_SQL: &str = "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM main.sqlite_schema \
     WHERE (lower(name) GLOB 'chain_post_close_*' \
       OR lower(tbl_name) IN ('chain_post_close_schema','chain_post_close_objects')) \
       AND NOT (type='index' AND name GLOB 'sqlite_autoindex_*' AND sql IS NULL) ORDER BY name";

type Definition = (String, String, String, Vec<u8>);
type RegisteredDefinition = (String, String, Vec<u8>);

fn cached_bundle<T: Clone>(
    cache: &OnceLock<T>,
    build: impl FnOnce() -> Result<T, ChainPostCloseError>,
) -> Result<T, ChainPostCloseError> {
    if let Some(bundle) = cache.get() {
        return Ok(bundle.clone());
    }
    let bundle = build()?;
    Ok(cache.get_or_init(|| bundle).clone())
}

#[derive(Clone)]
struct Bundle {
    digest: Sha256Digest,
    definitions: Vec<Definition>,
}
static BUNDLE: OnceLock<Bundle> = OnceLock::new();

impl Bundle {
    fn verified() -> Result<Self, ChainPostCloseError> {
        cached_bundle(&BUNDLE, Self::build)
    }

    fn build() -> Result<Self, ChainPostCloseError> {
        let digest = raw_digest(DDL.as_bytes());
        if digest.as_str() != DDL_SHA256 {
            return Err(ChainPostCloseError::BundleRejected);
        }
        let reference =
            Connection::open_in_memory().map_err(|_| ChainPostCloseError::BundleRejected)?;
        reference
            .execute_batch(DDL)
            .map_err(|_| ChainPostCloseError::BundleRejected)?;
        let definitions = catalog(&reference)?;
        if definitions.len() != OBJECT_COUNT {
            return Err(ChainPostCloseError::BundleRejected);
        }
        Ok(Self {
            digest,
            definitions,
        })
    }

    fn registered_definitions(&self) -> Vec<RegisteredDefinition> {
        self.definitions
            .iter()
            .map(|(name, kind, _, sql)| (name.clone(), kind.clone(), sql.clone()))
            .collect()
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Operation {
    Install,
    Verify,
}

#[derive(Eq, PartialEq)]
struct LegacyQualificationSnapshot {
    intent_id: String,
    effect_kind: String,
    outer_ordinal: i64,
    code: String,
    qualification_kind: String,
    begin_run_version: i64,
    begin_request_sha256: String,
    begin_lease_owner: String,
    begin_lease_generation: i64,
    begun_at: i64,
    result_outcome: Option<String>,
    result_run_version: Option<i64>,
    result_sha256: Option<String>,
    result_lease_owner: Option<String>,
    result_lease_generation: Option<i64>,
    result_committed_at: Option<i64>,
    cache_run_version: Option<i64>,
    cache_sha256: Option<String>,
    cache_lease_owner: Option<String>,
    cache_lease_generation: Option<i64>,
    cache_written_at: Option<i64>,
    qualified_from_layout_version: i64,
    sealed_by_layout_version: i64,
}

pub(super) fn install(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    run(connection, Operation::Install)
}

pub(super) fn verify(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    run(connection, Operation::Verify)
}

pub(super) fn verify_current(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let has_v2: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema \
             WHERE type='table' AND lower(name)='chain_post_close_layouts'",
            [],
            |row| row.get(0),
        )
        .map_err(|_| storage("v2_probe"))?;
    if has_v2 == 0 {
        return verify(connection);
    }
    match latest_layout(connection)? {
        Some(2) => run_v2(connection, V2Operation::Verify),
        Some(3) => run_v3(connection, V3Operation::Verify),
        Some(4) => run_v4(connection, V4Operation::Verify),
        Some(5) => run_v5(connection, V5Operation::Verify),
        Some(6) => run_v6(connection, V6Operation::Verify),
        Some(7) => run_v7(connection, V7Operation::Verify),
        Some(8) => run_v8(connection, V8Operation::Verify),
        Some(9) => run_v9(connection, V9Operation::Verify),
        Some(10) => run_v10(connection, V10Operation::Verify),
        Some(11) => v11::run(connection, false),
        Some(12) => v12::run(connection, false),
        Some(13) => v13::run(connection, false),
        Some(version) if version > 10 => Err(ChainPostCloseError::UnsupportedVersion),
        _ => run_v2(connection, V2Operation::Verify),
    }
}

pub(super) fn migrate_v2(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    // The frozen reader proves the exact predecessor before the migration transaction.
    verify(connection)?;
    run_v2(connection, V2Operation::Migrate)
}

pub(super) fn migrate_v3(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    run_v2(connection, V2Operation::Verify)?;
    run_v3(connection, V3Operation::Migrate)
}

pub(super) fn migrate_v4(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    run_v3(connection, V3Operation::Verify)?;
    run_v4(connection, V4Operation::Migrate)
}

pub(super) fn migrate_v5(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    run_v4(connection, V4Operation::Verify)?;
    run_v5(connection, V5Operation::Migrate)
}

pub(super) fn migrate_v6(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    run_v5(connection, V5Operation::Verify)?;
    run_v6(connection, V6Operation::Migrate)
}

pub(super) fn migrate_v7(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    run_v6(connection, V6Operation::Verify)?;
    run_v7(connection, V7Operation::Migrate)
}

pub(super) fn migrate_v9(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    run_v9(connection, V9Operation::Migrate)
}

pub(super) fn migrate_v10(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    run_v10(connection, V10Operation::Migrate)
}

pub(super) fn migrate_v11(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    v11::run(connection, true)
}

pub(super) fn migrate_v12(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    v12::run(connection, true)
}

pub(super) fn migrate_v13(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    v13::run(connection, true)
}

/// Internal historical-fact readers accept the current sealed layout (v12 or
/// v13) only after exact verification on their own current transaction. This
/// is not a versioned legacy facade.
pub(super) fn verify_parent_layout_v12(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), ChainPostCloseError> {
    match latest_layout(transaction)? {
        Some(12) => v12::verify_installed(transaction).map(|_| ()),
        Some(13) => v13::verify_installed(transaction).map(|_| ()),
        _ => Err(ChainPostCloseError::UnsupportedVersion),
    }
}

/// Installed metadata authority for exactly one live transaction. Never a fact,
/// lease, head or audit cache; only the verifier below can mint this value.
/// Limited to the closed fact writer: no DDL, metadata writes, transaction
/// control, or savepoint rollback spanning this proof's creation.
pub(super) struct V12CatalogProof<'transaction, 'connection> {
    transaction: &'transaction rusqlite::Transaction<'connection>,
    schema_cookie: i64,
    layout: i64,
}

impl V12CatalogProof<'_, '_> {
    pub(super) fn check(
        &self,
        transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), ChainPostCloseError> {
        if !std::ptr::eq(self.transaction, transaction)
            || transaction.is_autocommit()
            || pragma(transaction, "PRAGMA main.schema_version")? != self.schema_cookie
            || latest_layout(transaction)? != Some(self.layout)
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        // Versions 2..=layout are sealed. Even a deferred-FK row in an unsealed
        // registry namespace must invalidate the already-minted proof.
        let unsealed: i64 = transaction
            .query_row(
                "SELECT count(*) FROM chain_post_close_layout_objects WHERE layout_version NOT BETWEEN 2 AND ?1",
                [self.layout],
                |row| row.get(0),
            )
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        if unsealed != 0 {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        Ok(())
    }

    pub(super) fn layout(&self) -> i64 {
        self.layout
    }
}

pub(super) fn transaction_layout<'transaction, 'connection>(
    transaction: &'transaction rusqlite::Transaction<'connection>,
) -> Result<(i64, Option<V12CatalogProof<'transaction, 'connection>>), ChainPostCloseError> {
    let layout = runtime_layout_version(transaction)?;
    let proof = if matches!(layout, 12 | 13) {
        let proof = V12CatalogProof {
            transaction,
            schema_cookie: pragma(transaction, "PRAGMA main.schema_version")?,
            layout,
        };
        proof.check(transaction)?;
        Some(proof)
    } else {
        None
    };
    Ok((layout, proof))
}

pub(super) fn verify_v12_transaction<'transaction, 'connection>(
    transaction: &'transaction rusqlite::Transaction<'connection>,
) -> Result<V12CatalogProof<'transaction, 'connection>, ChainPostCloseError> {
    transaction_layout(transaction)?
        .1
        .ok_or(ChainPostCloseError::UnsupportedVersion)
}

pub(super) fn verify_parent_layout_v12_scoped(
    transaction: &rusqlite::Transaction<'_>,
    proof: Option<&V12CatalogProof<'_, '_>>,
) -> Result<(), ChainPostCloseError> {
    match proof {
        Some(proof) => proof.check(transaction),
        None => verify_parent_layout_v12(transaction),
    }
}

pub(super) fn verify_v12_read_pass(
    transaction: &rusqlite::Transaction<'_>,
    proof: &V12CatalogProof<'_, '_>,
) -> Result<(), ChainPostCloseError> {
    proof.check(transaction)?;
    verify_safeguards(transaction, pragma(transaction, "PRAGMA query_only")?)?;
    // This is mutable persisted evidence, not part of the metadata proof.
    verify_v7_fact_shape(transaction)
}

pub(super) fn migrate_v8(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    run_v7(connection, V7Operation::Verify)?;
    run_v8(connection, V8Operation::Migrate)
}

#[cfg(test)]
pub(super) fn verify_v2_reader(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    run_v2(connection, V2Operation::Verify)
}

#[cfg(test)]
pub(super) fn verify_v3_reader(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    run_v3(connection, V3Operation::Verify)
}

#[cfg(test)]
pub(super) fn verify_v10_reader(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    run_v10(connection, V10Operation::Verify)
}

#[cfg(test)]
pub(super) fn verify_v11_reader(
    connection: &mut Connection,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    v11::run(connection, false)
}

fn v2_catalog(connection: &Connection) -> Result<Vec<Definition>, ChainPostCloseError> {
    const SQL: &str = "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
        WHERE (lower(name) GLOB 'chain_post_close_*' OR lower(tbl_name) IN \
        ('chain_post_close_schema','chain_post_close_objects','chain_post_close_layouts', \
         'chain_post_close_layout_objects','chain_post_close_runs', \
         'chain_post_close_stage_begins','chain_post_close_stage_results')) \
        AND NOT(type='index' AND name GLOB 'sqlite_autoindex_*' AND sql IS NULL) \
        ORDER BY name";
    let mut statement = connection.prepare(SQL).map_err(|_| storage("v2_catalog"))?;
    let definitions = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|_| storage("v2_catalog"))?
        .collect::<rusqlite::Result<_>>()
        .map_err(|_| storage("v2_catalog"))?;
    Ok(definitions)
}

#[derive(Clone)]
struct V2Bundle {
    digest: Sha256Digest,
    definitions: Vec<Definition>,
}
static V2_BUNDLE: OnceLock<V2Bundle> = OnceLock::new();

impl V2Bundle {
    fn verified() -> Result<Self, ChainPostCloseError> {
        cached_bundle(&V2_BUNDLE, Self::build)
    }

    fn build() -> Result<Self, ChainPostCloseError> {
        let digest = raw_digest(V2_DDL.as_bytes());
        if digest.as_str() != V2_DDL_SHA256 {
            return Err(ChainPostCloseError::BundleRejected);
        }
        let connection =
            Connection::open_in_memory().map_err(|_| ChainPostCloseError::BundleRejected)?;
        connection
            .execute_batch(DDL)
            .and_then(|_| connection.execute_batch(V2_DDL))
            .map_err(|_| ChainPostCloseError::BundleRejected)?;
        let definitions =
            v2_catalog(&connection).map_err(|_| ChainPostCloseError::BundleRejected)?;
        if definitions.len() != V2_OBJECT_COUNT {
            return Err(ChainPostCloseError::BundleRejected);
        }
        Ok(Self {
            digest,
            definitions,
        })
    }

    fn registered_definitions(&self) -> Vec<RegisteredDefinition> {
        self.definitions
            .iter()
            .map(|(name, kind, _, sql)| (name.clone(), kind.clone(), sql.clone()))
            .collect()
    }
}

fn v3_catalog(connection: &Connection) -> Result<Vec<Definition>, ChainPostCloseError> {
    const SQL: &str = "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
        WHERE (lower(name) GLOB 'chain_post_close_*' OR lower(tbl_name) IN \
        ('chain_post_close_schema','chain_post_close_objects','chain_post_close_layouts', \
         'chain_post_close_layout_objects','chain_post_close_runs', \
         'chain_post_close_stage_begins','chain_post_close_stage_results', \
         'chain_post_close_concept_cache_writes')) \
        AND NOT(type='index' AND name GLOB 'sqlite_autoindex_*' AND sql IS NULL) \
        ORDER BY name";
    let mut statement = connection.prepare(SQL).map_err(|_| storage("v3_catalog"))?;
    let definitions = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|_| storage("v3_catalog"))?
        .collect::<rusqlite::Result<_>>()
        .map_err(|_| storage("v3_catalog"))?;
    Ok(definitions)
}

#[derive(Clone)]
struct V3Bundle {
    digest: Sha256Digest,
    definitions: Vec<Definition>,
}
static V3_BUNDLE: OnceLock<V3Bundle> = OnceLock::new();

impl V3Bundle {
    fn verified() -> Result<Self, ChainPostCloseError> {
        cached_bundle(&V3_BUNDLE, Self::build)
    }

    fn build() -> Result<Self, ChainPostCloseError> {
        let digest = raw_digest(V3_DDL.as_bytes());
        if digest.as_str() != V3_DDL_SHA256 {
            return Err(ChainPostCloseError::BundleRejected);
        }
        let connection =
            Connection::open_in_memory().map_err(|_| ChainPostCloseError::BundleRejected)?;
        connection
            .execute_batch(DDL)
            .and_then(|_| connection.execute_batch(V2_DDL))
            .and_then(|_| connection.execute_batch(V3_DDL))
            .map_err(|_| ChainPostCloseError::BundleRejected)?;
        let definitions =
            v3_catalog(&connection).map_err(|_| ChainPostCloseError::BundleRejected)?;
        if definitions.len() != V3_OBJECT_COUNT {
            return Err(ChainPostCloseError::BundleRejected);
        }
        Ok(Self {
            digest,
            definitions,
        })
    }

    fn registered_definitions(&self) -> Vec<RegisteredDefinition> {
        self.definitions
            .iter()
            .map(|(name, kind, _, sql)| (name.clone(), kind.clone(), sql.clone()))
            .collect()
    }
}

fn v4_catalog(connection: &Connection) -> Result<Vec<Definition>, ChainPostCloseError> {
    const SQL: &str = "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
        WHERE (lower(name) GLOB 'chain_post_close_*' OR lower(tbl_name) IN \
        ('chain_post_close_schema','chain_post_close_objects','chain_post_close_layouts', \
         'chain_post_close_layout_objects','chain_post_close_runs', \
         'chain_post_close_stage_begins','chain_post_close_stage_results', \
         'chain_post_close_concept_cache_writes', \
         'chain_post_close_cluster_configurations','chain_post_close_cluster_materials', \
         'chain_post_close_chain_daily_applications')) \
        AND NOT(type='index' AND name GLOB 'sqlite_autoindex_*' AND sql IS NULL) \
        ORDER BY name";
    let mut statement = connection.prepare(SQL).map_err(|_| storage("v4_catalog"))?;
    let definitions = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|_| storage("v4_catalog"))?
        .collect::<rusqlite::Result<_>>()
        .map_err(|_| storage("v4_catalog"))?;
    Ok(definitions)
}

#[derive(Clone)]
struct V4Bundle {
    digest: Sha256Digest,
    definitions: Vec<Definition>,
}
static V4_BUNDLE: OnceLock<V4Bundle> = OnceLock::new();

fn v5_catalog(connection: &Connection) -> Result<Vec<Definition>, ChainPostCloseError> {
    const SQL: &str = "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
        WHERE (lower(name) GLOB 'chain_post_close_*' OR lower(tbl_name) IN \
        ('chain_post_close_schema','chain_post_close_objects','chain_post_close_layouts', \
         'chain_post_close_layout_objects','chain_post_close_runs', \
         'chain_post_close_stage_begins','chain_post_close_stage_results', \
         'chain_post_close_concept_cache_writes', \
         'chain_post_close_cluster_configurations','chain_post_close_cluster_materials', \
         'chain_post_close_chain_daily_applications', \
         'chain_post_close_board_attempt_begins','chain_post_close_board_attempt_results', \
         'chain_post_close_board_kind_finals','chain_post_close_board_directory_materials', \
         'chain_post_close_board_selections')) \
        AND NOT(type='index' AND name GLOB 'sqlite_autoindex_*' AND sql IS NULL) \
        ORDER BY name";
    let mut statement = connection.prepare(SQL).map_err(|_| storage("v5_catalog"))?;
    let definitions = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|_| storage("v5_catalog"))?
        .collect::<rusqlite::Result<_>>()
        .map_err(|_| storage("v5_catalog"))?;
    Ok(definitions)
}

#[derive(Clone)]
struct V5Bundle {
    digest: Sha256Digest,
    definitions: Vec<Definition>,
}
static V5_BUNDLE: OnceLock<V5Bundle> = OnceLock::new();

fn v6_catalog(connection: &Connection) -> Result<Vec<Definition>, ChainPostCloseError> {
    const SQL: &str = "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
        WHERE (lower(name) GLOB 'chain_post_close_*' OR lower(tbl_name) IN \
        ('chain_post_close_schema','chain_post_close_objects','chain_post_close_layouts', \
         'chain_post_close_layout_objects','chain_post_close_runs', \
         'chain_post_close_stage_begins','chain_post_close_stage_results', \
         'chain_post_close_concept_cache_writes', \
         'chain_post_close_cluster_configurations','chain_post_close_cluster_materials', \
         'chain_post_close_chain_daily_applications', \
         'chain_post_close_board_attempt_begins','chain_post_close_board_attempt_results', \
         'chain_post_close_board_kind_finals','chain_post_close_board_directory_materials', \
         'chain_post_close_board_selections','chain_post_close_board_status_materials', \
         'chain_post_close_board_error_materials')) \
        AND NOT(type='index' AND name GLOB 'sqlite_autoindex_*' AND sql IS NULL) \
        ORDER BY name";
    let mut statement = connection.prepare(SQL).map_err(|_| storage("v6_catalog"))?;
    let definitions = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|_| storage("v6_catalog"))?
        .collect::<rusqlite::Result<_>>()
        .map_err(|_| storage("v6_catalog"))?;
    Ok(definitions)
}

#[derive(Clone)]
struct V6Bundle {
    digest: Sha256Digest,
    definitions: Vec<Definition>,
}
static V6_BUNDLE: OnceLock<V6Bundle> = OnceLock::new();

fn v7_catalog(connection: &Connection) -> Result<Vec<Definition>, ChainPostCloseError> {
    const SQL: &str = "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
        WHERE (lower(name) GLOB 'chain_post_close_*' OR lower(tbl_name) IN \
        ('chain_post_close_schema','chain_post_close_objects','chain_post_close_layouts', \
         'chain_post_close_layout_objects','chain_post_close_runs', \
         'chain_post_close_stage_begins','chain_post_close_stage_results', \
         'chain_post_close_concept_cache_writes', \
         'chain_post_close_cluster_configurations','chain_post_close_cluster_materials', \
         'chain_post_close_chain_daily_applications', \
         'chain_post_close_board_attempt_begins','chain_post_close_board_attempt_results', \
         'chain_post_close_board_kind_finals','chain_post_close_board_directory_materials', \
         'chain_post_close_board_selections','chain_post_close_board_status_materials', \
         'chain_post_close_board_error_materials', \
         'chain_post_close_concept_rpc_occurrences', \
         'chain_post_close_concept_rpc_attempt_begins', \
         'chain_post_close_concept_rpc_attempt_results', \
         'chain_post_close_concept_rpc_status_materials', \
         'chain_post_close_concept_rpc_error_materials', \
         'chain_post_close_concept_rpc_finals', \
         'chain_post_close_concept_rpc_legacy_outer_qualifications')) \
        AND NOT(type='index' AND name GLOB 'sqlite_autoindex_*' AND sql IS NULL) \
        ORDER BY name";
    let mut statement = connection.prepare(SQL).map_err(|_| storage("v7_catalog"))?;
    let definitions = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|_| storage("v7_catalog"))?
        .collect::<rusqlite::Result<_>>()
        .map_err(|_| storage("v7_catalog"))?;
    Ok(definitions)
}

#[derive(Clone)]
struct V7Bundle {
    digest: Sha256Digest,
    definitions: Vec<Definition>,
}
static V7_BUNDLE: OnceLock<V7Bundle> = OnceLock::new();

impl V7Bundle {
    fn verified() -> Result<Self, ChainPostCloseError> {
        cached_bundle(&V7_BUNDLE, Self::build)
    }

    fn build() -> Result<Self, ChainPostCloseError> {
        let digest = raw_digest(V7_DDL.as_bytes());
        if digest.as_str() != V7_DDL_SHA256 {
            return Err(ChainPostCloseError::BundleRejected);
        }
        let connection =
            Connection::open_in_memory().map_err(|_| ChainPostCloseError::BundleRejected)?;
        connection
            .execute_batch(DDL)
            .and_then(|_| connection.execute_batch(V2_DDL))
            .and_then(|_| connection.execute_batch(V3_DDL))
            .and_then(|_| connection.execute_batch(V4_DDL))
            .and_then(|_| {
                connection.execute_batch(
                    "CREATE TABLE data_acquisition_audit(id INTEGER PRIMARY KEY);\
                     CREATE TABLE data_acquisition_audit_chain(\
                         acquisition_audit_id INTEGER PRIMARY KEY);",
                )
            })
            .and_then(|_| connection.execute_batch(V5_DDL))
            .and_then(|_| connection.execute_batch(V6_DDL))
            .and_then(|_| connection.execute_batch(V7_DDL))
            .map_err(|_| ChainPostCloseError::BundleRejected)?;
        let definitions =
            v7_catalog(&connection).map_err(|_| ChainPostCloseError::BundleRejected)?;
        if definitions.len() != V7_OBJECT_COUNT {
            return Err(ChainPostCloseError::BundleRejected);
        }
        Ok(Self {
            digest,
            definitions,
        })
    }

    fn registered_definitions(&self) -> Vec<RegisteredDefinition> {
        self.definitions
            .iter()
            .map(|(name, kind, _, sql)| (name.clone(), kind.clone(), sql.clone()))
            .collect()
    }
}

fn v9_catalog(connection: &Connection) -> Result<Vec<Definition>, ChainPostCloseError> {
    const SQL: &str = "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
        WHERE (lower(name) GLOB 'chain_post_close_*' OR lower(tbl_name) IN \
        ('chain_post_close_schema','chain_post_close_objects','chain_post_close_layouts', \
         'chain_post_close_layout_objects','chain_post_close_runs', \
         'chain_post_close_stage_begins','chain_post_close_stage_results', \
         'chain_post_close_concept_cache_writes', \
         'chain_post_close_cluster_configurations','chain_post_close_cluster_materials', \
         'chain_post_close_chain_daily_applications', \
         'chain_post_close_board_attempt_begins','chain_post_close_board_attempt_results', \
         'chain_post_close_board_kind_finals','chain_post_close_board_directory_materials', \
         'chain_post_close_board_selections','chain_post_close_board_status_materials', \
         'chain_post_close_board_error_materials', \
         'chain_post_close_concept_rpc_occurrences', \
         'chain_post_close_concept_rpc_attempt_begins', \
         'chain_post_close_concept_rpc_attempt_results', \
         'chain_post_close_concept_rpc_status_materials', \
         'chain_post_close_concept_rpc_error_materials', \
         'chain_post_close_concept_rpc_finals', \
         'chain_post_close_concept_rpc_legacy_outer_qualifications', \
         'chain_post_close_position_materials', \
         'chain_post_close_position_concept_materials', \
         'chain_post_close_position_concept_rpc_occurrences', \
         'chain_post_close_position_concept_rpc_attempt_begins', \
         'chain_post_close_position_concept_rpc_attempt_results', \
         'chain_post_close_position_concept_rpc_status_materials', \
         'chain_post_close_position_concept_rpc_error_materials', \
         'chain_post_close_position_concept_rpc_finals', \
         'chain_post_close_position_concept_cache_writes')) \
        AND NOT(type='index' AND name GLOB 'sqlite_autoindex_*' AND sql IS NULL) \
        ORDER BY name";
    let mut statement = connection.prepare(SQL).map_err(|_| storage("v9_catalog"))?;
    let definitions = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|_| storage("v9_catalog"))?
        .collect::<rusqlite::Result<_>>()
        .map_err(|_| storage("v9_catalog"))?;
    Ok(definitions)
}

#[derive(Clone)]
struct V9Bundle {
    digest: Sha256Digest,
    definitions: Vec<Definition>,
}

static V9_BUNDLE: OnceLock<V9Bundle> = OnceLock::new();

impl V9Bundle {
    fn verified() -> Result<Self, ChainPostCloseError> {
        cached_bundle(&V9_BUNDLE, Self::build)
    }

    fn build() -> Result<Self, ChainPostCloseError> {
        let digest = raw_digest(V9_DDL.as_bytes());
        if digest.as_str() != V9_DDL_SHA256 {
            return Err(ChainPostCloseError::BundleRejected);
        }
        let connection =
            Connection::open_in_memory().map_err(|_| ChainPostCloseError::BundleRejected)?;
        connection
            .execute_batch(DDL)
            .and_then(|_| connection.execute_batch(V2_DDL))
            .and_then(|_| connection.execute_batch(V3_DDL))
            .and_then(|_| connection.execute_batch(V4_DDL))
            .and_then(|_| {
                connection.execute_batch(
                    "CREATE TABLE data_acquisition_audit(id INTEGER PRIMARY KEY);\
                     CREATE TABLE data_acquisition_audit_chain(\
                         acquisition_audit_id INTEGER PRIMARY KEY);",
                )
            })
            .and_then(|_| connection.execute_batch(V5_DDL))
            .and_then(|_| connection.execute_batch(V6_DDL))
            .and_then(|_| connection.execute_batch(V7_DDL))
            .and_then(|_| connection.execute_batch(V8_DDL))
            .and_then(|_| connection.execute_batch(V9_DDL))
            .map_err(|_| ChainPostCloseError::BundleRejected)?;
        let definitions =
            v9_catalog(&connection).map_err(|_| ChainPostCloseError::BundleRejected)?;
        if definitions.len() != V9_OBJECT_COUNT {
            return Err(ChainPostCloseError::BundleRejected);
        }
        Ok(Self {
            digest,
            definitions,
        })
    }

    fn registered_definitions(&self) -> Vec<RegisteredDefinition> {
        self.definitions
            .iter()
            .map(|(name, kind, _, sql)| (name.clone(), kind.clone(), sql.clone()))
            .collect()
    }
}

fn v10_catalog(connection: &Connection) -> Result<Vec<Definition>, ChainPostCloseError> {
    catalog(connection)
}

#[derive(Clone)]
struct V10Bundle {
    digest: Sha256Digest,
    definitions: Vec<Definition>,
}

static V10_BUNDLE: OnceLock<V10Bundle> = OnceLock::new();

impl V10Bundle {
    fn verified() -> Result<Self, ChainPostCloseError> {
        cached_bundle(&V10_BUNDLE, Self::build)
    }

    fn build() -> Result<Self, ChainPostCloseError> {
        let digest = raw_digest(V10_DDL.as_bytes());
        if digest.as_str() != V10_DDL_SHA256 {
            return Err(ChainPostCloseError::BundleRejected);
        }
        let connection =
            Connection::open_in_memory().map_err(|_| ChainPostCloseError::BundleRejected)?;
        connection
            .execute_batch(DDL)
            .and_then(|_| connection.execute_batch(V2_DDL))
            .and_then(|_| connection.execute_batch(V3_DDL))
            .and_then(|_| connection.execute_batch(V4_DDL))
            .and_then(|_| install_v10_bundle_audit_stubs(&connection))
            .and_then(|_| connection.execute_batch(V5_DDL))
            .and_then(|_| connection.execute_batch(V6_DDL))
            .and_then(|_| connection.execute_batch(V7_DDL))
            .and_then(|_| connection.execute_batch(V8_DDL))
            .and_then(|_| connection.execute_batch(V9_DDL))
            .and_then(|_| connection.execute_batch(V10_DDL))
            .map_err(|_| ChainPostCloseError::BundleRejected)?;
        let definitions =
            v10_catalog(&connection).map_err(|_| ChainPostCloseError::BundleRejected)?;
        if definitions.len() != V10_OBJECT_COUNT {
            return Err(ChainPostCloseError::BundleRejected);
        }
        Ok(Self {
            digest,
            definitions,
        })
    }

    fn registered_definitions(&self) -> Vec<RegisteredDefinition> {
        self.definitions
            .iter()
            .map(|(name, kind, _, sql)| (name.clone(), kind.clone(), sql.clone()))
            .collect()
    }
}

fn install_v10_bundle_audit_stubs(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE data_acquisition_audit(\
         id INTEGER PRIMARY KEY,schema_version INTEGER,capability TEXT,provider TEXT,source TEXT,\
         request_hash TEXT,source_at TEXT,observed_at TEXT,batch_id TEXT,outcome TEXT,\
         request_count INTEGER,accepted_count INTEGER,rejected_count INTEGER,reason_code TEXT,retryable INTEGER);\
         CREATE TABLE data_acquisition_audit_chain(acquisition_audit_id INTEGER PRIMARY KEY,record_hash TEXT);"
    )
}

fn v8_catalog(connection: &Connection) -> Result<Vec<Definition>, ChainPostCloseError> {
    const SQL: &str = "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
        WHERE (lower(name) GLOB 'chain_post_close_*' OR lower(tbl_name) IN \
        ('chain_post_close_schema','chain_post_close_objects','chain_post_close_layouts', \
         'chain_post_close_layout_objects','chain_post_close_runs', \
         'chain_post_close_stage_begins','chain_post_close_stage_results', \
         'chain_post_close_concept_cache_writes', \
         'chain_post_close_cluster_configurations','chain_post_close_cluster_materials', \
         'chain_post_close_chain_daily_applications', \
         'chain_post_close_board_attempt_begins','chain_post_close_board_attempt_results', \
         'chain_post_close_board_kind_finals','chain_post_close_board_directory_materials', \
         'chain_post_close_board_selections','chain_post_close_board_status_materials', \
         'chain_post_close_board_error_materials', \
         'chain_post_close_concept_rpc_occurrences', \
         'chain_post_close_concept_rpc_attempt_begins', \
         'chain_post_close_concept_rpc_attempt_results', \
         'chain_post_close_concept_rpc_status_materials', \
         'chain_post_close_concept_rpc_error_materials', \
         'chain_post_close_concept_rpc_finals', \
         'chain_post_close_concept_rpc_legacy_outer_qualifications', \
         'chain_post_close_position_materials', \
         'chain_post_close_position_concept_materials')) \
        AND NOT(type='index' AND name GLOB 'sqlite_autoindex_*' AND sql IS NULL) \
        ORDER BY name";
    let mut statement = connection.prepare(SQL).map_err(|_| storage("v8_catalog"))?;
    let definitions = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|_| storage("v8_catalog"))?
        .collect::<rusqlite::Result<_>>()
        .map_err(|_| storage("v8_catalog"))?;
    Ok(definitions)
}

#[derive(Clone)]
struct V8Bundle {
    digest: Sha256Digest,
    definitions: Vec<Definition>,
}

static V8_BUNDLE: OnceLock<V8Bundle> = OnceLock::new();

impl V8Bundle {
    fn verified() -> Result<Self, ChainPostCloseError> {
        cached_bundle(&V8_BUNDLE, Self::build)
    }

    fn build() -> Result<Self, ChainPostCloseError> {
        let digest = raw_digest(V8_DDL.as_bytes());
        if digest.as_str() != V8_DDL_SHA256 {
            return Err(ChainPostCloseError::BundleRejected);
        }
        let connection =
            Connection::open_in_memory().map_err(|_| ChainPostCloseError::BundleRejected)?;
        connection
            .execute_batch(DDL)
            .and_then(|_| connection.execute_batch(V2_DDL))
            .and_then(|_| connection.execute_batch(V3_DDL))
            .and_then(|_| connection.execute_batch(V4_DDL))
            .and_then(|_| {
                connection.execute_batch(
                    "CREATE TABLE data_acquisition_audit(id INTEGER PRIMARY KEY);\
                     CREATE TABLE data_acquisition_audit_chain(\
                         acquisition_audit_id INTEGER PRIMARY KEY);",
                )
            })
            .and_then(|_| connection.execute_batch(V5_DDL))
            .and_then(|_| connection.execute_batch(V6_DDL))
            .and_then(|_| connection.execute_batch(V7_DDL))
            .and_then(|_| connection.execute_batch(V8_DDL))
            .map_err(|_| ChainPostCloseError::BundleRejected)?;
        let definitions =
            v8_catalog(&connection).map_err(|_| ChainPostCloseError::BundleRejected)?;
        if definitions.len() != V8_OBJECT_COUNT {
            return Err(ChainPostCloseError::BundleRejected);
        }
        Ok(Self {
            digest,
            definitions,
        })
    }

    fn registered_definitions(&self) -> Vec<RegisteredDefinition> {
        self.definitions
            .iter()
            .map(|(name, kind, _, sql)| (name.clone(), kind.clone(), sql.clone()))
            .collect()
    }
}

impl V6Bundle {
    fn verified() -> Result<Self, ChainPostCloseError> {
        cached_bundle(&V6_BUNDLE, Self::build)
    }

    fn build() -> Result<Self, ChainPostCloseError> {
        let digest = raw_digest(V6_DDL.as_bytes());
        if digest.as_str() != V6_DDL_SHA256 {
            return Err(ChainPostCloseError::BundleRejected);
        }
        let connection =
            Connection::open_in_memory().map_err(|_| ChainPostCloseError::BundleRejected)?;
        connection
            .execute_batch(DDL)
            .and_then(|_| connection.execute_batch(V2_DDL))
            .and_then(|_| connection.execute_batch(V3_DDL))
            .and_then(|_| connection.execute_batch(V4_DDL))
            .and_then(|_| {
                connection.execute_batch(
                    "CREATE TABLE data_acquisition_audit(id INTEGER PRIMARY KEY);\
                     CREATE TABLE data_acquisition_audit_chain(\
                         acquisition_audit_id INTEGER PRIMARY KEY);",
                )
            })
            .and_then(|_| connection.execute_batch(V5_DDL))
            .and_then(|_| connection.execute_batch(V6_DDL))
            .map_err(|_| ChainPostCloseError::BundleRejected)?;
        let definitions =
            v6_catalog(&connection).map_err(|_| ChainPostCloseError::BundleRejected)?;
        if definitions.len() != V6_OBJECT_COUNT {
            return Err(ChainPostCloseError::BundleRejected);
        }
        Ok(Self {
            digest,
            definitions,
        })
    }

    fn registered_definitions(&self) -> Vec<RegisteredDefinition> {
        self.definitions
            .iter()
            .map(|(name, kind, _, sql)| (name.clone(), kind.clone(), sql.clone()))
            .collect()
    }
}

impl V5Bundle {
    fn verified() -> Result<Self, ChainPostCloseError> {
        cached_bundle(&V5_BUNDLE, Self::build)
    }

    fn build() -> Result<Self, ChainPostCloseError> {
        let digest = raw_digest(V5_DDL.as_bytes());
        if digest.as_str() != V5_DDL_SHA256 {
            return Err(ChainPostCloseError::BundleRejected);
        }
        let connection =
            Connection::open_in_memory().map_err(|_| ChainPostCloseError::BundleRejected)?;
        connection
            .execute_batch(DDL)
            .and_then(|_| connection.execute_batch(V2_DDL))
            .and_then(|_| connection.execute_batch(V3_DDL))
            .and_then(|_| connection.execute_batch(V4_DDL))
            .and_then(|_| {
                connection.execute_batch(
                    "CREATE TABLE data_acquisition_audit(id INTEGER PRIMARY KEY);\
                     CREATE TABLE data_acquisition_audit_chain(\
                         acquisition_audit_id INTEGER PRIMARY KEY);",
                )
            })
            .and_then(|_| connection.execute_batch(V5_DDL))
            .map_err(|_| ChainPostCloseError::BundleRejected)?;
        let definitions =
            v5_catalog(&connection).map_err(|_| ChainPostCloseError::BundleRejected)?;
        if definitions.len() != V5_OBJECT_COUNT {
            return Err(ChainPostCloseError::BundleRejected);
        }
        Ok(Self {
            digest,
            definitions,
        })
    }

    fn registered_definitions(&self) -> Vec<RegisteredDefinition> {
        self.definitions
            .iter()
            .map(|(name, kind, _, sql)| (name.clone(), kind.clone(), sql.clone()))
            .collect()
    }
}

impl V4Bundle {
    fn verified() -> Result<Self, ChainPostCloseError> {
        cached_bundle(&V4_BUNDLE, Self::build)
    }

    fn build() -> Result<Self, ChainPostCloseError> {
        let digest = raw_digest(V4_DDL.as_bytes());
        if digest.as_str() != V4_DDL_SHA256 {
            return Err(ChainPostCloseError::BundleRejected);
        }
        let connection =
            Connection::open_in_memory().map_err(|_| ChainPostCloseError::BundleRejected)?;
        connection
            .execute_batch(DDL)
            .and_then(|_| connection.execute_batch(V2_DDL))
            .and_then(|_| connection.execute_batch(V3_DDL))
            .and_then(|_| connection.execute_batch(V4_DDL))
            .map_err(|_| ChainPostCloseError::BundleRejected)?;
        let definitions =
            v4_catalog(&connection).map_err(|_| ChainPostCloseError::BundleRejected)?;
        if definitions.len() != V4_OBJECT_COUNT {
            return Err(ChainPostCloseError::BundleRejected);
        }
        Ok(Self {
            digest,
            definitions,
        })
    }

    fn registered_definitions(&self) -> Vec<RegisteredDefinition> {
        self.definitions
            .iter()
            .map(|(name, kind, _, sql)| (name.clone(), kind.clone(), sql.clone()))
            .collect()
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum V2Operation {
    Migrate,
    Verify,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum V3Operation {
    Migrate,
    Verify,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum V4Operation {
    Migrate,
    Verify,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum V5Operation {
    Migrate,
    Verify,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum V6Operation {
    Migrate,
    Verify,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum V7Operation {
    Migrate,
    Verify,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum V8Operation {
    Migrate,
    Verify,
}

fn run_v2(
    connection: &mut Connection,
    operation: V2Operation,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let v1 = Bundle::verified()?;
    let v2 = V2Bundle::verified()?;
    if !connection.is_autocommit() {
        return Err(ChainPostCloseError::ConnectionSafeguardFailed);
    }
    let original_query_only = pragma(connection, "PRAGMA query_only")?;
    if !matches!(original_query_only, 0 | 1)
        || (operation == V2Operation::Migrate && original_query_only != 0)
    {
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
    verify_safeguards(connection, original_query_only)?;

    let behavior = if operation == V2Operation::Migrate {
        TransactionBehavior::Immediate
    } else {
        TransactionBehavior::Deferred
    };
    let transaction = connection
        .transaction_with_behavior(behavior)
        .map_err(|_| storage("begin"))?;
    let result = (|| {
        attest_bundled_connection(&transaction).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        verify_safeguards(&transaction, 1)?;
        if operation == V2Operation::Migrate {
            restore_query_only(&transaction, original_query_only)?;
            verify_v1_metadata(&transaction, &v1)?;
            if v2_catalog(&transaction)? != v1.definitions {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            transaction
                .execute_batch(V2_DDL)
                .map_err(|_| storage("v2_ddl"))?;
            for (name, kind, _, definition) in &v2.definitions {
                let definition = std::str::from_utf8(definition)
                    .map_err(|_| ChainPostCloseError::BundleRejected)?;
                transaction
                    .execute(
                        "INSERT INTO chain_post_close_layout_objects \
                         (layout_version,name,object_type,definition) VALUES(2,?1,?2,?3)",
                        params![name, kind, definition],
                    )
                    .map_err(|_| storage("v2_registry"))?;
            }
            transaction
                .execute(
                    "INSERT INTO chain_post_close_layouts \
                     (layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                      artifact_codec_version,input_codec_version,stage_codec_version, \
                      description,bundle_sha256) \
                     VALUES(2,1,?1,1,1,1,'chain-post-close-layout-v2',?2)",
                    params![DDL_SHA256, v2.digest.as_str()],
                )
                .map_err(|_| storage("v2_seal"))?;
        }
        verify_v2_installed(&transaction, &v1, &v2)
    })();
    let result = match result {
        Ok(receipt) => transaction
            .commit()
            .map(|()| receipt)
            .map_err(|_| storage("commit")),
        Err(error) => match transaction.rollback() {
            Ok(()) => Err(error),
            Err(_) => Err(storage("rollback")),
        },
    };
    if !connection.is_autocommit() {
        let rollback = connection.execute_batch("ROLLBACK;");
        if rollback.is_err() || !connection.is_autocommit() {
            restore_query_only(connection, 1)?;
            return Err(storage("transaction_cleanup"));
        }
    }
    restore_query_only(connection, original_query_only)?;
    result
}

fn run_v3(
    connection: &mut Connection,
    operation: V3Operation,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let v1 = Bundle::verified()?;
    let v2 = V2Bundle::verified()?;
    let v3 = V3Bundle::verified()?;
    if !connection.is_autocommit() {
        return Err(ChainPostCloseError::ConnectionSafeguardFailed);
    }
    let original_query_only = pragma(connection, "PRAGMA query_only")?;
    if !matches!(original_query_only, 0 | 1)
        || (operation == V3Operation::Migrate && original_query_only != 0)
    {
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
    verify_safeguards(connection, original_query_only)?;

    let behavior = if operation == V3Operation::Migrate {
        TransactionBehavior::Immediate
    } else {
        TransactionBehavior::Deferred
    };
    let transaction = connection
        .transaction_with_behavior(behavior)
        .map_err(|_| storage("begin"))?;
    let result = (|| {
        attest_bundled_connection(&transaction).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        verify_safeguards(&transaction, 1)?;
        if operation == V3Operation::Migrate {
            restore_query_only(&transaction, original_query_only)?;
            verify_v2_installed(&transaction, &v1, &v2)?;
            transaction
                .execute_batch(V3_DDL)
                .map_err(|_| storage("v3_ddl"))?;
            for (name, kind, _, definition) in &v3.definitions {
                let definition = std::str::from_utf8(definition)
                    .map_err(|_| ChainPostCloseError::BundleRejected)?;
                transaction
                    .execute(
                        "INSERT INTO chain_post_close_layout_objects \
                         (layout_version,name,object_type,definition) VALUES(3,?1,?2,?3)",
                        params![name, kind, definition],
                    )
                    .map_err(|_| storage("v3_registry"))?;
            }
            transaction
                .execute(
                    "INSERT INTO chain_post_close_layouts \
                     (layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                      artifact_codec_version,input_codec_version,stage_codec_version, \
                      description,bundle_sha256) \
                     VALUES(3,2,?1,1,1,1,'chain-post-close-layout-v3',?2)",
                    params![v2.digest.as_str(), v3.digest.as_str()],
                )
                .map_err(|_| storage("v3_seal"))?;
        }
        verify_v3_installed(&transaction, &v1, &v2, &v3)
    })();
    let result = match result {
        Ok(receipt) => transaction
            .commit()
            .map(|()| receipt)
            .map_err(|_| storage("commit")),
        Err(error) => match transaction.rollback() {
            Ok(()) => Err(error),
            Err(_) => Err(storage("rollback")),
        },
    };
    if !connection.is_autocommit() {
        let rollback = connection.execute_batch("ROLLBACK;");
        if rollback.is_err() || !connection.is_autocommit() {
            restore_query_only(connection, 1)?;
            return Err(storage("transaction_cleanup"));
        }
    }
    restore_query_only(connection, original_query_only)?;
    result
}

fn run_v4(
    connection: &mut Connection,
    operation: V4Operation,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let v1 = Bundle::verified()?;
    let v2 = V2Bundle::verified()?;
    let v3 = V3Bundle::verified()?;
    let v4 = V4Bundle::verified()?;
    if !connection.is_autocommit() {
        return Err(ChainPostCloseError::ConnectionSafeguardFailed);
    }
    let original_query_only = pragma(connection, "PRAGMA query_only")?;
    if !matches!(original_query_only, 0 | 1)
        || (operation == V4Operation::Migrate && original_query_only != 0)
    {
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
    verify_safeguards(connection, original_query_only)?;

    let behavior = if operation == V4Operation::Migrate {
        TransactionBehavior::Immediate
    } else {
        TransactionBehavior::Deferred
    };
    let transaction = connection
        .transaction_with_behavior(behavior)
        .map_err(|_| storage("begin"))?;
    let result = (|| {
        attest_bundled_connection(&transaction).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        verify_safeguards(&transaction, 1)?;
        if operation == V4Operation::Migrate {
            restore_query_only(&transaction, original_query_only)?;
            verify_v3_installed(&transaction, &v1, &v2, &v3)?;
            transaction
                .execute_batch(V4_DDL)
                .map_err(|_| storage("v4_ddl"))?;
            for (name, kind, _, definition) in &v4.definitions {
                let definition = std::str::from_utf8(definition)
                    .map_err(|_| ChainPostCloseError::BundleRejected)?;
                transaction
                    .execute(
                        "INSERT INTO chain_post_close_layout_objects \
                         (layout_version,name,object_type,definition) VALUES(4,?1,?2,?3)",
                        params![name, kind, definition],
                    )
                    .map_err(|_| storage("v4_registry"))?;
            }
            transaction
                .execute(
                    "INSERT INTO chain_post_close_layouts \
                     (layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                      artifact_codec_version,input_codec_version,stage_codec_version, \
                      description,bundle_sha256) \
                     VALUES(4,3,?1,1,1,1,'chain-post-close-layout-v4',?2)",
                    params![v3.digest.as_str(), v4.digest.as_str()],
                )
                .map_err(|_| storage("v4_seal"))?;
        }
        verify_v4_installed(&transaction, &v1, &v2, &v3, &v4)
    })();
    let result = match result {
        Ok(receipt) => transaction
            .commit()
            .map(|()| receipt)
            .map_err(|_| storage("commit")),
        Err(error) => match transaction.rollback() {
            Ok(()) => Err(error),
            Err(_) => Err(storage("rollback")),
        },
    };
    if !connection.is_autocommit() {
        let rollback = connection.execute_batch("ROLLBACK;");
        if rollback.is_err() || !connection.is_autocommit() {
            restore_query_only(connection, 1)?;
            return Err(storage("transaction_cleanup"));
        }
    }
    restore_query_only(connection, original_query_only)?;
    result
}

fn run_v5(
    connection: &mut Connection,
    operation: V5Operation,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let v1 = Bundle::verified()?;
    let v2 = V2Bundle::verified()?;
    let v3 = V3Bundle::verified()?;
    let v4 = V4Bundle::verified()?;
    let v5 = V5Bundle::verified()?;
    if !connection.is_autocommit() {
        return Err(ChainPostCloseError::ConnectionSafeguardFailed);
    }
    let original_query_only = pragma(connection, "PRAGMA query_only")?;
    if !matches!(original_query_only, 0 | 1)
        || (operation == V5Operation::Migrate && original_query_only != 0)
    {
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
    verify_safeguards(connection, original_query_only)?;

    let behavior = if operation == V5Operation::Migrate {
        TransactionBehavior::Immediate
    } else {
        TransactionBehavior::Deferred
    };
    let transaction = connection
        .transaction_with_behavior(behavior)
        .map_err(|_| storage("begin"))?;
    let result = (|| {
        attest_bundled_connection(&transaction).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        verify_safeguards(&transaction, 1)?;
        if operation == V5Operation::Migrate {
            restore_query_only(&transaction, original_query_only)?;
            verify_v4_installed(&transaction, &v1, &v2, &v3, &v4)?;
            transaction
                .execute_batch(V5_DDL)
                .map_err(|_| storage("v5_ddl"))?;
            for (name, kind, _, definition) in &v5.definitions {
                let definition = std::str::from_utf8(definition)
                    .map_err(|_| ChainPostCloseError::BundleRejected)?;
                transaction
                    .execute(
                        "INSERT INTO chain_post_close_layout_objects \
                         (layout_version,name,object_type,definition) VALUES(5,?1,?2,?3)",
                        params![name, kind, definition],
                    )
                    .map_err(|_| storage("v5_registry"))?;
            }
            transaction
                .execute(
                    "INSERT INTO chain_post_close_layouts \
                     (layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                      artifact_codec_version,input_codec_version,stage_codec_version, \
                      description,bundle_sha256) \
                     VALUES(5,4,?1,1,1,1,'chain-post-close-layout-v5',?2)",
                    params![v4.digest.as_str(), v5.digest.as_str()],
                )
                .map_err(|_| storage("v5_seal"))?;
        }
        verify_v5_installed(&transaction, &v1, &v2, &v3, &v4, &v5)
    })();
    let result = match result {
        Ok(receipt) => transaction
            .commit()
            .map(|()| receipt)
            .map_err(|_| storage("commit")),
        Err(error) => match transaction.rollback() {
            Ok(()) => Err(error),
            Err(_) => Err(storage("rollback")),
        },
    };
    if !connection.is_autocommit() {
        let rollback = connection.execute_batch("ROLLBACK;");
        if rollback.is_err() || !connection.is_autocommit() {
            restore_query_only(connection, 1)?;
            return Err(storage("transaction_cleanup"));
        }
    }
    restore_query_only(connection, original_query_only)?;
    result
}

fn run_v6(
    connection: &mut Connection,
    operation: V6Operation,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let v1 = Bundle::verified()?;
    let v2 = V2Bundle::verified()?;
    let v3 = V3Bundle::verified()?;
    let v4 = V4Bundle::verified()?;
    let v5 = V5Bundle::verified()?;
    let v6 = V6Bundle::verified()?;
    if !connection.is_autocommit() {
        return Err(ChainPostCloseError::ConnectionSafeguardFailed);
    }
    let original_query_only = pragma(connection, "PRAGMA query_only")?;
    if !matches!(original_query_only, 0 | 1)
        || (operation == V6Operation::Migrate && original_query_only != 0)
    {
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
    verify_safeguards(connection, original_query_only)?;

    let behavior = if operation == V6Operation::Migrate {
        TransactionBehavior::Immediate
    } else {
        TransactionBehavior::Deferred
    };
    let transaction = connection
        .transaction_with_behavior(behavior)
        .map_err(|_| storage("begin"))?;
    let result = (|| {
        attest_bundled_connection(&transaction).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        verify_safeguards(&transaction, 1)?;
        if operation == V6Operation::Migrate {
            restore_query_only(&transaction, original_query_only)?;
            verify_v5_installed(&transaction, &v1, &v2, &v3, &v4, &v5)?;
            transaction
                .execute_batch(V6_DDL)
                .map_err(|_| storage("v6_ddl"))?;
            transaction
                .execute(
                    "INSERT INTO chain_post_close_board_status_materials( \
                     intent_id,kind,attempt_ordinal,result_run_version,result_sha256, \
                     request_sha256,provenance,legacy_layout_version) \
                     SELECT intent_id,kind,attempt_ordinal,run_version,result_sha256, \
                            request_sha256,'LegacyV5Absent',6 \
                     FROM chain_post_close_board_attempt_results WHERE wire_outcome='Status'",
                    [],
                )
                .map_err(|_| storage("v6_legacy_status"))?;
            for (name, kind, _, definition) in &v6.definitions {
                let definition = std::str::from_utf8(definition)
                    .map_err(|_| ChainPostCloseError::BundleRejected)?;
                transaction
                    .execute(
                        "INSERT INTO chain_post_close_layout_objects \
                         (layout_version,name,object_type,definition) VALUES(6,?1,?2,?3)",
                        params![name, kind, definition],
                    )
                    .map_err(|_| storage("v6_registry"))?;
            }
            transaction
                .execute(
                    "INSERT INTO chain_post_close_layouts \
                     (layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                      artifact_codec_version,input_codec_version,stage_codec_version, \
                      description,bundle_sha256) \
                     VALUES(6,5,?1,1,1,1,'chain-post-close-layout-v6',?2)",
                    params![v5.digest.as_str(), v6.digest.as_str()],
                )
                .map_err(|_| storage("v6_seal"))?;
        }
        verify_v6_installed(&transaction, &v1, &v2, &v3, &v4, &v5, &v6)
    })();
    let result = match result {
        Ok(receipt) => transaction
            .commit()
            .map(|()| receipt)
            .map_err(|_| storage("commit")),
        Err(error) => match transaction.rollback() {
            Ok(()) => Err(error),
            Err(_) => Err(storage("rollback")),
        },
    };
    if !connection.is_autocommit() {
        let rollback = connection.execute_batch("ROLLBACK;");
        if rollback.is_err() || !connection.is_autocommit() {
            restore_query_only(connection, 1)?;
            return Err(storage("transaction_cleanup"));
        }
    }
    restore_query_only(connection, original_query_only)?;
    result
}

fn run_v7(
    connection: &mut Connection,
    operation: V7Operation,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let v1 = Bundle::verified()?;
    let v2 = V2Bundle::verified()?;
    let v3 = V3Bundle::verified()?;
    let v4 = V4Bundle::verified()?;
    let v5 = V5Bundle::verified()?;
    let v6 = V6Bundle::verified()?;
    let v7 = V7Bundle::verified()?;
    if !connection.is_autocommit() {
        return Err(ChainPostCloseError::ConnectionSafeguardFailed);
    }
    let original_query_only = pragma(connection, "PRAGMA query_only")?;
    if !matches!(original_query_only, 0 | 1)
        || (operation == V7Operation::Migrate && original_query_only != 0)
    {
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
    verify_safeguards(connection, original_query_only)?;

    let behavior = if operation == V7Operation::Migrate {
        TransactionBehavior::Immediate
    } else {
        TransactionBehavior::Deferred
    };
    let transaction = connection
        .transaction_with_behavior(behavior)
        .map_err(|_| storage("begin"))?;
    let result = (|| {
        attest_bundled_connection(&transaction).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        verify_safeguards(&transaction, 1)?;
        if operation == V7Operation::Migrate {
            restore_query_only(&transaction, original_query_only)?;
            verify_v6_installed(&transaction, &v1, &v2, &v3, &v4, &v5, &v6)?;
            super::validate_migration_audit_facts(&transaction)?;
            super::validate_all_runs_at_layout(&transaction, 6)?;
            let legacy_qualifications = snapshot_legacy_qualifications(&transaction)?;
            transaction
                .execute_batch(V7_DDL)
                .map_err(|_| storage("v7_ddl"))?;
            install_legacy_qualifications(&transaction, &legacy_qualifications)?;
            if read_legacy_qualifications(&transaction)? != legacy_qualifications {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            for (name, kind, _, definition) in &v7.definitions {
                let definition = std::str::from_utf8(definition)
                    .map_err(|_| ChainPostCloseError::BundleRejected)?;
                transaction
                    .execute(
                        "INSERT INTO chain_post_close_layout_objects \
                         (layout_version,name,object_type,definition) VALUES(7,?1,?2,?3)",
                        params![name, kind, definition],
                    )
                    .map_err(|_| storage("v7_registry"))?;
            }
            transaction
                .execute(
                    "INSERT INTO chain_post_close_layouts \
                     (layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                      artifact_codec_version,input_codec_version,stage_codec_version, \
                      description,bundle_sha256) \
                     VALUES(7,6,?1,1,1,1,'chain-post-close-layout-v7',?2)",
                    params![v6.digest.as_str(), v7.digest.as_str()],
                )
                .map_err(|_| storage("v7_seal"))?;
        }
        let receipt = verify_v7_installed(&transaction, &v1, &v2, &v3, &v4, &v5, &v6, &v7)?;
        if operation == V7Operation::Migrate {
            super::validate_all_runs_at_layout(&transaction, 7)?;
        }
        Ok(receipt)
    })();
    let result = match result {
        Ok(receipt) => transaction
            .commit()
            .map(|()| receipt)
            .map_err(|_| storage("commit")),
        Err(error) => match transaction.rollback() {
            Ok(()) => Err(error),
            Err(_) => Err(storage("rollback")),
        },
    };
    if !connection.is_autocommit() {
        let rollback = connection.execute_batch("ROLLBACK;");
        if rollback.is_err() || !connection.is_autocommit() {
            restore_query_only(connection, 1)?;
            return Err(storage("transaction_cleanup"));
        }
    }
    restore_query_only(connection, original_query_only)?;
    result
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum V9Operation {
    Verify,
    Migrate,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum V10Operation {
    Verify,
    Migrate,
}

fn run_v10(
    connection: &mut Connection,
    operation: V10Operation,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
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
    if !connection.is_autocommit() {
        return Err(ChainPostCloseError::ConnectionSafeguardFailed);
    }
    let original_query_only = pragma(connection, "PRAGMA query_only")?;
    if !matches!(original_query_only, 0 | 1)
        || (operation == V10Operation::Migrate && original_query_only != 0)
    {
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
    verify_safeguards(connection, original_query_only)?;
    let behavior = if operation == V10Operation::Migrate {
        TransactionBehavior::Immediate
    } else {
        TransactionBehavior::Deferred
    };
    let transaction = connection
        .transaction_with_behavior(behavior)
        .map_err(|_| storage("begin"))?;
    let result = (|| {
        attest_bundled_connection(&transaction).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        verify_safeguards(&transaction, 1)?;
        if operation == V10Operation::Migrate {
            restore_query_only(&transaction, original_query_only)?;
            verify_v9_installed(&transaction, &v1, &v2, &v3, &v4, &v5, &v6, &v7, &v8, &v9)?;
            super::validate_migration_audit_facts(&transaction)?;
            super::validate_all_runs_at_layout(&transaction, 9)?;
            transaction
                .execute_batch(V10_DDL)
                .map_err(|_| storage("v10_ddl"))?;
            super::validate_all_runs_at_layout(&transaction, 10)?;
            for (name, kind, _, definition) in &v10.definitions {
                let definition = std::str::from_utf8(definition)
                    .map_err(|_| ChainPostCloseError::BundleRejected)?;
                transaction
                    .execute(
                        "INSERT INTO chain_post_close_layout_objects \
                     (layout_version,name,object_type,definition) VALUES(10,?1,?2,?3)",
                        params![name, kind, definition],
                    )
                    .map_err(|_| storage("v10_registry"))?;
            }
            transaction.execute(
                "INSERT INTO chain_post_close_layouts \
                 (layout_version,predecessor_layout_version,predecessor_bundle_sha256,\
                  artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256) \
                 VALUES(10,9,?1,1,1,1,'chain-post-close-layout-v10',?2)",
                params![v9.digest.as_str(),v10.digest.as_str()],
            ).map_err(|_| storage("v10_seal"))?;
        }
        let receipt = verify_v10_installed(
            &transaction,
            &v1,
            &v2,
            &v3,
            &v4,
            &v5,
            &v6,
            &v7,
            &v8,
            &v9,
            &v10,
        )?;
        super::validate_migration_audit_facts(&transaction)?;
        super::validate_all_runs_at_layout(&transaction, 10)?;
        Ok(receipt)
    })();
    let result = match result {
        Ok(receipt) => transaction
            .commit()
            .map(|()| receipt)
            .map_err(|_| storage("commit")),
        Err(error) => match transaction.rollback() {
            Ok(()) => Err(error),
            Err(_) => Err(storage("rollback")),
        },
    };
    if !connection.is_autocommit() {
        let rollback = connection.execute_batch("ROLLBACK;");
        if rollback.is_err() || !connection.is_autocommit() {
            restore_query_only(connection, 1)?;
            return Err(storage("transaction_cleanup"));
        }
    }
    restore_query_only(connection, original_query_only)?;
    result
}

fn run_v9(
    connection: &mut Connection,
    operation: V9Operation,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let v1 = Bundle::verified()?;
    let v2 = V2Bundle::verified()?;
    let v3 = V3Bundle::verified()?;
    let v4 = V4Bundle::verified()?;
    let v5 = V5Bundle::verified()?;
    let v6 = V6Bundle::verified()?;
    let v7 = V7Bundle::verified()?;
    let v8 = V8Bundle::verified()?;
    let v9 = V9Bundle::verified()?;
    if !connection.is_autocommit() {
        return Err(ChainPostCloseError::ConnectionSafeguardFailed);
    }
    let original_query_only = pragma(connection, "PRAGMA query_only")?;
    if !matches!(original_query_only, 0 | 1)
        || (operation == V9Operation::Migrate && original_query_only != 0)
    {
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
    verify_safeguards(connection, original_query_only)?;

    let behavior = if operation == V9Operation::Migrate {
        TransactionBehavior::Immediate
    } else {
        TransactionBehavior::Deferred
    };
    let transaction = connection
        .transaction_with_behavior(behavior)
        .map_err(|_| storage("begin"))?;
    let result = (|| {
        attest_bundled_connection(&transaction).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        verify_safeguards(&transaction, 1)?;
        if operation == V9Operation::Migrate {
            restore_query_only(&transaction, original_query_only)?;
            verify_v8_installed(&transaction, &v1, &v2, &v3, &v4, &v5, &v6, &v7, &v8)?;
            super::validate_migration_audit_facts(&transaction)?;
            super::validate_all_runs_at_layout(&transaction, 8)?;
            transaction
                .execute_batch(V9_DDL)
                .map_err(|_| storage("v9_ddl"))?;
            // Validate before registry rows create their deferred reference to the new seal.
            super::validate_all_runs_at_layout(&transaction, 9)?;
            for (name, kind, _, definition) in &v9.definitions {
                let definition = std::str::from_utf8(definition)
                    .map_err(|_| ChainPostCloseError::BundleRejected)?;
                transaction
                    .execute(
                        "INSERT INTO chain_post_close_layout_objects \
                         (layout_version,name,object_type,definition) VALUES(9,?1,?2,?3)",
                        params![name, kind, definition],
                    )
                    .map_err(|_| storage("v9_registry"))?;
            }
            transaction
                .execute(
                    "INSERT INTO chain_post_close_layouts \
                     (layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                      artifact_codec_version,input_codec_version,stage_codec_version, \
                      description,bundle_sha256) \
                     VALUES(9,8,?1,1,1,1,'chain-post-close-layout-v9',?2)",
                    params![v8.digest.as_str(), v9.digest.as_str()],
                )
                .map_err(|_| storage("v9_seal"))?;
        }
        let receipt =
            verify_v9_installed(&transaction, &v1, &v2, &v3, &v4, &v5, &v6, &v7, &v8, &v9)?;
        super::validate_migration_audit_facts(&transaction)?;
        super::validate_all_runs_at_layout(&transaction, 9)?;
        Ok(receipt)
    })();
    let result = match result {
        Ok(receipt) => transaction
            .commit()
            .map(|()| receipt)
            .map_err(|_| storage("commit")),
        Err(error) => match transaction.rollback() {
            Ok(()) => Err(error),
            Err(_) => Err(storage("rollback")),
        },
    };
    if !connection.is_autocommit() {
        let rollback = connection.execute_batch("ROLLBACK;");
        if rollback.is_err() || !connection.is_autocommit() {
            restore_query_only(connection, 1)?;
            return Err(storage("transaction_cleanup"));
        }
    }
    restore_query_only(connection, original_query_only)?;
    result
}

fn run_v8(
    connection: &mut Connection,
    operation: V8Operation,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let v1 = Bundle::verified()?;
    let v2 = V2Bundle::verified()?;
    let v3 = V3Bundle::verified()?;
    let v4 = V4Bundle::verified()?;
    let v5 = V5Bundle::verified()?;
    let v6 = V6Bundle::verified()?;
    let v7 = V7Bundle::verified()?;
    let v8 = V8Bundle::verified()?;
    if !connection.is_autocommit() {
        return Err(ChainPostCloseError::ConnectionSafeguardFailed);
    }
    let original_query_only = pragma(connection, "PRAGMA query_only")?;
    if !matches!(original_query_only, 0 | 1)
        || (operation == V8Operation::Migrate && original_query_only != 0)
    {
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
    verify_safeguards(connection, original_query_only)?;

    let behavior = if operation == V8Operation::Migrate {
        TransactionBehavior::Immediate
    } else {
        TransactionBehavior::Deferred
    };
    let transaction = connection
        .transaction_with_behavior(behavior)
        .map_err(|_| storage("begin"))?;
    let result = (|| {
        attest_bundled_connection(&transaction).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        verify_safeguards(&transaction, 1)?;
        if operation == V8Operation::Migrate {
            restore_query_only(&transaction, original_query_only)?;
            verify_v7_installed(&transaction, &v1, &v2, &v3, &v4, &v5, &v6, &v7)?;
            super::validate_migration_audit_facts(&transaction)?;
            super::validate_all_runs_at_layout(&transaction, 7)?;
            transaction
                .execute_batch(V8_DDL)
                .map_err(|_| storage("v8_ddl"))?;
            // Validate before registry rows create their deferred reference to the new seal.
            super::validate_all_runs_at_layout(&transaction, 8)?;
            for (name, kind, _, definition) in &v8.definitions {
                let definition = std::str::from_utf8(definition)
                    .map_err(|_| ChainPostCloseError::BundleRejected)?;
                transaction
                    .execute(
                        "INSERT INTO chain_post_close_layout_objects \
                         (layout_version,name,object_type,definition) VALUES(8,?1,?2,?3)",
                        params![name, kind, definition],
                    )
                    .map_err(|_| storage("v8_registry"))?;
            }
            transaction
                .execute(
                    "INSERT INTO chain_post_close_layouts \
                     (layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                      artifact_codec_version,input_codec_version,stage_codec_version, \
                      description,bundle_sha256) \
                     VALUES(8,7,?1,1,1,1,'chain-post-close-layout-v8',?2)",
                    params![v7.digest.as_str(), v8.digest.as_str()],
                )
                .map_err(|_| storage("v8_seal"))?;
        }
        let receipt = verify_v8_installed(&transaction, &v1, &v2, &v3, &v4, &v5, &v6, &v7, &v8)?;
        Ok(receipt)
    })();
    let result = match result {
        Ok(receipt) => transaction
            .commit()
            .map(|()| receipt)
            .map_err(|_| storage("commit")),
        Err(error) => match transaction.rollback() {
            Ok(()) => Err(error),
            Err(_) => Err(storage("rollback")),
        },
    };
    if !connection.is_autocommit() {
        let rollback = connection.execute_batch("ROLLBACK;");
        if rollback.is_err() || !connection.is_autocommit() {
            restore_query_only(connection, 1)?;
            return Err(storage("transaction_cleanup"));
        }
    }
    restore_query_only(connection, original_query_only)?;
    result
}

fn qualification_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<LegacyQualificationSnapshot> {
    Ok(LegacyQualificationSnapshot {
        intent_id: row.get(0)?,
        effect_kind: row.get(1)?,
        outer_ordinal: row.get(2)?,
        code: row.get(3)?,
        qualification_kind: row.get(4)?,
        begin_run_version: row.get(5)?,
        begin_request_sha256: row.get(6)?,
        begin_lease_owner: row.get(7)?,
        begin_lease_generation: row.get(8)?,
        begun_at: row.get(9)?,
        result_outcome: row.get(10)?,
        result_run_version: row.get(11)?,
        result_sha256: row.get(12)?,
        result_lease_owner: row.get(13)?,
        result_lease_generation: row.get(14)?,
        result_committed_at: row.get(15)?,
        cache_run_version: row.get(16)?,
        cache_sha256: row.get(17)?,
        cache_lease_owner: row.get(18)?,
        cache_lease_generation: row.get(19)?,
        cache_written_at: row.get(20)?,
        qualified_from_layout_version: row.get(21)?,
        sealed_by_layout_version: row.get(22)?,
    })
}

fn snapshot_legacy_qualifications(
    connection: &Connection,
) -> Result<Vec<LegacyQualificationSnapshot>, ChainPostCloseError> {
    let mut statement = connection
        .prepare(
            "SELECT begun.intent_id,begun.effect_kind,begun.effect_ordinal,begun.effect_key, \
           CASE WHEN result.intent_id IS NULL THEN 'LegacyUnconfirmed' \
                WHEN result.outcome='BusinessError' THEN 'LegacyBusinessError' \
                WHEN cache.intent_id IS NULL THEN 'LegacyReturnedPendingCache' \
                ELSE 'LegacyReturnedApplied' END, \
           begun.run_version,begun.request_sha256,begun.lease_owner,begun.lease_generation, \
           begun.begun_at,result.outcome,result.run_version,result.result_sha256, \
           result.lease_owner,result.lease_generation,result.committed_at,cache.run_version, \
           cache.concepts_sha256,cache.lease_owner,cache.lease_generation,cache.written_at,6,7 \
         FROM chain_post_close_stage_begins AS begun \
         LEFT JOIN chain_post_close_stage_results AS result \
           ON result.intent_id=begun.intent_id AND result.effect_kind=begun.effect_kind \
          AND result.effect_ordinal=begun.effect_ordinal \
         LEFT JOIN chain_post_close_concept_cache_writes AS cache \
           ON cache.intent_id=begun.intent_id AND cache.effect_kind=begun.effect_kind \
          AND cache.effect_ordinal=begun.effect_ordinal \
         WHERE begun.effect_kind='ConceptProvider' \
         ORDER BY begun.intent_id,begun.effect_ordinal",
        )
        .map_err(|_| storage("v7 qualification snapshot"))?;
    let rows = statement
        .query_map([], qualification_from_row)
        .map_err(|_| storage("v7 qualification snapshot"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("v7 qualification snapshot"))?;
    Ok(rows)
}

fn install_legacy_qualifications(
    connection: &Connection,
    rows: &[LegacyQualificationSnapshot],
) -> Result<(), ChainPostCloseError> {
    let mut statement = connection.prepare(
        "INSERT INTO chain_post_close_concept_rpc_legacy_outer_qualifications( \
         intent_id,effect_kind,outer_ordinal,code,qualification_kind,begin_run_version, \
         begin_request_sha256,begin_lease_owner,begin_lease_generation,begun_at, \
         result_outcome,result_run_version,result_sha256,result_lease_owner, \
         result_lease_generation,result_committed_at,cache_run_version,cache_sha256, \
         cache_lease_owner,cache_lease_generation,cache_written_at, \
         qualified_from_layout_version,sealed_by_layout_version) \
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",
    ).map_err(|_| storage("v7_legacy_qualification"))?;
    for row in rows {
        statement
            .execute(params![
                &row.intent_id,
                &row.effect_kind,
                row.outer_ordinal,
                &row.code,
                &row.qualification_kind,
                row.begin_run_version,
                &row.begin_request_sha256,
                &row.begin_lease_owner,
                row.begin_lease_generation,
                row.begun_at,
                &row.result_outcome,
                row.result_run_version,
                &row.result_sha256,
                &row.result_lease_owner,
                row.result_lease_generation,
                row.result_committed_at,
                row.cache_run_version,
                &row.cache_sha256,
                &row.cache_lease_owner,
                row.cache_lease_generation,
                row.cache_written_at,
                row.qualified_from_layout_version,
                row.sealed_by_layout_version,
            ])
            .map_err(|_| storage("v7_legacy_qualification"))?;
    }
    Ok(())
}

fn read_legacy_qualifications(
    connection: &Connection,
) -> Result<Vec<LegacyQualificationSnapshot>, ChainPostCloseError> {
    let mut statement = connection.prepare(
        "SELECT intent_id,effect_kind,outer_ordinal,code,qualification_kind,begin_run_version, \
         begin_request_sha256,begin_lease_owner,begin_lease_generation,begun_at,result_outcome, \
         result_run_version,result_sha256,result_lease_owner,result_lease_generation, \
         result_committed_at,cache_run_version,cache_sha256,cache_lease_owner, \
         cache_lease_generation,cache_written_at,qualified_from_layout_version, \
         sealed_by_layout_version \
         FROM chain_post_close_concept_rpc_legacy_outer_qualifications \
         ORDER BY intent_id,outer_ordinal",
    ).map_err(|_| storage("v7 qualification verification"))?;
    let rows = statement
        .query_map([], qualification_from_row)
        .map_err(|_| storage("v7 qualification verification"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| storage("v7 qualification verification"))?;
    Ok(rows)
}

fn verify_v1_metadata(connection: &Connection, bundle: &Bundle) -> Result<(), ChainPostCloseError> {
    let headers = connection
        .prepare(
            "SELECT schema_version,artifact_codec_version,description,bundle_sha256 \
             FROM chain_post_close_schema ORDER BY schema_version",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if headers
        != vec![(
            1,
            1,
            "chain-post-close-schema-v1".to_owned(),
            bundle.digest.as_str().to_owned(),
        )]
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let registered = registered(connection)?;
    if registered != bundle.registered_definitions() {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}

fn verify_v2_installed(
    connection: &Connection,
    v1: &Bundle,
    v2: &V2Bundle,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    verify_v1_metadata(connection, v1)?;
    if v2_catalog(connection)? != v2.definitions {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let layouts = connection
        .prepare(
            "SELECT layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                    artifact_codec_version,input_codec_version,stage_codec_version, \
                    description,bundle_sha256 \
             FROM chain_post_close_layouts ORDER BY layout_version",
        )
        .and_then(|mut statement| {
            statement
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
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if layouts.iter().any(|row| row.0 > 2) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    if layouts
        != vec![(
            2,
            1,
            v1.digest.as_str().to_owned(),
            1,
            1,
            1,
            "chain-post-close-layout-v2".to_owned(),
            v2.digest.as_str().to_owned(),
        )]
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let registered = connection
        .prepare(
            "SELECT layout_version,name,object_type,CAST(definition AS BLOB) \
             FROM chain_post_close_layout_objects ORDER BY layout_version,name",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let expected_registered = v2
        .registered_definitions()
        .into_iter()
        .map(|(name, kind, definition)| (2, name, kind, definition))
        .collect::<Vec<_>>();
    if registered != expected_registered {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(ChainPostCloseSchemaReceipt {
        ddl_sha256: v2.digest.clone(),
        schema_version: 2,
    })
}

fn verify_v3_installed(
    connection: &Connection,
    v1: &Bundle,
    v2: &V2Bundle,
    v3: &V3Bundle,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    verify_v1_metadata(connection, v1)?;
    if v3_catalog(connection)? != v3.definitions {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let layouts = connection
        .prepare(
            "SELECT layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                    artifact_codec_version,input_codec_version,stage_codec_version, \
                    description,bundle_sha256 \
             FROM chain_post_close_layouts ORDER BY layout_version",
        )
        .and_then(|mut statement| {
            statement
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
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if layouts.iter().any(|row| row.0 > 3) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    if layouts
        != vec![
            (
                2,
                1,
                v1.digest.as_str().to_owned(),
                1,
                1,
                1,
                "chain-post-close-layout-v2".to_owned(),
                v2.digest.as_str().to_owned(),
            ),
            (
                3,
                2,
                v2.digest.as_str().to_owned(),
                1,
                1,
                1,
                "chain-post-close-layout-v3".to_owned(),
                v3.digest.as_str().to_owned(),
            ),
        ]
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let registered = connection
        .prepare(
            "SELECT layout_version,name,object_type,CAST(definition AS BLOB) \
             FROM chain_post_close_layout_objects ORDER BY layout_version,name",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let mut expected_registered = v2
        .registered_definitions()
        .into_iter()
        .map(|(name, kind, definition)| (2, name, kind, definition))
        .collect::<Vec<_>>();
    expected_registered.extend(
        v3.registered_definitions()
            .into_iter()
            .map(|(name, kind, definition)| (3, name, kind, definition)),
    );
    if registered != expected_registered {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(ChainPostCloseSchemaReceipt {
        ddl_sha256: v3.digest.clone(),
        schema_version: 3,
    })
}

fn verify_v4_installed(
    connection: &Connection,
    v1: &Bundle,
    v2: &V2Bundle,
    v3: &V3Bundle,
    v4: &V4Bundle,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    verify_v1_metadata(connection, v1)?;
    if v4_catalog(connection)? != v4.definitions {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let layouts = connection
        .prepare(
            "SELECT layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                    artifact_codec_version,input_codec_version,stage_codec_version, \
                    description,bundle_sha256 \
             FROM chain_post_close_layouts ORDER BY layout_version",
        )
        .and_then(|mut statement| {
            statement
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
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if layouts.iter().any(|row| row.0 > 4) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    if layouts
        != vec![
            (
                2,
                1,
                v1.digest.as_str().to_owned(),
                1,
                1,
                1,
                "chain-post-close-layout-v2".to_owned(),
                v2.digest.as_str().to_owned(),
            ),
            (
                3,
                2,
                v2.digest.as_str().to_owned(),
                1,
                1,
                1,
                "chain-post-close-layout-v3".to_owned(),
                v3.digest.as_str().to_owned(),
            ),
            (
                4,
                3,
                v3.digest.as_str().to_owned(),
                1,
                1,
                1,
                "chain-post-close-layout-v4".to_owned(),
                v4.digest.as_str().to_owned(),
            ),
        ]
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let registered = connection
        .prepare(
            "SELECT layout_version,name,object_type,CAST(definition AS BLOB) \
             FROM chain_post_close_layout_objects ORDER BY layout_version,name",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let mut expected_registered = v2
        .registered_definitions()
        .into_iter()
        .map(|(name, kind, definition)| (2, name, kind, definition))
        .collect::<Vec<_>>();
    expected_registered.extend(
        v3.registered_definitions()
            .into_iter()
            .map(|(name, kind, definition)| (3, name, kind, definition)),
    );
    expected_registered.extend(
        v4.registered_definitions()
            .into_iter()
            .map(|(name, kind, definition)| (4, name, kind, definition)),
    );
    if registered != expected_registered {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(ChainPostCloseSchemaReceipt {
        ddl_sha256: v4.digest.clone(),
        schema_version: 4,
    })
}

fn verify_v5_installed(
    connection: &Connection,
    v1: &Bundle,
    v2: &V2Bundle,
    v3: &V3Bundle,
    v4: &V4Bundle,
    v5: &V5Bundle,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    verify_v1_metadata(connection, v1)?;
    if v5_catalog(connection)? != v5.definitions {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let layouts = connection
        .prepare(
            "SELECT layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                    artifact_codec_version,input_codec_version,stage_codec_version, \
                    description,bundle_sha256 \
             FROM chain_post_close_layouts ORDER BY layout_version",
        )
        .and_then(|mut statement| {
            statement
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
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if layouts.iter().any(|row| row.0 > 5) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    if layouts
        != vec![
            (
                2,
                1,
                v1.digest.as_str().to_owned(),
                1,
                1,
                1,
                "chain-post-close-layout-v2".to_owned(),
                v2.digest.as_str().to_owned(),
            ),
            (
                3,
                2,
                v2.digest.as_str().to_owned(),
                1,
                1,
                1,
                "chain-post-close-layout-v3".to_owned(),
                v3.digest.as_str().to_owned(),
            ),
            (
                4,
                3,
                v3.digest.as_str().to_owned(),
                1,
                1,
                1,
                "chain-post-close-layout-v4".to_owned(),
                v4.digest.as_str().to_owned(),
            ),
            (
                5,
                4,
                v4.digest.as_str().to_owned(),
                1,
                1,
                1,
                "chain-post-close-layout-v5".to_owned(),
                v5.digest.as_str().to_owned(),
            ),
        ]
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let registered = connection
        .prepare(
            "SELECT layout_version,name,object_type,CAST(definition AS BLOB) \
             FROM chain_post_close_layout_objects ORDER BY layout_version,name",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let mut expected_registered = v2
        .registered_definitions()
        .into_iter()
        .map(|(name, kind, definition)| (2, name, kind, definition))
        .collect::<Vec<_>>();
    expected_registered.extend(
        v3.registered_definitions()
            .into_iter()
            .map(|(name, kind, definition)| (3, name, kind, definition)),
    );
    expected_registered.extend(
        v4.registered_definitions()
            .into_iter()
            .map(|(name, kind, definition)| (4, name, kind, definition)),
    );
    expected_registered.extend(
        v5.registered_definitions()
            .into_iter()
            .map(|(name, kind, definition)| (5, name, kind, definition)),
    );
    if registered != expected_registered {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(ChainPostCloseSchemaReceipt {
        ddl_sha256: v5.digest.clone(),
        schema_version: 5,
    })
}

fn verify_v6_installed(
    connection: &Connection,
    v1: &Bundle,
    v2: &V2Bundle,
    v3: &V3Bundle,
    v4: &V4Bundle,
    v5: &V5Bundle,
    v6: &V6Bundle,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    verify_v1_metadata(connection, v1)?;
    if v6_catalog(connection)? != v6.definitions {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let layouts = connection
        .prepare(
            "SELECT layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                    artifact_codec_version,input_codec_version,stage_codec_version, \
                    description,bundle_sha256 \
             FROM chain_post_close_layouts ORDER BY layout_version",
        )
        .and_then(|mut statement| {
            statement
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
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if layouts.iter().any(|row| row.0 > 6) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    let expected = vec![
        (
            2,
            1,
            v1.digest.as_str(),
            "chain-post-close-layout-v2",
            v2.digest.as_str(),
        ),
        (
            3,
            2,
            v2.digest.as_str(),
            "chain-post-close-layout-v3",
            v3.digest.as_str(),
        ),
        (
            4,
            3,
            v3.digest.as_str(),
            "chain-post-close-layout-v4",
            v4.digest.as_str(),
        ),
        (
            5,
            4,
            v4.digest.as_str(),
            "chain-post-close-layout-v5",
            v5.digest.as_str(),
        ),
        (
            6,
            5,
            v5.digest.as_str(),
            "chain-post-close-layout-v6",
            v6.digest.as_str(),
        ),
    ];
    if layouts.len() != expected.len()
        || layouts.iter().zip(expected).any(|(actual, expected)| {
            actual.0 != expected.0
                || actual.1 != expected.1
                || actual.2 != expected.2
                || actual.3 != 1
                || actual.4 != 1
                || actual.5 != 1
                || actual.6 != expected.3
                || actual.7 != expected.4
        })
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let registered = connection
        .prepare(
            "SELECT layout_version,name,object_type,CAST(definition AS BLOB) \
             FROM chain_post_close_layout_objects ORDER BY layout_version,name",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let mut expected_registered = v2
        .registered_definitions()
        .into_iter()
        .map(|(name, kind, definition)| (2, name, kind, definition))
        .collect::<Vec<_>>();
    for (version, definitions) in [
        (3, v3.registered_definitions()),
        (4, v4.registered_definitions()),
        (5, v5.registered_definitions()),
        (6, v6.registered_definitions()),
    ] {
        expected_registered.extend(
            definitions
                .into_iter()
                .map(|(name, kind, definition)| (version, name, kind, definition)),
        );
    }
    if registered != expected_registered {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let missing_status: i64 = connection
        .query_row(
            "SELECT count(*) FROM chain_post_close_board_attempt_results AS result \
             LEFT JOIN chain_post_close_board_status_materials AS material \
               ON material.intent_id=result.intent_id AND material.kind=result.kind \
              AND material.attempt_ordinal=result.attempt_ordinal \
             WHERE result.wire_outcome='Status' AND material.intent_id IS NULL",
            [],
            |row| row.get(0),
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if missing_status != 0 {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(ChainPostCloseSchemaReceipt {
        ddl_sha256: v6.digest.clone(),
        schema_version: 6,
    })
}

#[allow(clippy::too_many_arguments)]
fn verify_v7_installed(
    connection: &Connection,
    v1: &Bundle,
    v2: &V2Bundle,
    v3: &V3Bundle,
    v4: &V4Bundle,
    v5: &V5Bundle,
    v6: &V6Bundle,
    v7: &V7Bundle,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    verify_v1_metadata(connection, v1)?;
    if v7_catalog(connection)? != v7.definitions {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let layouts = connection
        .prepare(
            "SELECT layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                    artifact_codec_version,input_codec_version,stage_codec_version, \
                    description,bundle_sha256 \
             FROM chain_post_close_layouts ORDER BY layout_version",
        )
        .and_then(|mut statement| {
            statement
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
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if layouts.iter().any(|row| row.0 > 7) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    let expected = vec![
        (
            2,
            1,
            v1.digest.as_str(),
            "chain-post-close-layout-v2",
            v2.digest.as_str(),
        ),
        (
            3,
            2,
            v2.digest.as_str(),
            "chain-post-close-layout-v3",
            v3.digest.as_str(),
        ),
        (
            4,
            3,
            v3.digest.as_str(),
            "chain-post-close-layout-v4",
            v4.digest.as_str(),
        ),
        (
            5,
            4,
            v4.digest.as_str(),
            "chain-post-close-layout-v5",
            v5.digest.as_str(),
        ),
        (
            6,
            5,
            v5.digest.as_str(),
            "chain-post-close-layout-v6",
            v6.digest.as_str(),
        ),
        (
            7,
            6,
            v6.digest.as_str(),
            "chain-post-close-layout-v7",
            v7.digest.as_str(),
        ),
    ];
    if layouts.len() != expected.len()
        || layouts.iter().zip(expected).any(|(actual, expected)| {
            actual.0 != expected.0
                || actual.1 != expected.1
                || actual.2 != expected.2
                || actual.3 != 1
                || actual.4 != 1
                || actual.5 != 1
                || actual.6 != expected.3
                || actual.7 != expected.4
        })
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let registered = connection
        .prepare(
            "SELECT layout_version,name,object_type,CAST(definition AS BLOB) \
             FROM chain_post_close_layout_objects ORDER BY layout_version,name",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let mut expected_registered = v2
        .registered_definitions()
        .into_iter()
        .map(|(name, kind, definition)| (2, name, kind, definition))
        .collect::<Vec<_>>();
    for (version, definitions) in [
        (3, v3.registered_definitions()),
        (4, v4.registered_definitions()),
        (5, v5.registered_definitions()),
        (6, v6.registered_definitions()),
        (7, v7.registered_definitions()),
    ] {
        expected_registered.extend(
            definitions
                .into_iter()
                .map(|(name, kind, definition)| (version, name, kind, definition)),
        );
    }
    if registered != expected_registered {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    verify_v7_fact_shape(connection)?;
    Ok(ChainPostCloseSchemaReceipt {
        ddl_sha256: v7.digest.clone(),
        schema_version: 7,
    })
}

#[allow(clippy::too_many_arguments)]
fn verify_v9_installed(
    connection: &Connection,
    v1: &Bundle,
    v2: &V2Bundle,
    v3: &V3Bundle,
    v4: &V4Bundle,
    v5: &V5Bundle,
    v6: &V6Bundle,
    v7: &V7Bundle,
    v8: &V8Bundle,
    v9: &V9Bundle,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    verify_v1_metadata(connection, v1)?;
    if v9_catalog(connection)? != v9.definitions {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let layouts = connection
        .prepare(
            "SELECT layout_version,predecessor_layout_version,predecessor_bundle_sha256,\
                    artifact_codec_version,input_codec_version,stage_codec_version,\
                    description,bundle_sha256 FROM chain_post_close_layouts ORDER BY layout_version",
        )
        .and_then(|mut statement| {
            statement
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
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if layouts.iter().any(|row| row.0 > 9) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    let expected = [
        (
            2,
            1,
            v1.digest.as_str(),
            "chain-post-close-layout-v2",
            v2.digest.as_str(),
        ),
        (
            3,
            2,
            v2.digest.as_str(),
            "chain-post-close-layout-v3",
            v3.digest.as_str(),
        ),
        (
            4,
            3,
            v3.digest.as_str(),
            "chain-post-close-layout-v4",
            v4.digest.as_str(),
        ),
        (
            5,
            4,
            v4.digest.as_str(),
            "chain-post-close-layout-v5",
            v5.digest.as_str(),
        ),
        (
            6,
            5,
            v5.digest.as_str(),
            "chain-post-close-layout-v6",
            v6.digest.as_str(),
        ),
        (
            7,
            6,
            v6.digest.as_str(),
            "chain-post-close-layout-v7",
            v7.digest.as_str(),
        ),
        (
            8,
            7,
            v7.digest.as_str(),
            "chain-post-close-layout-v8",
            v8.digest.as_str(),
        ),
        (
            9,
            8,
            v8.digest.as_str(),
            "chain-post-close-layout-v9",
            v9.digest.as_str(),
        ),
    ];
    if layouts.len() != expected.len()
        || layouts.iter().zip(expected).any(|(actual, expected)| {
            actual.0 != expected.0
                || actual.1 != expected.1
                || actual.2 != expected.2
                || actual.3 != 1
                || actual.4 != 1
                || actual.5 != 1
                || actual.6 != expected.3
                || actual.7 != expected.4
        })
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let registered = connection
        .prepare(
            "SELECT layout_version,name,object_type,CAST(definition AS BLOB) \
             FROM chain_post_close_layout_objects ORDER BY layout_version,name",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let mut expected_registered = v2
        .registered_definitions()
        .into_iter()
        .map(|(name, kind, definition)| (2, name, kind, definition))
        .collect::<Vec<_>>();
    for (version, definitions) in [
        (3, v3.registered_definitions()),
        (4, v4.registered_definitions()),
        (5, v5.registered_definitions()),
        (6, v6.registered_definitions()),
        (7, v7.registered_definitions()),
        (8, v8.registered_definitions()),
        (9, v9.registered_definitions()),
    ] {
        expected_registered.extend(
            definitions
                .into_iter()
                .map(|(name, kind, definition)| (version, name, kind, definition)),
        );
    }
    if registered != expected_registered {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    verify_v7_fact_shape(connection)?;
    Ok(ChainPostCloseSchemaReceipt {
        ddl_sha256: v9.digest.clone(),
        schema_version: 9,
    })
}

#[allow(clippy::too_many_arguments)]
fn verify_v10_installed(
    connection: &Connection,
    v1: &Bundle,
    v2: &V2Bundle,
    v3: &V3Bundle,
    v4: &V4Bundle,
    v5: &V5Bundle,
    v6: &V6Bundle,
    v7: &V7Bundle,
    v8: &V8Bundle,
    v9: &V9Bundle,
    v10: &V10Bundle,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    verify_v1_metadata(connection, v1)?;
    if v10_catalog(connection)? != v10.definitions {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let layouts = connection
        .prepare(
            "SELECT layout_version,predecessor_layout_version,predecessor_bundle_sha256,\
         artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256 \
         FROM chain_post_close_layouts ORDER BY layout_version",
        )
        .and_then(|mut statement| {
            statement
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
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if layouts.iter().any(|row| row.0 > 10) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    let expected = [
        (
            2,
            1,
            v1.digest.as_str(),
            "chain-post-close-layout-v2",
            v2.digest.as_str(),
        ),
        (
            3,
            2,
            v2.digest.as_str(),
            "chain-post-close-layout-v3",
            v3.digest.as_str(),
        ),
        (
            4,
            3,
            v3.digest.as_str(),
            "chain-post-close-layout-v4",
            v4.digest.as_str(),
        ),
        (
            5,
            4,
            v4.digest.as_str(),
            "chain-post-close-layout-v5",
            v5.digest.as_str(),
        ),
        (
            6,
            5,
            v5.digest.as_str(),
            "chain-post-close-layout-v6",
            v6.digest.as_str(),
        ),
        (
            7,
            6,
            v6.digest.as_str(),
            "chain-post-close-layout-v7",
            v7.digest.as_str(),
        ),
        (
            8,
            7,
            v7.digest.as_str(),
            "chain-post-close-layout-v8",
            v8.digest.as_str(),
        ),
        (
            9,
            8,
            v8.digest.as_str(),
            "chain-post-close-layout-v9",
            v9.digest.as_str(),
        ),
        (
            10,
            9,
            v9.digest.as_str(),
            "chain-post-close-layout-v10",
            v10.digest.as_str(),
        ),
    ];
    if layouts.len() != expected.len()
        || layouts.iter().zip(expected).any(|(actual, expected)| {
            actual.0 != expected.0
                || actual.1 != expected.1
                || actual.2 != expected.2
                || actual.3 != 1
                || actual.4 != 1
                || actual.5 != 1
                || actual.6 != expected.3
                || actual.7 != expected.4
        })
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let registered = connection
        .prepare(
            "SELECT layout_version,name,object_type,CAST(definition AS BLOB) \
         FROM chain_post_close_layout_objects ORDER BY layout_version,name",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let mut expected_registered = Vec::new();
    for (version, definitions) in [
        (2, v2.registered_definitions()),
        (3, v3.registered_definitions()),
        (4, v4.registered_definitions()),
        (5, v5.registered_definitions()),
        (6, v6.registered_definitions()),
        (7, v7.registered_definitions()),
        (8, v8.registered_definitions()),
        (9, v9.registered_definitions()),
        (10, v10.registered_definitions()),
    ] {
        expected_registered.extend(
            definitions
                .into_iter()
                .map(|(name, kind, definition)| (version, name, kind, definition)),
        );
    }
    if registered != expected_registered {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    verify_v7_fact_shape(connection)?;
    Ok(ChainPostCloseSchemaReceipt {
        ddl_sha256: v10.digest.clone(),
        schema_version: 10,
    })
}

fn verify_v8_installed(
    connection: &Connection,
    v1: &Bundle,
    v2: &V2Bundle,
    v3: &V3Bundle,
    v4: &V4Bundle,
    v5: &V5Bundle,
    v6: &V6Bundle,
    v7: &V7Bundle,
    v8: &V8Bundle,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    verify_v1_metadata(connection, v1)?;
    if v8_catalog(connection)? != v8.definitions {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let layouts = connection
        .prepare(
            "SELECT layout_version,predecessor_layout_version,predecessor_bundle_sha256,\
                    artifact_codec_version,input_codec_version,stage_codec_version,\
                    description,bundle_sha256 FROM chain_post_close_layouts ORDER BY layout_version",
        )
        .and_then(|mut statement| {
            statement
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
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if layouts.iter().any(|row| row.0 > 8) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    let expected = [
        (
            2,
            1,
            v1.digest.as_str(),
            "chain-post-close-layout-v2",
            v2.digest.as_str(),
        ),
        (
            3,
            2,
            v2.digest.as_str(),
            "chain-post-close-layout-v3",
            v3.digest.as_str(),
        ),
        (
            4,
            3,
            v3.digest.as_str(),
            "chain-post-close-layout-v4",
            v4.digest.as_str(),
        ),
        (
            5,
            4,
            v4.digest.as_str(),
            "chain-post-close-layout-v5",
            v5.digest.as_str(),
        ),
        (
            6,
            5,
            v5.digest.as_str(),
            "chain-post-close-layout-v6",
            v6.digest.as_str(),
        ),
        (
            7,
            6,
            v6.digest.as_str(),
            "chain-post-close-layout-v7",
            v7.digest.as_str(),
        ),
        (
            8,
            7,
            v7.digest.as_str(),
            "chain-post-close-layout-v8",
            v8.digest.as_str(),
        ),
    ];
    if layouts.len() != expected.len()
        || layouts.iter().zip(expected).any(|(actual, expected)| {
            actual.0 != expected.0
                || actual.1 != expected.1
                || actual.2 != expected.2
                || actual.3 != 1
                || actual.4 != 1
                || actual.5 != 1
                || actual.6 != expected.3
                || actual.7 != expected.4
        })
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let registered = connection
        .prepare(
            "SELECT layout_version,name,object_type,CAST(definition AS BLOB) \
             FROM chain_post_close_layout_objects ORDER BY layout_version,name",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let mut expected_registered = v2
        .registered_definitions()
        .into_iter()
        .map(|(name, kind, definition)| (2, name, kind, definition))
        .collect::<Vec<_>>();
    for (version, definitions) in [
        (3, v3.registered_definitions()),
        (4, v4.registered_definitions()),
        (5, v5.registered_definitions()),
        (6, v6.registered_definitions()),
        (7, v7.registered_definitions()),
        (8, v8.registered_definitions()),
    ] {
        expected_registered.extend(
            definitions
                .into_iter()
                .map(|(name, kind, definition)| (version, name, kind, definition)),
        );
    }
    if registered != expected_registered {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    verify_v7_fact_shape(connection)?;
    Ok(ChainPostCloseSchemaReceipt {
        ddl_sha256: v8.digest.clone(),
        schema_version: 8,
    })
}

fn verify_v7_fact_shape(connection: &Connection) -> Result<(), ChainPostCloseError> {
    let missing_status: i64 = connection
        .query_row(
            "SELECT count(*) FROM chain_post_close_board_attempt_results AS result \
             LEFT JOIN chain_post_close_board_status_materials AS material \
               ON material.intent_id=result.intent_id AND material.kind=result.kind \
              AND material.attempt_ordinal=result.attempt_ordinal \
             WHERE result.wire_outcome='Status' AND material.intent_id IS NULL",
            [],
            |row| row.get(0),
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let legacy_mismatch: i64 = connection
        .query_row(
            "SELECT count(*) FROM chain_post_close_stage_begins AS begun \
             LEFT JOIN chain_post_close_concept_rpc_legacy_outer_qualifications AS legacy \
               ON legacy.intent_id=begun.intent_id AND legacy.effect_kind=begun.effect_kind \
              AND legacy.outer_ordinal=begun.effect_ordinal AND legacy.code=begun.effect_key \
              AND legacy.begin_run_version=begun.run_version \
              AND legacy.begin_request_sha256=begun.request_sha256 \
              AND legacy.begin_lease_owner=begun.lease_owner \
              AND legacy.begin_lease_generation=begun.lease_generation \
              AND legacy.begun_at=begun.begun_at \
             LEFT JOIN chain_post_close_concept_rpc_finals AS final \
               ON final.intent_id=begun.intent_id AND final.outer_ordinal=begun.effect_ordinal \
              AND final.code=begun.effect_key AND final.outer_begin_run_version=begun.run_version \
              AND final.outer_begin_sha256=begun.request_sha256 \
             WHERE begun.effect_kind='ConceptProvider' \
               AND ((legacy.intent_id IS NULL AND final.intent_id IS NULL) \
                    OR (legacy.intent_id IS NOT NULL AND final.intent_id IS NOT NULL))",
            [],
            |row| row.get(0),
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let orphan_provenance: i64 = connection
        .query_row(
            "SELECT \
               (SELECT count(*) FROM chain_post_close_concept_rpc_legacy_outer_qualifications AS q \
                LEFT JOIN chain_post_close_stage_begins AS b \
                  ON b.intent_id=q.intent_id AND b.effect_kind=q.effect_kind \
                 AND b.effect_ordinal=q.outer_ordinal AND b.effect_key=q.code \
                 AND b.run_version=q.begin_run_version WHERE b.intent_id IS NULL) + \
               (SELECT count(*) FROM chain_post_close_concept_rpc_finals AS f \
                LEFT JOIN chain_post_close_stage_begins AS b \
                  ON b.intent_id=f.intent_id AND b.effect_kind='ConceptProvider' \
                 AND b.effect_ordinal=f.outer_ordinal AND b.effect_key=f.code \
                 AND b.run_version=f.outer_begin_run_version \
                 AND b.request_sha256=f.outer_begin_sha256 WHERE b.intent_id IS NULL)",
            [],
            |row| row.get(0),
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if missing_status != 0 || legacy_mismatch != 0 || orphan_provenance != 0 {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}

pub(super) fn verify_runtime_layout(connection: &Connection) -> Result<(), ChainPostCloseError> {
    let original_query_only = pragma(connection, "PRAGMA query_only")?;
    verify_safeguards(connection, original_query_only)?;
    let attestation = attest_bundled_connection(connection)
        .map_err(|_| ChainPostCloseError::SchemaRejected)
        .and_then(|_| verify_safeguards(connection, 1));
    restore_query_only(connection, original_query_only)?;
    verify_safeguards(connection, original_query_only)?;
    attestation?;
    let v1 = Bundle::verified()?;
    let v2 = V2Bundle::verified()?;
    match latest_layout(connection)? {
        Some(2) => verify_v2_installed(connection, &v1, &v2).map(|_| ()),
        Some(3) => {
            let v3 = V3Bundle::verified()?;
            verify_v3_installed(connection, &v1, &v2, &v3).map(|_| ())
        }
        Some(4) => {
            let v3 = V3Bundle::verified()?;
            let v4 = V4Bundle::verified()?;
            verify_v4_installed(connection, &v1, &v2, &v3, &v4).map(|_| ())
        }
        Some(5) => {
            let v3 = V3Bundle::verified()?;
            let v4 = V4Bundle::verified()?;
            let v5 = V5Bundle::verified()?;
            verify_v5_installed(connection, &v1, &v2, &v3, &v4, &v5).map(|_| ())
        }
        Some(6) => {
            let v3 = V3Bundle::verified()?;
            let v4 = V4Bundle::verified()?;
            let v5 = V5Bundle::verified()?;
            let v6 = V6Bundle::verified()?;
            verify_v6_installed(connection, &v1, &v2, &v3, &v4, &v5, &v6).map(|_| ())
        }
        Some(7) => {
            let v3 = V3Bundle::verified()?;
            let v4 = V4Bundle::verified()?;
            let v5 = V5Bundle::verified()?;
            let v6 = V6Bundle::verified()?;
            let v7 = V7Bundle::verified()?;
            verify_v7_installed(connection, &v1, &v2, &v3, &v4, &v5, &v6, &v7).map(|_| ())
        }
        Some(8) => {
            let v3 = V3Bundle::verified()?;
            let v4 = V4Bundle::verified()?;
            let v5 = V5Bundle::verified()?;
            let v6 = V6Bundle::verified()?;
            let v7 = V7Bundle::verified()?;
            let v8 = V8Bundle::verified()?;
            verify_v8_installed(connection, &v1, &v2, &v3, &v4, &v5, &v6, &v7, &v8).map(|_| ())
        }
        Some(9) => {
            let v2 = V2Bundle::verified()?;
            let v3 = V3Bundle::verified()?;
            let v4 = V4Bundle::verified()?;
            let v5 = V5Bundle::verified()?;
            let v6 = V6Bundle::verified()?;
            let v7 = V7Bundle::verified()?;
            let v8 = V8Bundle::verified()?;
            let v9 = V9Bundle::verified()?;
            verify_v9_installed(connection, &v1, &v2, &v3, &v4, &v5, &v6, &v7, &v8, &v9).map(|_| ())
        }
        Some(10) => {
            let v3 = V3Bundle::verified()?;
            let v4 = V4Bundle::verified()?;
            let v5 = V5Bundle::verified()?;
            let v6 = V6Bundle::verified()?;
            let v7 = V7Bundle::verified()?;
            let v8 = V8Bundle::verified()?;
            let v9 = V9Bundle::verified()?;
            let v10 = V10Bundle::verified()?;
            verify_v10_installed(
                connection, &v1, &v2, &v3, &v4, &v5, &v6, &v7, &v8, &v9, &v10,
            )
            .map(|_| ())
        }
        Some(11) => v11::verify_installed(connection).map(|_| ()),
        Some(12) => v12::verify_installed(connection).map(|_| ()),
        Some(13) => v13::verify_installed(connection).map(|_| ()),
        Some(version) if version > 11 => Err(ChainPostCloseError::UnsupportedVersion),
        _ => Err(ChainPostCloseError::SchemaRejected),
    }
}

pub(super) fn verify_runtime_layout_version(
    connection: &Connection,
    expected: i64,
) -> Result<(), ChainPostCloseError> {
    if runtime_layout_version(connection)? != expected {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    Ok(())
}

pub(super) fn runtime_layout_version(connection: &Connection) -> Result<i64, ChainPostCloseError> {
    verify_runtime_layout(connection)?;
    latest_layout(connection)?.ok_or(ChainPostCloseError::SchemaRejected)
}

fn latest_layout(connection: &Connection) -> Result<Option<i64>, ChainPostCloseError> {
    connection
        .query_row(
            "SELECT MAX(layout_version) FROM chain_post_close_layouts",
            [],
            |row| row.get(0),
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)
}

fn registered(connection: &Connection) -> Result<Vec<RegisteredDefinition>, ChainPostCloseError> {
    let mut statement = connection
        .prepare(
            "SELECT name,object_type,CAST(definition AS BLOB) \
             FROM chain_post_close_objects ORDER BY name",
        )
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let definitions = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|_| ChainPostCloseError::SchemaRejected)?
        .collect::<rusqlite::Result<_>>()
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    Ok(definitions)
}

fn storage(operation: &'static str) -> ChainPostCloseError {
    ChainPostCloseError::StorageFailed { operation }
}

fn pragma(connection: &Connection, sql: &'static str) -> Result<i64, ChainPostCloseError> {
    connection
        .query_row(sql, [], |row| row.get(0))
        .map_err(|_| ChainPostCloseError::ConnectionSafeguardFailed)
}

fn verify_safeguards(connection: &Connection, query_only: i64) -> Result<(), ChainPostCloseError> {
    for (sql, expected) in [
        ("PRAGMA query_only", query_only),
        ("PRAGMA foreign_keys", 1),
        ("PRAGMA recursive_triggers", 1),
        ("PRAGMA synchronous", 2),
        ("PRAGMA busy_timeout", 250),
    ] {
        if pragma(connection, sql)? != expected {
            return Err(ChainPostCloseError::ConnectionSafeguardFailed);
        }
    }
    Ok(())
}

fn restore_query_only(connection: &Connection, original: i64) -> Result<(), ChainPostCloseError> {
    let sql = match original {
        0 => "PRAGMA query_only=OFF;",
        1 => "PRAGMA query_only=ON;",
        _ => return Err(ChainPostCloseError::ConnectionSafeguardFailed),
    };
    connection
        .execute_batch(sql)
        .map_err(|_| ChainPostCloseError::ConnectionSafeguardFailed)?;
    if pragma(connection, "PRAGMA query_only")? != original {
        return Err(ChainPostCloseError::ConnectionSafeguardFailed);
    }
    Ok(())
}

fn run(
    connection: &mut Connection,
    operation: Operation,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    let bundle = Bundle::verified()?;
    if !connection.is_autocommit() {
        return Err(ChainPostCloseError::ConnectionSafeguardFailed);
    }
    let original_query_only = pragma(connection, "PRAGMA query_only")?;
    if !matches!(original_query_only, 0 | 1)
        || (operation == Operation::Install && original_query_only != 0)
    {
        return Err(ChainPostCloseError::ConnectionSafeguardFailed);
    }
    // These settings must be established before BEGIN, on this actual handle.
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON; PRAGMA synchronous=FULL;",
        )
        .map_err(|_| ChainPostCloseError::ConnectionSafeguardFailed)?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|_| ChainPostCloseError::ConnectionSafeguardFailed)?;
    verify_safeguards(connection, original_query_only)?;

    let behavior = if operation == Operation::Install {
        TransactionBehavior::Immediate
    } else {
        TransactionBehavior::Deferred
    };
    let transaction = connection
        .transaction_with_behavior(behavior)
        .map_err(|_| storage("begin"))?;
    let result = inspect_or_install(&transaction, &bundle, operation, original_query_only);
    let result = match result {
        Ok(receipt) => transaction
            .commit()
            .map(|()| receipt)
            .map_err(|_| storage("commit")),
        Err(error) => match transaction.rollback() {
            Ok(()) => Err(error),
            Err(_) => Err(storage("rollback")),
        },
    };

    // Do not rely only on Transaction's best-effort Drop rollback after a commit failure.
    if !connection.is_autocommit() {
        let rollback = connection.execute_batch("ROLLBACK;");
        if rollback.is_err() || !connection.is_autocommit() {
            restore_query_only(connection, 1)?;
            return Err(storage("transaction_cleanup"));
        }
    }
    // Failure to restore is itself a stopping error, never a successful attestation.
    restore_query_only(connection, original_query_only)?;
    result
}

fn inspect_or_install(
    connection: &Connection,
    bundle: &Bundle,
    operation: Operation,
    original_query_only: i64,
) -> Result<ChainPostCloseSchemaReceipt, ChainPostCloseError> {
    verify_safeguards(connection, original_query_only)?;
    attest_bundled_connection(connection).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    verify_safeguards(connection, 1)?;
    if operation == Operation::Install {
        // Only restore the capability captured before BEGIN; a readonly request was refused.
        restore_query_only(connection, original_query_only)?;
        verify_safeguards(connection, original_query_only)?;
    }
    let actual = catalog(connection)?;
    if actual.is_empty() {
        if operation == Operation::Verify {
            return Err(ChainPostCloseError::NotInstalled);
        }
        connection
            .execute_batch(DDL)
            .map_err(|_| storage("install_ddl"))?;
        for (name, kind, _, definition) in &bundle.definitions {
            let definition =
                std::str::from_utf8(definition).map_err(|_| ChainPostCloseError::BundleRejected)?;
            connection
                .execute(
                    "INSERT INTO main.chain_post_close_objects(name,object_type,definition) \
                     VALUES (?1,?2,?3)",
                    params![name, kind, definition],
                )
                .map_err(|_| storage("register_object"))?;
        }
        // Header insertion seals both tables after all fixed objects have been registered.
        connection
            .execute(
                "INSERT INTO main.chain_post_close_schema \
                 (schema_version,artifact_codec_version,description,bundle_sha256) \
                 VALUES (1,1,'chain-post-close-schema-v1',?1)",
                params![bundle.digest.as_str()],
            )
            .map_err(|_| storage("seal_schema"))?;
    }
    verify_installed(connection, bundle)?;
    Ok(ChainPostCloseSchemaReceipt {
        ddl_sha256: bundle.digest.clone(),
        schema_version: 1,
    })
}

fn catalog(connection: &Connection) -> Result<Vec<Definition>, ChainPostCloseError> {
    let mut statement = connection
        .prepare(CATALOG_SQL)
        .map_err(|_| storage("catalog"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|_| storage("catalog"))?;
    rows.collect::<rusqlite::Result<_>>()
        .map_err(|_| storage("catalog"))
}

fn verify_installed(connection: &Connection, bundle: &Bundle) -> Result<(), ChainPostCloseError> {
    if catalog(connection)? != bundle.definitions {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let mut statement = connection
        .prepare(
            "SELECT schema_version,artifact_codec_version,description,bundle_sha256 \
             FROM main.chain_post_close_schema",
        )
        .map_err(|_| storage("schema_header"))?;
    let headers = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|_| storage("schema_header"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if headers.len() != 1 {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let (version, codec, description, digest) = &headers[0];
    if *version != 1 || *codec != 1 {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    if description != "chain-post-close-schema-v1" || digest != bundle.digest.as_str() {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let mut statement = connection
        .prepare(
            "SELECT name,object_type,CAST(definition AS BLOB) \
             FROM main.chain_post_close_objects ORDER BY name",
        )
        .map_err(|_| storage("registry"))?;
    let registered = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|_| storage("registry"))?
        .collect::<rusqlite::Result<Vec<RegisteredDefinition>>>()
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if registered != bundle.registered_definitions() {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}
