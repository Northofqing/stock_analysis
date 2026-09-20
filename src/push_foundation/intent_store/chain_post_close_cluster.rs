use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{Duration, NaiveDate};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::market_data::TopStock;
use crate::monitor::push_job::{raw_digest, IntentId, UtcMicros};
use crate::pipeline::chain_analysis::preparation::FixedClusterConfiguration;
use crate::pipeline::chain_analysis::ChainCluster;

use super::{
    check_lease, inspect_concept_batch_from_recovery_on, inspect_run_on, schema, storage,
    ChainPostCloseError, LocalChainPostClose, RunLease, RunRecovery,
};

#[derive(Serialize, Deserialize)]
struct ConfigurationEnvelope {
    schema_version: u32,
    min_cluster_size: String,
}

#[derive(Serialize, Deserialize)]
struct ConceptMapEnvelope {
    schema_version: u32,
    concepts: BTreeMap<String, Vec<String>>,
}

#[derive(Serialize, Deserialize)]
struct MaterialEnvelope {
    schema_version: u32,
    min_cluster_size: String,
    clusters: Vec<ChainCluster>,
    isolated: Vec<TopStock>,
}

#[derive(Serialize, Deserialize)]
struct LifecycleEnvelope {
    schema_version: u32,
    business_date: String,
    rows: Vec<LifecycleRow>,
    days: BTreeMap<String, i64>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct LifecycleRow {
    concept: String,
    stocks: Vec<String>,
    continuation_count: i32,
}

struct LoadedConfiguration {
    run_id: String,
    context_digest: String,
    input_digest: String,
    bytes: Vec<u8>,
    digest: String,
    owner: String,
    generation: u64,
    run_version: u64,
    fixed_at: i64,
    value: FixedClusterConfiguration,
}

struct LoadedMaterial {
    run_id: String,
    context_digest: String,
    input_digest: String,
    concept_bytes: Vec<u8>,
    bytes: Vec<u8>,
    digest: String,
    configuration_version: u64,
    configuration_digest: String,
    owner: String,
    generation: u64,
    prior_head: u64,
    run_version: u64,
    materialized_at: i64,
    clusters: Vec<ChainCluster>,
    isolated: Vec<TopStock>,
}

struct LoadedApplication {
    run_id: String,
    context_digest: String,
    input_digest: String,
    business_date: String,
    material_version: u64,
    material_digest: String,
    bytes: Vec<u8>,
    owner: String,
    generation: u64,
    run_version: u64,
    applied_at: i64,
    rows: Vec<LifecycleRow>,
    days: BTreeMap<String, i64>,
}

pub(crate) struct ClusterApplicationRecovery {
    configuration: FixedClusterConfiguration,
    clusters: Vec<ChainCluster>,
    isolated: Vec<TopStock>,
    lifecycle_days: BTreeMap<String, i64>,
    material_bytes: Vec<u8>,
    lifecycle_bytes: Vec<u8>,
}

pub(super) struct BoardParentFact {
    pub(super) run_id: String,
    pub(super) context_digest: String,
    pub(super) input_digest: String,
    pub(super) application_version: u64,
    pub(super) lifecycle_digest: String,
    pub(super) material_version: u64,
    pub(super) material_digest: String,
    pub(super) clusters: Vec<ChainCluster>,
    pub(super) application_owner: String,
    pub(super) application_generation: u64,
    pub(super) applied_at: i64,
}

impl ClusterApplicationRecovery {
    pub(crate) fn min_cluster_size(&self) -> usize {
        self.configuration.min_cluster_size()
    }

    pub(crate) fn clusters(&self) -> &[ChainCluster] {
        &self.clusters
    }

    pub(crate) fn isolated(&self) -> &[TopStock] {
        &self.isolated
    }

    pub(crate) fn lifecycle_days(&self) -> &BTreeMap<String, i64> {
        &self.lifecycle_days
    }

    pub(crate) fn material_bytes(&self) -> &[u8] {
        &self.material_bytes
    }

    pub(crate) fn lifecycle_bytes(&self) -> &[u8] {
        &self.lifecycle_bytes
    }
}

impl LocalChainPostClose<'_> {
    pub(super) fn fix_cluster_configuration(
        &mut self,
        mut lease: RunLease,
        configuration: FixedClusterConfiguration,
        now: UtcMicros,
    ) -> Result<RunLease, ChainPostCloseError> {
        let bytes = configuration_bytes(configuration)?;
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        verify_cluster_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        validate_run_lease(&transaction, &recovery, &lease, now)?;
        validate_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        if let Some(saved) = load_configuration(&transaction, &lease.intent_id)? {
            validate_configuration(&saved, &recovery)?;
            if saved.bytes != bytes || saved.value != configuration {
                return Err(ChainPostCloseError::InvalidInput {
                    check: "cluster configuration conflict",
                });
            }
            transaction.commit().map_err(|_| storage("commit"))?;
            return Ok(lease);
        }

        let context_digest = recovery.context.canonical_sha256();
        let input_bytes = recovery.input.encode()?;
        let input_digest = raw_digest(&input_bytes);
        let digest = raw_digest(&bytes);
        let previous = lease.head;
        lease.head = previous
            .checked_add(1)
            .ok_or_else(|| storage("head overflow"))?;
        advance_run(&transaction, &lease, previous, now, "configuration cas")?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_cluster_configurations( \
                 intent_id,run_id,run_context_sha256,input_sha256,configuration_codec_version, \
                 configuration_bytes,configuration_length,configuration_sha256,lease_owner, \
                 lease_generation,prior_head_version,run_version,fixed_at) \
                 VALUES(?1,?2,?3,?4,1,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    lease.intent_id.as_str(),
                    lease.run_id.as_str(),
                    context_digest.as_str(),
                    input_digest.as_str(),
                    &bytes,
                    i64::try_from(bytes.len()).map_err(|_| storage("configuration length"))?,
                    digest.as_str(),
                    lease.owner.as_str(),
                    lease.generation,
                    previous,
                    lease.head,
                    now.get(),
                ],
            )
            .map_err(|_| storage("configuration fact"))?;
        validate_fact_versions(&transaction, &lease.intent_id, lease.head)?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(lease)
    }

    pub(super) fn load_cluster_material(
        &mut self,
        lease: &RunLease,
        concepts: &HashMap<String, Vec<String>>,
    ) -> Result<Option<(Vec<ChainCluster>, Vec<TopStock>)>, ChainPostCloseError> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        verify_cluster_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        validate_lease_identity(&recovery, lease)?;
        validate_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        let configuration = load_configuration(&transaction, &lease.intent_id)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        validate_configuration(&configuration, &recovery)?;
        let material = load_material(&transaction, &lease.intent_id)?;
        let result = material
            .map(|material| {
                validate_material(&material, &configuration, &recovery, concepts)?;
                Ok((material.clusters, material.isolated))
            })
            .transpose()?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(result)
    }

    pub(super) fn record_cluster_material(
        &mut self,
        mut lease: RunLease,
        concepts: &HashMap<String, Vec<String>>,
        clusters: &[ChainCluster],
        isolated: &[TopStock],
        now: UtcMicros,
    ) -> Result<RunLease, ChainPostCloseError> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        verify_cluster_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        validate_run_lease(&transaction, &recovery, &lease, now)?;
        validate_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        if load_material(&transaction, &lease.intent_id)?.is_some() {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let inspection =
            inspect_concept_batch_from_recovery_on(&transaction, &lease.intent_id, &recovery)?;
        if !inspection.is_complete() || inspection.concepts() != concepts {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let configuration = load_configuration(&transaction, &lease.intent_id)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        validate_configuration(&configuration, &recovery)?;
        validate_cluster_partition(&recovery, concepts, clusters, isolated)?;
        let concept_bytes = concept_map_bytes(concepts)?;
        let material_bytes = material_bytes(configuration.value, clusters, isolated)?;
        let context_digest = recovery.context.canonical_sha256();
        let input_digest = raw_digest(&recovery.input.encode()?);
        let concept_digest = raw_digest(&concept_bytes);
        let material_digest = raw_digest(&material_bytes);
        let previous = lease.head;
        lease.head = previous
            .checked_add(1)
            .ok_or_else(|| storage("head overflow"))?;
        advance_run(&transaction, &lease, previous, now, "material cas")?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_cluster_materials( \
                 intent_id,run_id,run_context_sha256,input_sha256,concept_map_codec_version, \
                 concept_map_bytes,concept_map_length,concept_map_sha256, \
                 concept_state_through_head_version,configuration_run_version, \
                 configuration_sha256,material_codec_version,material_bytes,material_length, \
                 material_sha256,lease_owner,lease_generation,prior_head_version,run_version, \
                 materialized_at) VALUES(?1,?2,?3,?4,1,?5,?6,?7,?8,?9,?10,1,?11,?12,?13, \
                 ?14,?15,?16,?17,?18)",
                params![
                    lease.intent_id.as_str(),
                    lease.run_id.as_str(),
                    context_digest.as_str(),
                    input_digest.as_str(),
                    &concept_bytes,
                    i64::try_from(concept_bytes.len())
                        .map_err(|_| storage("concept map length"))?,
                    concept_digest.as_str(),
                    previous,
                    configuration.run_version,
                    configuration.digest,
                    &material_bytes,
                    i64::try_from(material_bytes.len()).map_err(|_| storage("material length"))?,
                    material_digest.as_str(),
                    lease.owner.as_str(),
                    lease.generation,
                    previous,
                    lease.head,
                    now.get(),
                ],
            )
            .map_err(|_| storage("material fact"))?;
        validate_fact_versions(&transaction, &lease.intent_id, lease.head)?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(lease)
    }

    pub(super) fn apply_chain_daily(
        &mut self,
        mut lease: RunLease,
        date: NaiveDate,
        rows: &[(String, Vec<String>, i32)],
        now: UtcMicros,
    ) -> Result<(RunLease, HashMap<String, i64>), ChainPostCloseError> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("begin"))?;
        verify_cluster_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, &lease.intent_id)?;
        validate_run_lease(&transaction, &recovery, &lease, now)?;
        validate_fact_versions(&transaction, &lease.intent_id, recovery.head)?;
        let configuration = load_configuration(&transaction, &lease.intent_id)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        validate_configuration(&configuration, &recovery)?;
        let material = load_material(&transaction, &lease.intent_id)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let inspection =
            inspect_concept_batch_from_recovery_on(&transaction, &lease.intent_id, &recovery)?;
        if !inspection.is_complete() {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        validate_material(&material, &configuration, &recovery, inspection.concepts())?;
        validate_application_rows(&material.clusters, rows)?;
        if let Some(application) = load_application(&transaction, &lease.intent_id)? {
            validate_application(&application, &material, &recovery)?;
            transaction.commit().map_err(|_| storage("commit"))?;
            return Ok((lease, application.days.into_iter().collect()));
        }
        if recovery.input.business_date() != date.format("%Y-%m-%d").to_string() {
            return Err(ChainPostCloseError::InvalidInput {
                check: "cluster business date",
            });
        }
        for (concept, codes, continuation_count) in rows {
            let stocks = serde_json::to_string(codes).map_err(|_| storage("chain_daily codec"))?;
            transaction
                .execute(
                    "INSERT OR REPLACE INTO chain_daily(date,concept,stocks,continuation_count) \
                     VALUES(?1,?2,?3,?4)",
                    params![
                        date.format("%Y-%m-%d").to_string(),
                        concept,
                        stocks,
                        continuation_count
                    ],
                )
                .map_err(|_| storage("chain_daily write"))?;
        }
        let cutoff = date.checked_sub_signed(Duration::days(9)).ok_or(
            ChainPostCloseError::InvalidInput {
                check: "cluster lifecycle window",
            },
        )?;
        let mut days = BTreeMap::new();
        for (concept, _, _) in rows {
            let count = transaction
                .query_row(
                    "SELECT COUNT(DISTINCT date) FROM chain_daily \
                     WHERE concept=?1 AND date>=?2 AND date<=?3",
                    params![
                        concept,
                        cutoff.format("%Y-%m-%d").to_string(),
                        date.format("%Y-%m-%d").to_string(),
                    ],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|_| storage("chain_daily lifecycle"))?;
            days.insert(concept.clone(), count);
        }
        let lifecycle_rows = rows
            .iter()
            .map(|(concept, stocks, continuation_count)| LifecycleRow {
                concept: concept.clone(),
                stocks: stocks.clone(),
                continuation_count: *continuation_count,
            })
            .collect::<Vec<_>>();
        let lifecycle_bytes = lifecycle_bytes(date, &lifecycle_rows, &days)?;
        let lifecycle_digest = raw_digest(&lifecycle_bytes);
        let context_digest = recovery.context.canonical_sha256();
        let input_digest = raw_digest(&recovery.input.encode()?);
        let previous = lease.head;
        lease.head = previous
            .checked_add(1)
            .ok_or_else(|| storage("head overflow"))?;
        advance_run(&transaction, &lease, previous, now, "application cas")?;
        transaction
            .execute(
                "INSERT INTO chain_post_close_chain_daily_applications( \
                 intent_id,run_id,run_context_sha256,input_sha256,business_date, \
                 material_run_version,material_sha256,lifecycle_codec_version,lifecycle_bytes, \
                 lifecycle_length,lifecycle_sha256,lease_owner,lease_generation, \
                 prior_head_version,run_version,applied_at) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,1,?8,?9,?10,?11,?12,?13,?14,?15)",
                params![
                    lease.intent_id.as_str(),
                    lease.run_id.as_str(),
                    context_digest.as_str(),
                    input_digest.as_str(),
                    date.format("%Y-%m-%d").to_string(),
                    material.run_version,
                    material.digest,
                    &lifecycle_bytes,
                    i64::try_from(lifecycle_bytes.len())
                        .map_err(|_| storage("lifecycle length"))?,
                    lifecycle_digest.as_str(),
                    lease.owner.as_str(),
                    lease.generation,
                    previous,
                    lease.head,
                    now.get(),
                ],
            )
            .map_err(|_| storage("application fact"))?;
        validate_fact_versions(&transaction, &lease.intent_id, lease.head)?;
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok((lease, days.into_iter().collect()))
    }

    pub(crate) fn inspect_cluster_application(
        &mut self,
        intent_id: &IntentId,
    ) -> Result<ClusterApplicationRecovery, ChainPostCloseError> {
        let transaction = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("begin"))?;
        verify_cluster_layout(&transaction)?;
        let recovery = inspect_run_on(&transaction, intent_id)?;
        let inspection =
            inspect_concept_batch_from_recovery_on(&transaction, intent_id, &recovery)?;
        if !inspection.is_complete() {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        let configuration = load_configuration(&transaction, intent_id)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        validate_configuration(&configuration, &recovery)?;
        let material =
            load_material(&transaction, intent_id)?.ok_or(ChainPostCloseError::SchemaRejected)?;
        validate_material(&material, &configuration, &recovery, inspection.concepts())?;
        let application = load_application(&transaction, intent_id)?
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        validate_application(&application, &material, &recovery)?;
        validate_fact_versions(&transaction, intent_id, recovery.head)?;
        let mut clusters = material.clusters;
        for cluster in &mut clusters {
            cluster.streak_days = *application
                .days
                .get(&cluster.concept)
                .ok_or(ChainPostCloseError::SchemaRejected)?;
        }
        let result = ClusterApplicationRecovery {
            configuration: configuration.value,
            clusters,
            isolated: material.isolated,
            lifecycle_days: application.days,
            material_bytes: material.bytes,
            lifecycle_bytes: application.bytes,
        };
        transaction.commit().map_err(|_| storage("commit"))?;
        Ok(result)
    }
}

pub(super) fn load_board_parent(
    connection: &Transaction<'_>,
    intent_id: &IntentId,
    recovery: &RunRecovery,
) -> Result<BoardParentFact, ChainPostCloseError> {
    let inspection = inspect_concept_batch_from_recovery_on(connection, intent_id, recovery)?;
    if !inspection.is_complete() {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let configuration =
        load_configuration(connection, intent_id)?.ok_or(ChainPostCloseError::SchemaRejected)?;
    validate_configuration(&configuration, recovery)?;
    let material =
        load_material(connection, intent_id)?.ok_or(ChainPostCloseError::SchemaRejected)?;
    validate_material(&material, &configuration, recovery, inspection.concepts())?;
    let application =
        load_application(connection, intent_id)?.ok_or(ChainPostCloseError::SchemaRejected)?;
    validate_application(&application, &material, recovery)?;
    validate_fact_versions(connection, intent_id, recovery.head)?;
    Ok(BoardParentFact {
        run_id: application.run_id,
        context_digest: application.context_digest,
        input_digest: application.input_digest,
        application_version: application.run_version,
        lifecycle_digest: raw_digest(&application.bytes).as_str().to_owned(),
        material_version: material.run_version,
        material_digest: material.digest,
        clusters: material.clusters,
        application_owner: application.owner,
        application_generation: application.generation,
        applied_at: application.applied_at,
    })
}

pub(super) fn validate_existing_cluster_facts_at_layout(
    connection: &Transaction<'_>,
    intent_id: &IntentId,
    recovery: &RunRecovery,
    inspection: &super::ConceptBatchInspection,
    layout_version: i64,
) -> Result<Option<BoardParentFact>, ChainPostCloseError> {
    validate_existing_cluster_facts_scoped(
        connection,
        intent_id,
        recovery,
        inspection,
        layout_version,
        None,
    )
}

pub(super) fn validate_existing_cluster_facts_scoped(
    connection: &Transaction<'_>,
    intent_id: &IntentId,
    recovery: &RunRecovery,
    inspection: &super::ConceptBatchInspection,
    layout_version: i64,
    proof: Option<&schema::V12CatalogProof<'_, '_>>,
) -> Result<Option<BoardParentFact>, ChainPostCloseError> {
    if proof.is_some() && layout_version < 12 {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    if layout_version >= 12 {
        schema::verify_parent_layout_v12_scoped(connection, proof)?;
    } else if !matches!(layout_version, 6 | 7 | 8 | 9 | 10 | 11) {
        return Err(ChainPostCloseError::UnsupportedVersion);
    }
    let configuration = load_configuration(connection, intent_id)?;
    let material = load_material(connection, intent_id)?;
    let application = load_application(connection, intent_id)?;
    match (
        configuration.as_ref(),
        material.as_ref(),
        application.as_ref(),
    ) {
        (None, None, None) => Ok(None),
        (Some(configuration), None, None) => {
            validate_configuration(configuration, recovery)?;
            Ok(None)
        }
        (Some(configuration), Some(material), None) => {
            validate_configuration(configuration, recovery)?;
            if !inspection.is_complete() {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            validate_material(material, configuration, recovery, inspection.concepts())?;
            Ok(None)
        }
        (Some(configuration), Some(material), Some(application)) => {
            validate_configuration(configuration, recovery)?;
            if !inspection.is_complete() {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            validate_material(material, configuration, recovery, inspection.concepts())?;
            validate_application(application, material, recovery)?;
            Ok(Some(BoardParentFact {
                run_id: application.run_id.clone(),
                context_digest: application.context_digest.clone(),
                input_digest: application.input_digest.clone(),
                application_version: application.run_version,
                lifecycle_digest: raw_digest(&application.bytes).as_str().to_owned(),
                material_version: material.run_version,
                material_digest: material.digest.clone(),
                clusters: material.clusters.clone(),
                application_owner: application.owner.clone(),
                application_generation: application.generation,
                applied_at: application.applied_at,
            }))
        }
        _ => Err(ChainPostCloseError::SchemaRejected),
    }
}

fn verify_cluster_layout(connection: &Connection) -> Result<(), ChainPostCloseError> {
    match schema::runtime_layout_version(connection)? {
        // The unversioned facade has already attested the exact catalog.
        4..=13 => Ok(()),
        _ => Err(ChainPostCloseError::UnsupportedVersion),
    }
}

fn validate_run_lease(
    connection: &Connection,
    recovery: &RunRecovery,
    lease: &RunLease,
    now: UtcMicros,
) -> Result<(), ChainPostCloseError> {
    validate_lease_identity(recovery, lease)?;
    check_lease(connection, lease, now)
}

fn validate_lease_identity(
    recovery: &RunRecovery,
    lease: &RunLease,
) -> Result<(), ChainPostCloseError> {
    if recovery.input.encode()? != lease.input.encode()?
        || recovery.context.run_id() != &lease.run_id
        || recovery.generation != lease.generation
        || recovery.head != lease.head
    {
        return Err(ChainPostCloseError::StaleLease {
            intent_id: lease.intent_id.as_str().to_owned(),
        });
    }
    Ok(())
}

fn advance_run(
    connection: &Connection,
    lease: &RunLease,
    previous: u64,
    now: UtcMicros,
    operation: &'static str,
) -> Result<(), ChainPostCloseError> {
    let changed = connection
        .execute(
            "UPDATE chain_post_close_runs SET head_version=?1,updated_at=?2 \
             WHERE intent_id=?3 AND run_id=?4 AND lease_owner=?5 AND lease_generation=?6 \
               AND head_version=?7 AND lease_until>?2",
            params![
                lease.head,
                now.get(),
                lease.intent_id.as_str(),
                lease.run_id.as_str(),
                lease.owner.as_str(),
                lease.generation,
                previous,
            ],
        )
        .map_err(|_| storage(operation))?;
    if changed != 1 {
        return Err(ChainPostCloseError::StaleLease {
            intent_id: lease.intent_id.as_str().to_owned(),
        });
    }
    Ok(())
}

fn configuration_bytes(
    configuration: FixedClusterConfiguration,
) -> Result<Vec<u8>, ChainPostCloseError> {
    serde_json::to_vec(&ConfigurationEnvelope {
        schema_version: 1,
        min_cluster_size: configuration.min_cluster_size().to_string(),
    })
    .map_err(|_| storage("configuration codec"))
}

fn decode_configuration(bytes: &[u8]) -> Result<FixedClusterConfiguration, ChainPostCloseError> {
    let envelope: ConfigurationEnvelope =
        serde_json::from_slice(bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let size = envelope
        .min_cluster_size
        .parse::<usize>()
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let value = FixedClusterConfiguration::resolve(Some(&size.to_string()));
    if envelope.schema_version != 1 || configuration_bytes(value)?.as_slice() != bytes {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(value)
}

fn concept_map_bytes(
    concepts: &HashMap<String, Vec<String>>,
) -> Result<Vec<u8>, ChainPostCloseError> {
    serde_json::to_vec(&ConceptMapEnvelope {
        schema_version: 1,
        concepts: concepts.clone().into_iter().collect(),
    })
    .map_err(|_| storage("concept map codec"))
}

fn decode_concept_map(bytes: &[u8]) -> Result<HashMap<String, Vec<String>>, ChainPostCloseError> {
    let envelope: ConceptMapEnvelope =
        serde_json::from_slice(bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let concepts = envelope.concepts.into_iter().collect::<HashMap<_, _>>();
    if envelope.schema_version != 1 || concept_map_bytes(&concepts)?.as_slice() != bytes {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(concepts)
}

fn material_bytes(
    configuration: FixedClusterConfiguration,
    clusters: &[ChainCluster],
    isolated: &[TopStock],
) -> Result<Vec<u8>, ChainPostCloseError> {
    if clusters
        .iter()
        .flat_map(|cluster| cluster.stocks.iter())
        .chain(isolated.iter())
        .any(|stock| !stock_is_finite(stock))
    {
        return Err(ChainPostCloseError::InvalidInput {
            check: "cluster material number",
        });
    }
    serde_json::to_vec(&MaterialEnvelope {
        schema_version: 1,
        min_cluster_size: configuration.min_cluster_size().to_string(),
        clusters: clusters.to_vec(),
        isolated: isolated.to_vec(),
    })
    .map_err(|_| storage("material codec"))
}

fn decode_material(
    bytes: &[u8],
) -> Result<(FixedClusterConfiguration, Vec<ChainCluster>, Vec<TopStock>), ChainPostCloseError> {
    let envelope: MaterialEnvelope =
        serde_json::from_slice(bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let value = FixedClusterConfiguration::resolve(Some(&envelope.min_cluster_size));
    if envelope.schema_version != 1
        || value.min_cluster_size().to_string() != envelope.min_cluster_size
        || material_bytes(value, &envelope.clusters, &envelope.isolated)?.as_slice() != bytes
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok((value, envelope.clusters, envelope.isolated))
}

fn lifecycle_bytes(
    date: NaiveDate,
    rows: &[LifecycleRow],
    days: &BTreeMap<String, i64>,
) -> Result<Vec<u8>, ChainPostCloseError> {
    serde_json::to_vec(&LifecycleEnvelope {
        schema_version: 1,
        business_date: date.format("%Y-%m-%d").to_string(),
        rows: rows.to_vec(),
        days: days.clone(),
    })
    .map_err(|_| storage("lifecycle codec"))
}

fn decode_lifecycle(bytes: &[u8]) -> Result<LifecycleEnvelope, ChainPostCloseError> {
    let envelope: LifecycleEnvelope =
        serde_json::from_slice(bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let date = NaiveDate::parse_from_str(&envelope.business_date, "%Y-%m-%d")
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if envelope.schema_version != 1
        || lifecycle_bytes(date, &envelope.rows, &envelope.days)?.as_slice() != bytes
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(envelope)
}

fn load_configuration(
    connection: &Connection,
    intent_id: &IntentId,
) -> Result<Option<LoadedConfiguration>, ChainPostCloseError> {
    let row = connection
        .query_row(
            "SELECT run_id,run_context_sha256,input_sha256,configuration_codec_version, \
                    configuration_bytes,configuration_length,configuration_sha256,lease_owner, \
                    lease_generation,run_version,fixed_at \
             FROM chain_post_close_cluster_configurations WHERE intent_id=?1",
            [intent_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("configuration read"))?;
    row.map(
        |(
            run_id,
            context_digest,
            input_digest,
            codec,
            bytes,
            length,
            digest,
            owner,
            generation,
            version,
            at,
        )| {
            if codec != 1
                || length != i64::try_from(bytes.len()).unwrap_or(-1)
                || digest != raw_digest(&bytes).as_str()
                || generation < 1
                || version < 1
                || at < 0
            {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            let value = decode_configuration(&bytes)?;
            Ok(LoadedConfiguration {
                run_id,
                context_digest,
                input_digest,
                bytes,
                digest,
                owner,
                generation: u64::try_from(generation)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                run_version: u64::try_from(version)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                fixed_at: at,
                value,
            })
        },
    )
    .transpose()
}

fn validate_configuration(
    loaded: &LoadedConfiguration,
    recovery: &RunRecovery,
) -> Result<(), ChainPostCloseError> {
    if loaded.run_id != recovery.context.run_id().as_str()
        || loaded.context_digest != recovery.context.canonical_sha256().as_str()
        || loaded.input_digest != raw_digest(&recovery.input.encode()?).as_str()
        || loaded.digest != raw_digest(&loaded.bytes).as_str()
        || loaded.generation > recovery.generation
        || (loaded.generation == recovery.generation && loaded.owner != recovery.owner)
        || loaded.run_version > recovery.head
        || loaded.fixed_at > recovery.updated_at
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}

fn load_material(
    connection: &Connection,
    intent_id: &IntentId,
) -> Result<Option<LoadedMaterial>, ChainPostCloseError> {
    type Row = (
        String,
        String,
        String,
        i64,
        Vec<u8>,
        i64,
        String,
        i64,
        i64,
        String,
        i64,
        Vec<u8>,
        i64,
        String,
        String,
        i64,
        i64,
        i64,
        i64,
    );
    let row: Option<Row> = connection
        .query_row(
            "SELECT run_id,run_context_sha256,input_sha256,concept_map_codec_version, \
                concept_map_bytes,concept_map_length,concept_map_sha256, \
                concept_state_through_head_version,configuration_run_version,configuration_sha256, \
                material_codec_version,material_bytes,material_length,material_sha256,lease_owner, \
                lease_generation,prior_head_version,run_version,materialized_at \
         FROM chain_post_close_cluster_materials WHERE intent_id=?1",
            [intent_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                    row.get(13)?,
                    row.get(14)?,
                    row.get(15)?,
                    row.get(16)?,
                    row.get(17)?,
                    row.get(18)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage("material read"))?;
    row.map(
        |(
            run_id,
            context_digest,
            input_digest,
            concept_codec,
            concept_bytes,
            concept_length,
            concept_digest,
            concept_head,
            config_version,
            config_digest,
            material_codec,
            bytes,
            length,
            digest,
            owner,
            generation,
            prior,
            version,
            at,
        )| {
            let (configuration, clusters, isolated) = decode_material(&bytes)?;
            if concept_codec != 1
                || material_codec != 1
                || concept_length != i64::try_from(concept_bytes.len()).unwrap_or(-1)
                || concept_digest != raw_digest(&concept_bytes).as_str()
                || length != i64::try_from(bytes.len()).unwrap_or(-1)
                || digest != raw_digest(&bytes).as_str()
                || concept_head != prior
                || version != prior.checked_add(1).unwrap_or(-1)
                || config_version < 1
                || config_version >= version
                || generation < 1
                || at < 0
            {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            let _ = configuration;
            Ok(LoadedMaterial {
                run_id,
                context_digest,
                input_digest,
                concept_bytes,
                bytes,
                digest,
                configuration_version: u64::try_from(config_version)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                configuration_digest: config_digest,
                owner,
                generation: u64::try_from(generation)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                prior_head: u64::try_from(prior)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                run_version: u64::try_from(version)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                materialized_at: at,
                clusters,
                isolated,
            })
        },
    )
    .transpose()
}

fn validate_material(
    material: &LoadedMaterial,
    configuration: &LoadedConfiguration,
    recovery: &RunRecovery,
    concepts: &HashMap<String, Vec<String>>,
) -> Result<(), ChainPostCloseError> {
    let (encoded_configuration, _, _) = decode_material(&material.bytes)?;
    if material.run_id != recovery.context.run_id().as_str()
        || material.context_digest != recovery.context.canonical_sha256().as_str()
        || material.input_digest != raw_digest(&recovery.input.encode()?).as_str()
        || material.digest != raw_digest(&material.bytes).as_str()
        || decode_concept_map(&material.concept_bytes)? != *concepts
        || encoded_configuration != configuration.value
        || material.configuration_version != configuration.run_version
        || material.configuration_digest != configuration.digest
        || !child_authority_is_consistent(
            configuration.generation,
            &configuration.owner,
            material.generation,
            &material.owner,
        )
        || material.prior_head + 1 != material.run_version
        || material.generation > recovery.generation
        || (material.generation == recovery.generation && material.owner != recovery.owner)
        || material.run_version > recovery.head
        || material.materialized_at < configuration.fixed_at
        || material.materialized_at > recovery.updated_at
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    validate_cluster_partition(recovery, concepts, &material.clusters, &material.isolated)
}

fn load_application(
    connection: &Connection,
    intent_id: &IntentId,
) -> Result<Option<LoadedApplication>, ChainPostCloseError> {
    type Row = (
        String,
        String,
        String,
        String,
        i64,
        String,
        i64,
        Vec<u8>,
        i64,
        String,
        String,
        i64,
        i64,
        i64,
        i64,
    );
    let row: Option<Row> = connection.query_row(
        "SELECT run_id,run_context_sha256,input_sha256,business_date,material_run_version, \
                material_sha256,lifecycle_codec_version,lifecycle_bytes,lifecycle_length, \
                lifecycle_sha256,lease_owner,lease_generation,run_version,applied_at,prior_head_version \
         FROM chain_post_close_chain_daily_applications WHERE intent_id=?1",
        [intent_id.as_str()],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?,row.get(10)?,row.get(11)?,row.get(12)?,row.get(13)?,row.get(14)?)),
    ).optional().map_err(|_| storage("application read"))?;
    row.map(
        |(
            run_id,
            context_digest,
            input_digest,
            date,
            material_version,
            material_digest,
            codec,
            bytes,
            length,
            digest,
            owner,
            generation,
            version,
            at,
            prior,
        )| {
            let envelope = decode_lifecycle(&bytes)?;
            if codec != 1
                || envelope.business_date != date
                || length != i64::try_from(bytes.len()).unwrap_or(-1)
                || digest != raw_digest(&bytes).as_str()
                || version != prior.checked_add(1).unwrap_or(-1)
                || material_version < 1
                || material_version >= version
                || generation < 1
                || at < 0
            {
                return Err(ChainPostCloseError::SchemaRejected);
            }
            Ok(LoadedApplication {
                run_id,
                context_digest,
                input_digest,
                business_date: date,
                material_version: u64::try_from(material_version)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                material_digest,
                bytes,
                owner,
                generation: u64::try_from(generation)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                run_version: u64::try_from(version)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                applied_at: at,
                rows: envelope.rows,
                days: envelope.days,
            })
        },
    )
    .transpose()
}

fn validate_application(
    application: &LoadedApplication,
    material: &LoadedMaterial,
    recovery: &RunRecovery,
) -> Result<(), ChainPostCloseError> {
    if application.run_id != recovery.context.run_id().as_str()
        || application.context_digest != recovery.context.canonical_sha256().as_str()
        || application.input_digest != raw_digest(&recovery.input.encode()?).as_str()
        || application.business_date != recovery.input.business_date()
        || application.material_version != material.run_version
        || application.material_digest != material.digest
        || !child_authority_is_consistent(
            material.generation,
            &material.owner,
            application.generation,
            &application.owner,
        )
        || application.generation > recovery.generation
        || (application.generation == recovery.generation && application.owner != recovery.owner)
        || application.run_version > recovery.head
        || application.applied_at < material.materialized_at
        || application.applied_at > recovery.updated_at
        || application.days.len() != material.clusters.len()
        || application.rows.len() != material.clusters.len()
        || application
            .rows
            .iter()
            .zip(&material.clusters)
            .any(|(row, cluster)| {
                row.concept != cluster.concept
                    || row
                        .stocks
                        .iter()
                        .map(String::as_str)
                        .ne(cluster.stocks.iter().map(|stock| stock.code.as_str()))
                    || i32::try_from(cluster.continuation_count).ok()
                        != Some(row.continuation_count)
            })
        || material
            .clusters
            .iter()
            .any(|cluster| !application.days.contains_key(&cluster.concept))
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}

fn child_authority_is_consistent(
    parent_generation: u64,
    parent_owner: &str,
    child_generation: u64,
    child_owner: &str,
) -> bool {
    child_generation > parent_generation
        || (child_generation == parent_generation && child_owner == parent_owner)
}

fn validate_cluster_partition(
    recovery: &RunRecovery,
    concepts: &HashMap<String, Vec<String>>,
    clusters: &[ChainCluster],
    isolated: &[TopStock],
) -> Result<(), ChainPostCloseError> {
    let expected = recovery
        .input
        .stocks()
        .iter()
        .map(|stock| (stock.code.as_str(), stock))
        .collect::<HashMap<_, _>>();
    let mut covered = HashSet::new();
    let mut clustered = HashSet::new();
    let invalid_cluster = clusters.iter().any(|cluster| {
        let mut members = HashSet::new();
        cluster.concept.trim().is_empty()
            || cluster.stocks.is_empty()
            || cluster.stocks.iter().any(|stock| {
                !members.insert(stock.code.as_str())
                    || !stock_matches(expected.get(stock.code.as_str()).copied(), stock)
                    || concepts
                        .get(&stock.code)
                        .is_none_or(|labels| !labels.iter().any(|label| label == &cluster.concept))
            })
            || cluster.aliases.iter().any(|alias| {
                let alias_members = expected
                    .keys()
                    .copied()
                    .filter(|code| {
                        concepts
                            .get(*code)
                            .is_some_and(|labels| labels.iter().any(|label| label == alias))
                    })
                    .collect::<HashSet<_>>();
                let overlap = cluster
                    .stocks
                    .iter()
                    .filter(|stock| alias_members.contains(stock.code.as_str()))
                    .count();
                alias.trim().is_empty()
                    || alias_members.is_empty()
                    || overlap * 10 < alias_members.len() * 7
            })
            || {
                covered.extend(cluster.stocks.iter().map(|stock| stock.code.as_str()));
                clustered.extend(cluster.stocks.iter().map(|stock| stock.code.as_str()));
                false
            }
    });
    let mut isolated_codes = HashSet::new();
    let invalid_isolated = isolated.iter().any(|stock| {
        !isolated_codes.insert(stock.code.as_str())
            || clustered.contains(stock.code.as_str())
            || !stock_matches(expected.get(stock.code.as_str()).copied(), stock)
            || {
                covered.insert(stock.code.as_str());
                false
            }
    });
    if expected.len() != recovery.input.stocks().len()
        || recovery
            .input
            .stocks()
            .iter()
            .any(|stock| !stock_is_finite(stock))
        || invalid_cluster
        || invalid_isolated
        || covered.len() != expected.len()
        || expected.keys().any(|code| !covered.contains(code))
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}

fn stock_matches(expected: Option<&TopStock>, actual: &TopStock) -> bool {
    expected.is_some_and(|expected| {
        serde_json::to_vec(expected).ok() == serde_json::to_vec(actual).ok()
            && stock_is_finite(actual)
    })
}

fn stock_is_finite(stock: &TopStock) -> bool {
    stock.change_pct.is_finite()
        && stock.price.is_finite()
        && stock.volume_ratio.is_none_or(f64::is_finite)
        && stock.main_net_yi.is_none_or(f64::is_finite)
}

fn validate_application_rows(
    clusters: &[ChainCluster],
    rows: &[(String, Vec<String>, i32)],
) -> Result<(), ChainPostCloseError> {
    if clusters.len() != rows.len()
        || clusters
            .iter()
            .zip(rows)
            .any(|(cluster, (concept, codes, count))| {
                cluster.concept != *concept
                    || cluster
                        .stocks
                        .iter()
                        .map(|stock| stock.code.as_str())
                        .ne(codes.iter().map(String::as_str))
                    || i32::try_from(cluster.continuation_count).ok() != Some(*count)
            })
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(())
}

fn validate_fact_versions(
    connection: &Connection,
    intent_id: &IntentId,
    head: u64,
) -> Result<(), ChainPostCloseError> {
    super::validate_run_fact_versions(connection, intent_id, head)
}
