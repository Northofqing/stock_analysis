//! Strict restoration of embedded deployment-set candidates.
//!
//! Structural integrity is not source, owner, or binary authentication.

use serde_json::Value;

use crate::monitor::push_job::{
    CalendarId, GitSha40, Namespace, ProducerId, RunId, Sha256Digest, SourceContractId,
    SourceContractVersion, UnitId,
};

use super::{
    restore_activation_deployment_set, ActivationDeploymentSet, ActivationDeploymentSetDecodeError,
    CalendarDeclaration, SharedDependencyDeclaration, UnitDeploymentObservation,
};
use crate::push_foundation::activation::DesiredActivationState;
use crate::push_foundation::operational_readiness::DependencyKind;

pub(super) fn decode_activation_deployment_set(
    value: &Value,
    expected_sha256: &Sha256Digest,
) -> Result<ActivationDeploymentSet, ActivationDeploymentSetDecodeError> {
    let namespace = decode_namespace(field(value, "namespace")?)?;
    let calendar_value = field(value, "calendar")?;
    let calendar = CalendarDeclaration {
        calendar_id: CalendarId::try_new(string_field(calendar_value, "calendar_id")?.to_owned())
            .map_err(|_| invalid("calendar_id"))?,
        authority_sha256: digest_field(calendar_value, "authority_sha256")?,
        utc_offset_seconds: unsigned_field(calendar_value, "utc_offset_seconds")?,
    };
    let enabled_producers = array_field(value, "enabled_producers")?
        .iter()
        .map(|item| {
            ProducerId::try_new(
                item.as_str()
                    .ok_or(invalid("enabled_producers"))?
                    .to_owned(),
            )
            .map_err(|_| invalid("enabled_producers"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let recovery_units = array_field(value, "recovery_units")?
        .iter()
        .map(|item| {
            UnitId::try_new(item.as_str().ok_or(invalid("recovery_units"))?.to_owned())
                .map_err(|_| invalid("recovery_units"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let shared_dependencies = array_field(value, "shared_dependencies")?
        .iter()
        .map(decode_shared_dependency)
        .collect::<Result<Vec<_>, _>>()?;
    let units = array_field(value, "units")?
        .iter()
        .map(decode_unit)
        .collect::<Result<Vec<_>, _>>()?;
    if unsigned_field(value, "schema_version")? != 1 {
        return Err(invalid("schema_version"));
    }
    let set = restore_activation_deployment_set(
        namespace,
        digest_field(value, "catalog_sha256")?,
        calendar,
        enabled_producers,
        recovery_units,
        shared_dependencies,
        units,
    )?;
    if set.sha256() != expected_sha256 {
        return Err(ActivationDeploymentSetDecodeError::InconsistentSet);
    }
    Ok(set)
}

fn decode_shared_dependency(
    value: &Value,
) -> Result<SharedDependencyDeclaration, ActivationDeploymentSetDecodeError> {
    Ok(SharedDependencyDeclaration::new(
        dependency_kind(string_field(value, "dependency_kind")?)?,
        SourceContractId::try_new(string_field(value, "contract_id")?.to_owned())
            .map_err(|_| invalid("contract_id"))?,
        SourceContractVersion::try_new(string_field(value, "contract_version")?.to_owned())
            .map_err(|_| invalid("contract_version"))?,
        digest_field(value, "sha256")?,
    ))
}

fn decode_unit(
    value: &Value,
) -> Result<UnitDeploymentObservation, ActivationDeploymentSetDecodeError> {
    let unit_id = UnitId::try_new(string_field(value, "unit_id")?.to_owned())
        .map_err(|_| invalid("unit_id"))?;
    match string_field(value, "activation_status")? {
        "Unregistered" => {
            for key in [
                "generation",
                "manifest_sha256",
                "journal_event_id",
                "journal_sha256",
                "desired_state",
                "physical_owner",
                "build_commit",
                "build_sha256",
                "source_binding_sha256",
            ] {
                if !field(value, key)?.is_null() {
                    return Err(invalid(key));
                }
            }
            Ok(UnitDeploymentObservation::Unregistered { unit_id })
        }
        "CaughtUp" => {
            let build_commit = GitSha40::parse(string_field(value, "build_commit")?)
                .map_err(|_| invalid("build_commit"))?;
            let desired_state =
                DesiredActivationState::parse(string_field(value, "desired_state")?)
                    .ok_or(invalid("desired_state"))?;
            Ok(UnitDeploymentObservation::CaughtUp {
                unit_id,
                generation: unsigned_field(value, "generation")?,
                manifest_sha256: digest_field(value, "manifest_sha256")?,
                journal_event_id: digest_field(value, "journal_event_id")?,
                journal_sha256: digest_field(value, "journal_sha256")?,
                desired_state,
                physical_owner: string_field(value, "physical_owner")?.to_owned(),
                build_commit: build_commit.as_str().to_owned(),
                build_sha256: digest_field(value, "build_sha256")?,
                source_binding_sha256: digest_field(value, "source_binding_sha256")?,
            })
        }
        _ => Err(invalid("activation_status")),
    }
}

fn decode_namespace(value: &Value) -> Result<Namespace, ActivationDeploymentSetDecodeError> {
    match string_field(value, "kind")? {
        "Production" if field(value, "run_id")?.is_null() => Ok(Namespace::Production),
        "Test" => Ok(Namespace::test(
            RunId::try_new(string_field(value, "run_id")?.to_owned())
                .map_err(|_| invalid("run_id"))?,
        )),
        _ => Err(invalid("namespace")),
    }
}

fn dependency_kind(value: &str) -> Result<DependencyKind, ActivationDeploymentSetDecodeError> {
    use DependencyKind::*;
    [Namespace, Durable, Audit, TypedAuthority, Schema, Manifest]
        .into_iter()
        .find(|kind| kind.as_str() == value)
        .ok_or(invalid("dependency_kind"))
}

fn field<'a>(
    value: &'a Value,
    key: &'static str,
) -> Result<&'a Value, ActivationDeploymentSetDecodeError> {
    value.get(key).ok_or(invalid(key))
}

fn string_field<'a>(
    value: &'a Value,
    key: &'static str,
) -> Result<&'a str, ActivationDeploymentSetDecodeError> {
    field(value, key)?.as_str().ok_or(invalid(key))
}

fn unsigned_field(
    value: &Value,
    key: &'static str,
) -> Result<u64, ActivationDeploymentSetDecodeError> {
    field(value, key)?.as_u64().ok_or(invalid(key))
}

fn array_field<'a>(
    value: &'a Value,
    key: &'static str,
) -> Result<&'a [Value], ActivationDeploymentSetDecodeError> {
    field(value, key)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or(invalid(key))
}

fn digest_field(
    value: &Value,
    key: &'static str,
) -> Result<Sha256Digest, ActivationDeploymentSetDecodeError> {
    Sha256Digest::parse(key, string_field(value, key)?).map_err(|_| invalid(key))
}

fn invalid(field: &'static str) -> ActivationDeploymentSetDecodeError {
    ActivationDeploymentSetDecodeError::InvalidField { field }
}
