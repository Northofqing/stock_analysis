//! Fixed Generic resources and their attested activation binding; never supplied through IPC.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::activation_fence::{EffectRequest, ExecutionContext, FenceError, Scope, WorkClass};
use super::generic_transport::{
    GenericDispatchFence, GenericDispatchRequest, GenericTransportAuthorityAdapter,
    GenericTransportRoute,
};
use super::{BusinessIntentStore, IntentSnapshot};
use crate::durable_delivery::{
    AuthoritativeSink, DurableDeliveryCoordinator, FoundationTerminalQuery, ImmutableAppendPort,
};
use crate::monitor::push_job::{
    canonical_preimage, derive_decision_id, raw_digest, CanonicalValue, CompletionPolicy,
    DeliveryResultView, IntentId, Sha256Digest, TerminalDisposition, UtcMicros,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GenericEffectKind {
    Dispatch,
    Reconcile,
}

impl GenericEffectKind {
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Dispatch => "generic-dispatch",
            Self::Reconcile => "generic-reconcile",
        }
    }
    pub(super) fn action(self) -> &'static str {
        match self {
            Self::Dispatch => "DispatchGeneric",
            Self::Reconcile => "ReconcileGeneric",
        }
    }
    pub(super) fn class(self) -> WorkClass {
        match self {
            Self::Dispatch => WorkClass::NewWork,
            Self::Reconcile => WorkClass::Recovery,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum GenericDisposition {
    Accepted,
    Rejected,
    ManualConfirmedAccepted,
    ManualConfirmedNotDelivered,
    Uncertain,
    PendingSeal,
    MissingAuthority,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GenericEffectResult {
    pub(super) decision_id: String,
    pub(super) intent_id: String,
    #[serde(deserialize_with = "required_optional_string")]
    pub(super) terminal_ref: Option<String>,
    pub(super) terminal_disposition: GenericDisposition,
    #[serde(deserialize_with = "required_optional_string")]
    pub(super) terminal_binding_sha256: Option<String>,
    #[serde(deserialize_with = "required_optional_string")]
    pub(super) terminal_evidence_sha256: Option<String>,
    pub(super) effect_sha256: String,
}

fn required_optional_string<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

impl GenericEffectResult {
    pub(super) fn is_resolved(&self) -> bool {
        matches!(
            self.terminal_disposition,
            GenericDisposition::Accepted
                | GenericDisposition::Rejected
                | GenericDisposition::ManualConfirmedAccepted
                | GenericDisposition::ManualConfirmedNotDelivered
        )
    }

    pub(super) fn validate(&self, request: &EffectRequest) -> Result<(), FenceError> {
        if request.actor != "Dispatcher"
            || !matches!(
                (request.action.as_str(), request.work_class),
                ("DispatchGeneric", WorkClass::NewWork) | ("ReconcileGeneric", WorkClass::Recovery)
            )
            || self.effect_sha256 != request.effect_sha256
        {
            return Err(FenceError::Store);
        }
        let digest = Sha256Digest::parse("activation result intent", &self.intent_id)
            .map_err(|_| FenceError::Store)?;
        if derive_decision_id(&IntentId::from_digest(&digest)).as_str() != self.decision_id {
            return Err(FenceError::Store);
        }
        let terminal = !matches!(
            self.terminal_disposition,
            GenericDisposition::PendingSeal | GenericDisposition::MissingAuthority
        );
        if terminal != self.terminal_ref.is_some()
            || terminal != self.terminal_binding_sha256.is_some()
            || terminal != self.terminal_evidence_sha256.is_some()
            || self
                .terminal_ref
                .as_ref()
                .is_some_and(|v| v.trim().is_empty())
        {
            return Err(FenceError::Store);
        }
        for value in [&self.effect_sha256]
            .into_iter()
            .chain(self.terminal_binding_sha256.iter())
            .chain(self.terminal_evidence_sha256.iter())
        {
            Sha256Digest::parse("activation result digest", value)
                .map_err(|_| FenceError::Store)?;
        }
        Ok(())
    }

    pub(super) fn canonical_fields(&self) -> BTreeMap<&'static str, CanonicalValue> {
        let disposition = match self.terminal_disposition {
            GenericDisposition::Accepted => "Accepted",
            GenericDisposition::Rejected => "Rejected",
            GenericDisposition::ManualConfirmedAccepted => "ManualConfirmedAccepted",
            GenericDisposition::ManualConfirmedNotDelivered => "ManualConfirmedNotDelivered",
            GenericDisposition::Uncertain => "Uncertain",
            GenericDisposition::PendingSeal => "PendingSeal",
            GenericDisposition::MissingAuthority => "MissingAuthority",
        };
        BTreeMap::from([
            (
                "decision_id",
                CanonicalValue::String(self.decision_id.clone()),
            ),
            ("intent_id", CanonicalValue::String(self.intent_id.clone())),
            (
                "effect_sha256",
                CanonicalValue::String(self.effect_sha256.clone()),
            ),
            (
                "terminal_disposition",
                CanonicalValue::String(disposition.into()),
            ),
            ("terminal_ref", optional_string(&self.terminal_ref)),
            (
                "terminal_binding_sha256",
                optional_string(&self.terminal_binding_sha256),
            ),
            (
                "terminal_evidence_sha256",
                optional_string(&self.terminal_evidence_sha256),
            ),
        ])
    }
}

fn optional_string(value: &Option<String>) -> CanonicalValue {
    value.as_ref().map_or(CanonicalValue::Null, |value| {
        CanonicalValue::String(value.clone())
    })
}

struct GenericResources {
    database: PathBuf,
    business_device: u64,
    business_inode: u64,
    coordinator: Arc<DurableDeliveryCoordinator>,
    durable_binding: (String, u64, u64, String, String),
    snapshot: IntentSnapshot,
    route: GenericTransportRoute,
    fence: GenericDispatchFence,
    completion_policy: CompletionPolicy,
    sink: AuthoritativeSink,
    sink_identity: String,
    append_port: Arc<dyn ImmutableAppendPort + Send + Sync>,
    dispatched_at: UtcMicros,
    verified_at: UtcMicros,
}

pub(super) struct GenericEffect {
    kind: GenericEffectKind,
    scope: Scope,
    digest: String,
    resources: Arc<GenericResources>,
}

impl GenericEffect {
    pub(super) fn kind(&self) -> GenericEffectKind {
        self.kind
    }
    pub(super) fn digest(&self) -> &str {
        &self.digest
    }

    pub(super) fn canonical_bytes(&self) -> Result<Vec<u8>, FenceError> {
        let r = &self.resources;
        let attested = r
            .snapshot
            .attested_ready_binding()
            .map_err(|_| FenceError::EffectMismatch)?;
        let mut fields = self.scope.canonical_fields();
        for (key, value) in [
            ("effect_id", self.kind.id()),
            ("actor", "Dispatcher"),
            ("action", self.kind.action()),
            (
                "work_class",
                match self.kind.class() {
                    WorkClass::NewWork => "NewWork",
                    WorkClass::Recovery => "Recovery",
                },
            ),
            (
                "business_store_path",
                r.database.to_str().ok_or(FenceError::Store)?,
            ),
            ("durable_store_path", r.durable_binding.0.as_str()),
            ("durable_environment", r.durable_binding.3.as_str()),
            ("durable_owner", r.durable_binding.4.as_str()),
            ("sink_identity", r.sink_identity.as_str()),
            ("decision_id", attested.decision_id.as_str()),
            ("intent_id", attested.intent_id.as_str()),
            ("occurrence", attested.occurrence.as_str()),
        ] {
            fields.insert(key, CanonicalValue::String(value.into()));
        }
        for (key, value) in [
            ("business_store_device", r.business_device),
            ("business_store_inode", r.business_inode),
            ("durable_store_device", r.durable_binding.1),
            ("durable_store_inode", r.durable_binding.2),
        ] {
            fields.insert(key, CanonicalValue::Unsigned(value));
        }
        for (key, value) in [
            ("dispatched_at", r.dispatched_at),
            ("verified_at", r.verified_at),
        ] {
            fields.insert(key, CanonicalValue::String(value.get().to_string()));
        }
        fields.insert(
            "snapshot",
            CanonicalValue::Object(r.snapshot.activation_snapshot_fields()),
        );
        fields.insert("route", CanonicalValue::Object(r.route.activation_fields()));
        fields.insert(
            "dispatch_fence",
            CanonicalValue::Object(r.fence.activation_fields()),
        );
        fields.insert(
            "completion_policy_bytes",
            CanonicalValue::Array(
                r.completion_policy
                    .activation_binding_bytes()
                    .into_iter()
                    .map(|v| CanonicalValue::Unsigned(u64::from(v)))
                    .collect(),
            ),
        );
        Ok(canonical_preimage(
            "ActivationGenericTransportEffect/v1",
            &fields,
        ))
    }

    pub(super) fn validate_runtime(
        &self,
        context: &ExecutionContext,
        coordinator: &DurableDeliveryCoordinator,
    ) -> Result<(), FenceError> {
        context.authorize_generic(self, &self.scope)?;
        let r = &self.resources;
        if !std::ptr::eq(coordinator, r.coordinator.as_ref())
            || coordinator
                .activation_storage_binding()
                .map_err(|_| FenceError::Store)?
                != r.durable_binding
            || r.sink.sink_identity() != r.sink_identity
            || self.digest != raw_digest(&self.canonical_bytes()?).as_str()
        {
            return Err(FenceError::EffectMismatch);
        }
        let meta = std::fs::metadata(&r.database).map_err(|_| FenceError::Store)?;
        if (meta.dev(), meta.ino()) != (r.business_device, r.business_inode) {
            return Err(FenceError::Store);
        }
        let attested = r
            .snapshot
            .attested_ready_binding()
            .map_err(|_| FenceError::EffectMismatch)?;
        let store = BusinessIntentStore::open(&r.database).map_err(|_| FenceError::Store)?;
        if store
            .inspect(&attested.intent_id)
            .map_err(|_| FenceError::Store)?
            .as_ref()
            != Some(&r.snapshot)
        {
            return Err(FenceError::EffectMismatch);
        }
        Ok(())
    }

    pub(super) fn dispatch_request(&self) -> GenericDispatchRequest<'_> {
        let r = &self.resources;
        GenericDispatchRequest::new(
            &r.snapshot,
            &r.route,
            &r.fence,
            &r.completion_policy,
            Arc::clone(&r.sink),
            r.append_port.as_ref(),
            r.dispatched_at,
            r.verified_at,
        )
    }

    pub(super) fn execute(
        &self,
        context: &ExecutionContext,
    ) -> Result<GenericEffectResult, FenceError> {
        context.before_effect()?;
        let r = &self.resources;
        // Failures before admission to the actual seam cannot manufacture a terminal observation.
        self.validate_runtime(context, r.coordinator.as_ref())?;
        let attested = r
            .snapshot
            .attested_ready_binding()
            .map_err(|_| FenceError::EffectMismatch)?;
        let mut observation = GenericEffectResult {
            decision_id: attested.decision_id.as_str().into(),
            intent_id: attested.intent_id.as_str().into(),
            terminal_ref: None,
            terminal_disposition: GenericDisposition::MissingAuthority,
            terminal_binding_sha256: None,
            terminal_evidence_sha256: None,
            effect_sha256: self.digest.clone(),
        };
        let result =
            GenericTransportAuthorityAdapter::new(&r.coordinator).execute_current(context, self);
        match result {
            Ok(result) => {
                let terminal = match result.view() {
                    DeliveryResultView::TransportAccepted(v)
                    | DeliveryResultView::TransportRejected(v)
                    | DeliveryResultView::TransportUncertain(v)
                    | DeliveryResultView::AlreadyTerminal(v) => v,
                    _ => return Err(FenceError::Store),
                };
                observation.terminal_ref = Some(terminal.ref_id().as_str().into());
                observation.terminal_binding_sha256 =
                    Some(terminal.binding_sha256().as_str().into());
                observation.terminal_evidence_sha256 =
                    Some(terminal.evidence_sha256().as_str().into());
                observation.terminal_disposition = match terminal.terminal_disposition() {
                    TerminalDisposition::Accepted => GenericDisposition::Accepted,
                    TerminalDisposition::Rejected => GenericDisposition::Rejected,
                    TerminalDisposition::Uncertain => GenericDisposition::Uncertain,
                    TerminalDisposition::ManualConfirmedAccepted => {
                        GenericDisposition::ManualConfirmedAccepted
                    }
                    TerminalDisposition::ManualConfirmedNotDelivered => {
                        GenericDisposition::ManualConfirmedNotDelivered
                    }
                };
            }
            Err(_) => {
                if matches!(
                    r.coordinator
                        .inspect_foundation_terminal(attested.decision_id.as_str()),
                    Ok(FoundationTerminalQuery::PendingSeal { .. })
                ) {
                    observation.terminal_disposition = GenericDisposition::PendingSeal;
                }
            }
        }
        Ok(observation)
    }

    #[cfg(test)]
    pub(super) fn bind_fixture(
        scope: &Scope,
        fixture: GenericEffectFixture,
    ) -> Result<[Self; 2], FenceError> {
        if !scope.namespace.starts_with("Test:") {
            return Err(FenceError::ProductionRefused);
        }
        let attested = fixture
            .snapshot
            .attested_ready_binding()
            .map_err(|_| FenceError::EffectMismatch)?;
        let database = std::fs::canonicalize(&fixture.database).map_err(|_| FenceError::Store)?;
        let metadata = std::fs::metadata(&database).map_err(|_| FenceError::Store)?;
        let durable_binding = fixture
            .coordinator
            .activation_storage_binding()
            .map_err(|_| FenceError::Store)?;
        let store = BusinessIntentStore::open(&database).map_err(|_| FenceError::Store)?;
        if fixture.snapshot.namespace() != scope.namespace
            || attested.unit_id.as_str() != scope.unit
            || durable_binding.3 != scope.namespace
            || store
                .inspect(&attested.intent_id)
                .map_err(|_| FenceError::Store)?
                .as_ref()
                != Some(&fixture.snapshot)
            || fixture.sink.sink_identity() != fixture.route.required_channel().as_str()
            || fixture.verified_at < fixture.dispatched_at
            || fixture.route.template_sha256() != &attested.template_sha256
            || fixture.snapshot.state() != super::IntentState::AwaitingAuthority
            || !fixture
                .fence
                .matches(&fixture.snapshot, fixture.verified_at)
            || fixture.completion_policy.completion_owner().unit_id() != &attested.unit_id
            || fixture
                .completion_policy
                .completion_owner()
                .completion_owner()
                != &attested.completion_owner
            || !fixture
                .completion_policy
                .allows_authority(crate::monitor::push_job::AuthorityClass::GenericCounted)
        {
            return Err(FenceError::EffectMismatch);
        }
        let resources = Arc::new(GenericResources {
            database,
            business_device: metadata.dev(),
            business_inode: metadata.ino(),
            durable_binding,
            sink_identity: fixture.sink.sink_identity().into(),
            coordinator: fixture.coordinator,
            snapshot: fixture.snapshot,
            route: fixture.route,
            fence: fixture.fence,
            completion_policy: fixture.completion_policy,
            sink: fixture.sink,
            append_port: fixture.append_port,
            dispatched_at: fixture.dispatched_at,
            verified_at: fixture.verified_at,
        });
        let bind = |kind| -> Result<Self, FenceError> {
            let mut effect = Self {
                kind,
                scope: scope.clone(),
                digest: String::new(),
                resources: Arc::clone(&resources),
            };
            effect.digest = raw_digest(&effect.canonical_bytes()?).as_str().into();
            Ok(effect)
        };
        Ok([
            bind(GenericEffectKind::Dispatch)?,
            bind(GenericEffectKind::Reconcile)?,
        ])
    }
}

#[cfg(test)]
pub(super) struct GenericEffectFixture {
    pub(super) database: PathBuf,
    pub(super) coordinator: Arc<DurableDeliveryCoordinator>,
    pub(super) snapshot: IntentSnapshot,
    pub(super) route: GenericTransportRoute,
    pub(super) fence: GenericDispatchFence,
    pub(super) completion_policy: CompletionPolicy,
    pub(super) sink: AuthoritativeSink,
    pub(super) append_port: Arc<dyn ImmutableAppendPort + Send + Sync>,
    pub(super) dispatched_at: UtcMicros,
    pub(super) verified_at: UtcMicros,
}
