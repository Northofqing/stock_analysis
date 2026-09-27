//! Append-only connection qualification facts in the existing Macro run chain.
//! Recorded identities prove historical facts, never live transport authority.
use super::{macro_codec as codec, macro_stage as old, storage, ChainPostCloseError, RunRecovery};
use crate::grpc_client::client::external_control_attempt::ExternalControlKind;
use crate::grpc_client::connection_qualification::ConnectionIdentity;
use crate::grpc_client::provider_attempts::ExternalProviderCatalog;
use crate::monitor::push_job::{raw_digest, IntentId};
use crate::search_service::macro_news::runner::QueryKey;
use codec::{require, Result};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(super) const TABLE: &str = "chain_post_close_macro_connection_facts";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HealthBegin {
    pub(super) version: u32,
    pub(super) identity: ConnectionIdentity,
    pub(super) plan_version: u64,
    pub(super) plan_sha256: String,
    pub(super) request: codec::ControlRequest,
    pub(super) control_begin_version: Option<u64>,
}

impl HealthBegin {
    pub(super) fn validate(&self) -> Result<()> {
        self.request.validate()?;
        require(
            self.version == 1
                && self.identity.validate_recorded()
                && self.plan_version > 0
                && self.plan_sha256.len() == 64
                && self.request.kind() == ExternalControlKind::Health
                && self
                    .control_begin_version
                    .is_none_or(|version| version > self.plan_version),
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HealthResult {
    pub(super) version: u32,
    pub(super) identity: ConnectionIdentity,
    pub(super) plan_version: u64,
    pub(super) plan_sha256: String,
    pub(super) begin_version: u64,
    pub(super) begin_sha256: String,
    pub(super) control_result_version: Option<u64>,
    pub(super) control_result_sha256: String,
    pub(super) raw: Option<codec::ControlRawResult>,
    pub(super) qualified: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapabilitiesBegin {
    pub(super) version: u32,
    pub(super) identity: ConnectionIdentity,
    pub(super) plan_version: u64,
    pub(super) plan_sha256: String,
    pub(super) request: codec::ControlRequest,
    pub(super) qualification_version: u64,
    pub(super) qualification_sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EffectLink {
    pub(super) version: u32,
    pub(super) identity: ConnectionIdentity,
    pub(super) plan_version: u64,
    pub(super) plan_sha256: String,
    pub(super) qualification_version: u64,
    pub(super) qualification_sha256: String,
    pub(super) effect_begin_version: u64,
    pub(super) request: codec::ControlRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) data: Option<DataEffect>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DataEffect {
    pub(super) query: QueryKey,
    pub(super) attempt: u32,
    pub(super) request_plan_version: u64,
    pub(super) request: codec::Request,
    pub(super) capabilities_begin_version: u64,
    pub(super) capabilities_result_version: u64,
    pub(super) capabilities_result_sha256: String,
}

pub(super) struct Capabilities {
    pub(super) request: codec::ControlRequest,
    pub(super) begin_version: u64,
    pub(super) result_version: u64,
    pub(super) result_sha256: String,
    time: i64,
    catalog: ExternalProviderCatalog,
}

#[derive(Default)]
pub(super) struct Recovery {
    pub(super) facts: Vec<old::Fact>,
    pub(super) begins: BTreeMap<String, (old::Fact, HealthBegin)>,
    pub(super) results: BTreeMap<String, (old::Fact, HealthResult)>,
    pub(super) capabilities: BTreeMap<String, Capabilities>,
    pub(super) capability_begins: BTreeMap<String, (old::Fact, CapabilitiesBegin)>,
    pub(super) health_responses: BTreeMap<String, Vec<u8>>,
    pub(super) data_catalogs: BTreeMap<u64, ExternalProviderCatalog>,
    pub(super) data_connections: BTreeMap<u64, ConnectionIdentity>,
    pub(super) pending: Vec<u64>,
}

impl Recovery {
    pub(super) fn qualified(
        &self,
        identity: &ConnectionIdentity,
    ) -> Result<(&old::Fact, &HealthResult)> {
        let (fact, result) = self
            .results
            .get(&identity.epoch)
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        require(result.qualified && &result.identity == identity)?;
        Ok((fact, result))
    }
}

/// Reads only in the same validated transaction as the original run/plan.
/// The typed projection checks duplicated columns as well as canonical bytes.
pub(super) fn load(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    run: &RunRecovery,
    plan: &codec::Plan,
    plan_version: u64,
    plan_sha256: &str,
) -> Result<Recovery> {
    let facts = old::facts(transaction, intent, TABLE)?;
    let mut recovered = Recovery::default();
    for fact in &facts {
        fact.validate(run)?;
        require(fact.time >= plan.started && fact.time < plan.deadline)?;
        let columns: (String,String,u64,String,Option<u64>,Option<String>,Option<u64>,Option<String>) = transaction.query_row(
            "SELECT kind,connection_epoch,plan_version,plan_sha256,predecessor_version,predecessor_sha256,effect_version,outcome FROM chain_post_close_macro_connection_facts WHERE intent_id=?1 AND run_version=?2",
            params![intent.as_str(),fact.version],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)),
        ).map_err(|_| storage("connection fact columns"))?;
        require(columns.2 == plan_version && columns.3 == plan_sha256)?;
        match columns.0.as_str() {
            "HealthBegin" => {
                let value: HealthBegin = codec::decode(&fact.bytes)?;
                value.validate()?;
                require(
                    value.plan_version == plan_version
                        && value.plan_sha256 == plan_sha256
                        && value
                            .control_begin_version
                            .is_none_or(|version| version == fact.version + 1)
                        && value.request.endpoint() == plan.endpoint()
                        && Some(value.request.authority()) == plan.request.authority.as_deref(),
                )?;
                require(
                    columns
                        == (
                            "HealthBegin".into(),
                            value.identity.epoch.clone(),
                            plan_version,
                            plan_sha256.into(),
                            None,
                            None,
                            None,
                            None,
                        ),
                )?;
                if let Some(version) = value.control_begin_version {
                    paired_begin(transaction, intent, fact, version, "Health", &value.request)?;
                } else {
                    recovered.pending.push(fact.version);
                }
                require(
                    recovered
                        .begins
                        .insert(value.identity.epoch.clone(), (fact.clone(), value))
                        .is_none(),
                )?;
            }
            "HealthResult" => {
                let value: HealthResult = codec::decode(&fact.bytes)?;
                let (begin_fact, begin) = recovered
                    .begins
                    .get(&value.identity.epoch)
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                require(
                    value.version == 1
                        && value.identity == begin.identity
                        && value.plan_version == plan_version
                        && value.plan_sha256 == plan_sha256
                        && value.begin_version == begin_fact.version
                        && value.begin_sha256 == begin_fact.digest
                        && fact.prior >= begin_fact.version
                        && fact.time >= begin_fact.time
                        && fact.owner == begin_fact.owner
                        && fact.generation == begin_fact.generation,
                )?;
                require(
                    columns
                        == (
                            "HealthResult".into(),
                            value.identity.epoch.clone(),
                            plan_version,
                            plan_sha256.into(),
                            Some(begin_fact.version),
                            Some(begin_fact.digest.clone()),
                            None,
                            Some(
                                if value.qualified {
                                    "Qualified"
                                } else {
                                    "Rejected"
                                }
                                .into(),
                            ),
                        ),
                )?;
                let raw_bytes: Vec<u8> = if let Some(control_begin) = begin.control_begin_version {
                    require(
                        value.raw.is_none()
                            && value
                                .control_result_version
                                .is_some_and(|version| version < fact.version),
                    )?;
                    transaction.query_row(
                    "SELECT bytes FROM chain_post_close_macro_control_attempt_results WHERE intent_id=?1 AND run_version=?2 AND begin_version=?3 AND kind='Health' AND lease_owner=?4 AND lease_generation=?5 AND recorded_at=?6 AND sha256=?7",
                    params![intent.as_str(), value.control_result_version, control_begin, fact.owner, fact.generation, fact.time, value.control_result_sha256],
                    |row| row.get(0),
                    ).map_err(|_| ChainPostCloseError::SchemaRejected)?
                } else {
                    require(value.control_result_version.is_none())?;
                    let index = recovered
                        .pending
                        .iter()
                        .position(|version| *version == begin_fact.version)
                        .ok_or(ChainPostCloseError::SchemaRejected)?;
                    recovered.pending.remove(index);
                    codec::encode(
                        value
                            .raw
                            .as_ref()
                            .ok_or(ChainPostCloseError::SchemaRejected)?,
                    )?
                };
                require(raw_digest(&raw_bytes).as_str() == value.control_result_sha256)?;
                let raw: codec::ControlRawResult = codec::decode(&raw_bytes)?;
                raw.validate_connection_binding(&value.identity, None)?;
                let qualified = raw.project(&begin.request)?.is_ok();
                let policy_qualified =
                    raw.response_bytes()
                        .and_then(|bytes| {
                            crate::grpc_client::external_decoder::ExternalDecoder::for_descriptor(
                                &value.identity.descriptor_sha256,
                            )
                            .and_then(|decoder| decoder.health(bytes))
                            .ok()
                        })
                        .is_some_and(|response| {
                            crate::grpc_client::build_identity::BuildIdentityTrust::bundled()
                                .is_ok_and(|trust| {
                                    response.request_id == begin.request.request_id()
                                        && trust
                                            .recorded_health(
                                                &value.identity.policy_sha256,
                                                &value.identity.descriptor_sha256,
                                                &response,
                                            )
                                            .is_ok()
                                })
                        });
                require(qualified == value.qualified && qualified == policy_qualified)?;
                if let Some(bytes) = raw.response_bytes() {
                    require(
                        recovered
                            .health_responses
                            .insert(value.identity.epoch.clone(), bytes.to_vec())
                            .is_none(),
                    )?;
                }
                require(
                    recovered
                        .results
                        .insert(value.identity.epoch.clone(), (fact.clone(), value))
                        .is_none(),
                )?;
            }
            "CapabilitiesBegin" => {
                let value: CapabilitiesBegin = codec::decode(&fact.bytes)?;
                value.request.validate()?;
                let (health_fact, _) = recovered.qualified(&value.identity)?;
                require(
                    value.version == 1
                        && value.plan_version == plan_version
                        && value.plan_sha256 == plan_sha256
                        && value.qualification_version == health_fact.version
                        && value.qualification_sha256 == health_fact.digest
                        && value.request.kind() == ExternalControlKind::Capabilities
                        && value.request.endpoint() == plan.endpoint()
                        && Some(value.request.authority()) == plan.request.authority.as_deref()
                        && fact.time >= health_fact.time
                        && fact.owner == health_fact.owner
                        && fact.generation == health_fact.generation
                        && fact.prior >= health_fact.version,
                )?;
                require(
                    columns
                        == (
                            "CapabilitiesBegin".into(),
                            value.identity.epoch.clone(),
                            plan_version,
                            plan_sha256.into(),
                            Some(health_fact.version),
                            Some(health_fact.digest.clone()),
                            None,
                            None,
                        ),
                )?;
                require(
                    !recovered.capabilities.contains_key(&value.identity.epoch)
                        && recovered
                            .capability_begins
                            .insert(value.identity.epoch.clone(), (fact.clone(), value))
                            .is_none(),
                )?;
                recovered.pending.push(fact.version);
            }
            "CapabilitiesResult" => {
                let value: HealthResult = codec::decode(&fact.bytes)?;
                let (begin_fact, begin) = recovered
                    .capability_begins
                    .get(&value.identity.epoch)
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                require(
                    value.version == 1
                        && value.identity == begin.identity
                        && value.plan_version == plan_version
                        && value.plan_sha256 == plan_sha256
                        && value.begin_version == begin_fact.version
                        && value.begin_sha256 == begin_fact.digest
                        && fact.prior >= begin_fact.version
                        && fact.time >= begin_fact.time
                        && fact.owner == begin_fact.owner
                        && fact.generation == begin_fact.generation
                        && value.control_result_version.is_none(),
                )?;
                require(
                    columns
                        == (
                            "CapabilitiesResult".into(),
                            value.identity.epoch.clone(),
                            plan_version,
                            plan_sha256.into(),
                            Some(begin_fact.version),
                            Some(begin_fact.digest.clone()),
                            None,
                            Some(
                                if value.qualified {
                                    "Qualified"
                                } else {
                                    "Rejected"
                                }
                                .into(),
                            ),
                        ),
                )?;
                let raw = value
                    .raw
                    .as_ref()
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                raw.validate_connection_binding(
                    &value.identity,
                    Some(
                        recovered
                            .health_responses
                            .get(&value.identity.epoch)
                            .ok_or(ChainPostCloseError::SchemaRejected)?,
                    ),
                )?;
                require(
                    raw_digest(&codec::encode(raw)?).as_str() == value.control_result_sha256
                        && raw.project(&begin.request)?.is_ok() == value.qualified,
                )?;
                let index = recovered
                    .pending
                    .iter()
                    .position(|version| *version == begin_fact.version)
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                recovered.pending.remove(index);
                if value.qualified {
                    require(
                        recovered
                            .capabilities
                            .insert(
                                value.identity.epoch.clone(),
                                Capabilities {
                                    request: begin.request.clone(),
                                    begin_version: begin_fact.version,
                                    result_version: fact.version,
                                    result_sha256: fact.digest.clone(),
                                    time: fact.time,
                                    catalog: raw.validated_provider_catalog(&begin.request)?,
                                },
                            )
                            .is_none(),
                    )?;
                }
            }
            "EffectLink" => {
                let value: EffectLink = codec::decode(&fact.bytes)?;
                let (result_fact, _) = recovered.qualified(&value.identity)?;
                value.request.validate()?;
                require(
                    value.version == 1
                        && value.plan_version == plan_version
                        && value.plan_sha256 == plan_sha256
                        && value.qualification_version == result_fact.version
                        && value.qualification_sha256 == result_fact.digest
                        && value.effect_begin_version == fact.version + 1
                        && value.request.kind() == ExternalControlKind::Capabilities
                        && value.request.endpoint() == plan.endpoint()
                        && Some(value.request.authority()) == plan.request.authority.as_deref()
                        && fact.time >= result_fact.time
                        && fact.owner == result_fact.owner
                        && fact.generation == result_fact.generation,
                )?;
                require(
                    columns
                        == (
                            "EffectLink".into(),
                            value.identity.epoch.clone(),
                            plan_version,
                            plan_sha256.into(),
                            Some(result_fact.version),
                            Some(result_fact.digest.clone()),
                            Some(value.effect_begin_version),
                            None,
                        ),
                )?;
                if let Some(data) = &value.data {
                    let capabilities = recovered
                        .capabilities
                        .get(&value.identity.epoch)
                        .ok_or(ChainPostCloseError::SchemaRejected)?;
                    require(
                        data.capabilities_begin_version == capabilities.begin_version
                            && data.capabilities_result_version == capabilities.result_version
                            && data.capabilities_result_sha256 == capabilities.result_sha256
                            && capabilities.result_version < fact.version
                            && capabilities.time <= fact.time
                            && codec::encode(&value.request)?
                                == codec::encode(&capabilities.request)?,
                    )?;
                    let identity = super::macro_plan_v3::definition(plan)?
                        .identity(data.query)
                        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
                    data.request.validate_for(&identity)?;
                    require(
                        data.request.contract_profile()
                            == crate::grpc_client::client::ContractProfile::ExternalV1,
                    )?;
                    let (phase, item, candidate) = super::macro_native::query_columns(data.query)?;
                    let paired: i64 = transaction.query_row(
                        "SELECT count(*) FROM chain_post_close_macro_attempt_begins WHERE intent_id=?1 AND run_version=?2 AND request_plan_version=?3 AND request_sha256=?4 AND phase=?5 AND item_ordinal=?6 AND candidate_ordinal=?7 AND attempt_ordinal=?8 AND lease_owner=?9 AND lease_generation=?10 AND recorded_at=?11",
                        params![intent.as_str(),value.effect_begin_version,data.request_plan_version,raw_digest(&data.request.bytes).as_str(),phase,item,candidate,data.attempt,fact.owner,fact.generation,fact.time],
                        |row| row.get(0),
                    ).map_err(|_| ChainPostCloseError::SchemaRejected)?;
                    require(
                        recovered
                            .data_connections
                            .insert(value.effect_begin_version, value.identity.clone())
                            .is_none(),
                    )?;
                    require(
                        paired == 1
                            && recovered
                                .data_catalogs
                                .insert(value.effect_begin_version, capabilities.catalog.clone())
                                .is_none(),
                    )?;
                } else {
                    paired_begin(
                        transaction,
                        intent,
                        fact,
                        value.effect_begin_version,
                        "Capabilities",
                        &value.request,
                    )?;
                    let result: Option<(u64,String,Vec<u8>,i64,String)> = transaction.query_row(
                        "SELECT run_version,sha256,bytes,recorded_at,outcome FROM chain_post_close_macro_control_attempt_results WHERE intent_id=?1 AND begin_version=?2 AND kind='Capabilities' AND lease_owner=?3 AND lease_generation=?4",
                        params![intent.as_str(),value.effect_begin_version,fact.owner,fact.generation],
                        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
                    ).optional().map_err(|_| ChainPostCloseError::SchemaRejected)?;
                    if let Some((version, digest, bytes, time, outcome)) = result {
                        require(
                            version > value.effect_begin_version
                                && time >= fact.time
                                && raw_digest(&bytes).as_str() == digest,
                        )?;
                        let raw: codec::ControlRawResult = codec::decode(&bytes)?;
                        raw.validate_connection_binding(
                            &value.identity,
                            Some(
                                recovered
                                    .health_responses
                                    .get(&value.identity.epoch)
                                    .ok_or(ChainPostCloseError::SchemaRejected)?,
                            ),
                        )?;
                        let ready = raw.project(&value.request)?.is_ok();
                        require((outcome == "Ready") == ready)?;
                        if ready {
                            let catalog = raw.validated_provider_catalog(&value.request)?;
                            require(
                                recovered
                                    .capabilities
                                    .insert(
                                        value.identity.epoch.clone(),
                                        Capabilities {
                                            request: value.request.clone(),
                                            begin_version: value.effect_begin_version,
                                            result_version: version,
                                            result_sha256: digest,
                                            time,
                                            catalog,
                                        },
                                    )
                                    .is_none(),
                            )?;
                        }
                    }
                }
            }
            _ => return Err(ChainPostCloseError::SchemaRejected),
        }
    }
    recovered.facts = facts;
    Ok(recovered)
}

fn paired_begin(
    transaction: &Transaction<'_>,
    intent: &IntentId,
    fact: &old::Fact,
    version: u64,
    kind: &str,
    request: &codec::ControlRequest,
) -> Result<()> {
    let paired: i64 = transaction.query_row(
        "SELECT count(*) FROM chain_post_close_macro_control_attempt_begins WHERE intent_id=?1 AND run_version=?2 AND kind=?3 AND request_sha256=?4 AND lease_owner=?5 AND lease_generation=?6 AND recorded_at=?7",
        params![intent.as_str(),version,kind,raw_digest(request.request_bytes()).as_str(),fact.owner,fact.generation,fact.time],
        |row| row.get(0),
    ).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    require(paired == 1)
}
