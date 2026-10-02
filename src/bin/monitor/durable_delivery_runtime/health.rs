//! Existing runtime cache only. This path must never build a runtime.

use super::*;
use stock_analysis::durable_delivery::{DeliveryStatusSnapshot, DurableDeliveryError};

pub(crate) enum CachedDeliveryObservation {
    NotInitialized,
    Observed(DeliveryStatusSnapshot),
    Unavailable(super::super::health_cmd::durable_delivery::Failure),
}

fn typed_failure(
    error: DurableDeliveryError,
) -> super::super::health_cmd::durable_delivery::Failure {
    use super::super::health_cmd::durable_delivery::Failure;
    match error {
        DurableDeliveryError::InvalidConfiguration(_) => Failure::InvalidConfiguration,
        DurableDeliveryError::IsolationViolation(_) => Failure::IsolationViolation,
        DurableDeliveryError::InvalidEnvelope(_) => Failure::InvalidEnvelope,
        DurableDeliveryError::PolicyMismatch(_) => Failure::PolicyMismatch,
        DurableDeliveryError::DecisionIdentityConflict { .. } => Failure::DecisionIdentityConflict,
        DurableDeliveryError::DecisionNotFound(_) => Failure::DecisionNotFound,
        DurableDeliveryError::IllegalTransition { .. } => Failure::IllegalTransition,
        DurableDeliveryError::InvalidManualResolution(_) => Failure::InvalidManualResolution,
        DurableDeliveryError::ImmutableAppendConflict(_) => Failure::ImmutableAppendConflict,
        DurableDeliveryError::AuditPredecessorBlocked => Failure::AuditPredecessorBlocked,
        DurableDeliveryError::Sqlite(_) => Failure::Sqlite,
        DurableDeliveryError::Serialization(_) => Failure::Serialization,
        DurableDeliveryError::Io(_) => Failure::Io,
    }
}

#[cfg(not(test))]
fn cached_state(namespace: &RuntimeNamespace) -> Result<Option<Arc<RuntimeState>>, ()> {
    let Some(entry) = RUNTIME.get() else {
        return Ok(None);
    };
    if &entry.namespace != namespace {
        return Err(());
    }
    entry
        .state
        .as_ref()
        .map(|state| Some(Arc::clone(state)))
        .map_err(|_| ())
}

#[cfg(test)]
fn cached_state(namespace: &RuntimeNamespace) -> Result<Option<Arc<RuntimeState>>, ()> {
    let RuntimeNamespace::Test { test_code } = namespace else {
        return Err(());
    };
    let Some(registry) = TEST_RUNTIMES.get() else {
        return Ok(None);
    };
    // Clone only an existing entry; release this mutex before any SQLite work.
    let states = registry.lock().map_err(|_| ())?;
    states
        .get(test_code)
        .map(|state| {
            state
                .as_ref()
                .map(|state| Some(Arc::clone(state)))
                .map_err(|_| ())
        })
        .unwrap_or(Ok(None))
}

pub(crate) fn read_cached_delivery_status() -> CachedDeliveryObservation {
    use super::super::health_cmd::durable_delivery::Failure;
    let namespace = match resolve_runtime_namespace() {
        Ok(namespace) => namespace,
        Err(_) => return CachedDeliveryObservation::Unavailable(Failure::RuntimeNamespace),
    };
    read_cached_status_in_namespace(namespace)
}

fn read_cached_status_in_namespace(namespace: RuntimeNamespace) -> CachedDeliveryObservation {
    use super::super::health_cmd::durable_delivery::Failure;
    let state = match cached_state(&namespace) {
        Ok(Some(state)) if state.namespace == namespace => state,
        Ok(None) => return CachedDeliveryObservation::NotInitialized,
        _ => return CachedDeliveryObservation::Unavailable(Failure::RuntimeUnavailable),
    };
    match state.coordinator.read_delivery_status() {
        Ok(snapshot) => CachedDeliveryObservation::Observed(snapshot),
        Err(error) => CachedDeliveryObservation::Unavailable(typed_failure(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m3_durable_health_missing_cache_does_not_construct_a_runtime() {
        let namespace = RuntimeNamespace::Test {
            test_code: "TEST_CODE_M3_NEVER_INITIALIZED_HEALTH".to_owned(),
        };
        assert!(matches!(
            read_cached_status_in_namespace(namespace),
            CachedDeliveryObservation::NotInitialized,
        ));
        if let Some(registry) = TEST_RUNTIMES.get() {
            assert!(!registry
                .lock()
                .unwrap()
                .contains_key("TEST_CODE_M3_NEVER_INITIALIZED_HEALTH"));
        }
    }

    #[test]
    fn m3_durable_health_error_projection_discards_sensitive_display() {
        let secret = "TEST_CODE_SECRET_ACCOUNT_SQL_PATH";
        let failure = typed_failure(DurableDeliveryError::IsolationViolation(secret.to_owned()));
        let bytes = serde_json::to_string(&failure).unwrap();
        assert!(!bytes.contains(secret));
        assert_eq!(
            failure,
            super::super::super::health_cmd::durable_delivery::Failure::IsolationViolation
        );
    }
}
