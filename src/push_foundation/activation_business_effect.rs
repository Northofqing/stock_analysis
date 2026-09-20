//! One attested business intent, recovered by a broker-owned worker without a sender.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
#[cfg(test)]
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::activation_business_source::ActivationBusinessSource;
use super::activation_fence::{EffectRequest, ExecutionContext, FenceError, Scope, WorkClass};
use super::intent_store::{
    AttestedReadyIntent, BusinessIntentStore, IntentSnapshot, TransitionReceipt,
};
use super::reconciler::{
    reconcile_current, RecoveryBindingError, RecoveryBindings, RecoveryBindingsPort,
    RecoveryBoundary, RecoveryConfig,
};
use super::terminal_authority::{
    AuthorityDescriptor, AuthorityQuery, AuthorityQueryFailure, TerminalAuthorityPort,
    TerminalTemplateBinding,
};
#[cfg(test)]
use crate::durable_delivery::DurableDeliveryCoordinator;
#[cfg(test)]
use crate::event::{AuditDispatcher, NewsFlashWindow};
use crate::monitor::push_job::{
    canonical_preimage, derive_decision_id, raw_digest, AuthorityClass, CanonicalValue,
    CompletionPolicy, DecisionId, IntentId, Sha256Digest,
};
#[cfg(test)]
use crate::monitor::push_job::{
    derive_occurrence_id, ChannelId, OccurrenceFamily, OccurrenceIdentityMaterial, OccurrenceKey,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BusinessRecoveryResult {
    pub(super) intent_id: String,
    pub(super) decision_id: String,
    pub(super) effect_sha256: String,
    pub(super) state: String,
    pub(super) version: u64,
    pub(super) lease_generation: u64,
    pub(super) snapshot_sha256: String,
    pub(super) transition_chain_sha256: String,
    pub(super) transition_count: u64,
    #[serde(deserialize_with = "required_optional_string")]
    pub(super) last_event_id: Option<String>,
    #[serde(deserialize_with = "required_optional_string")]
    pub(super) last_event_sha256: Option<String>,
    pub(super) recovery_boundary: String,
}

fn required_optional_string<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}

impl BusinessRecoveryResult {
    pub(super) fn is_resolved(&self) -> bool {
        self.recovery_boundary == "Finalized"
            && matches!(
                self.state.as_str(),
                "Completed" | "NotDelivered" | "NoData" | "Disabled"
            )
    }

    pub(super) fn validate(&self, request: &EffectRequest) -> Result<(), FenceError> {
        if request.actor != "Finalizer"
            || request.action != "ReconcileBusiness"
            || request.work_class != WorkClass::Recovery
            || request.effect_id != "business-reconcile"
            || request.effect_sha256 != self.effect_sha256
            || self.transition_count != self.version
            || self.lease_generation > self.version
            || (self.transition_count > 0) != self.last_event_id.is_some()
            || self.last_event_id.is_some() != self.last_event_sha256.is_some()
            || !matches!(
                self.state.as_str(),
                "PendingDispatch"
                    | "AwaitingAuthority"
                    | "AwaitingFinalizer"
                    | "ResolutionRequired"
                    | "Completed"
                    | "NotDelivered"
                    | "NoData"
                    | "Disabled"
            )
            || !matches!(
                self.recovery_boundary.as_str(),
                "DispatchPending"
                    | "LiveForeignLease"
                    | "AuthorityBlocked"
                    | "RejectedAuthorizationRequired"
                    | "OperatorAuditRequired"
                    | "ManualResolutionRequired"
                    | "Finalized"
            )
            || !matches!(
                (self.state.as_str(), self.recovery_boundary.as_str()),
                ("PendingDispatch", "DispatchPending" | "LiveForeignLease")
                    | (
                        "AwaitingAuthority",
                        "LiveForeignLease"
                            | "AuthorityBlocked"
                            | "RejectedAuthorizationRequired"
                            | "OperatorAuditRequired"
                    )
                    | ("AwaitingFinalizer", "LiveForeignLease" | "AuthorityBlocked")
                    | ("ResolutionRequired", "ManualResolutionRequired")
                    | (
                        "Completed" | "NotDelivered" | "NoData" | "Disabled",
                        "Finalized"
                    )
            )
        {
            return Err(FenceError::Store);
        }
        let digest = Sha256Digest::parse("business result intent", &self.intent_id)
            .map_err(|_| FenceError::Store)?;
        if derive_decision_id(&IntentId::from_digest(&digest)).as_str() != self.decision_id {
            return Err(FenceError::Store);
        }
        for digest in [
            &self.effect_sha256,
            &self.snapshot_sha256,
            &self.transition_chain_sha256,
        ]
        .into_iter()
        .chain(self.last_event_sha256.iter())
        .chain(self.last_event_id.iter())
        {
            Sha256Digest::parse("business result digest", digest).map_err(|_| FenceError::Store)?;
        }
        if self.last_event_id.as_ref().is_some_and(|v| v.is_empty()) {
            return Err(FenceError::Store);
        }
        Ok(())
    }

    pub(super) fn canonical_fields(&self) -> BTreeMap<&'static str, CanonicalValue> {
        let mut fields = BTreeMap::new();
        for (key, value) in [
            ("intent_id", &self.intent_id),
            ("decision_id", &self.decision_id),
            ("effect_sha256", &self.effect_sha256),
            ("state", &self.state),
            ("snapshot_sha256", &self.snapshot_sha256),
            ("transition_chain_sha256", &self.transition_chain_sha256),
            ("recovery_boundary", &self.recovery_boundary),
        ] {
            fields.insert(key, CanonicalValue::String(value.clone()));
        }
        for (key, value) in [
            ("version", self.version),
            ("lease_generation", self.lease_generation),
            ("transition_count", self.transition_count),
        ] {
            fields.insert(key, CanonicalValue::Unsigned(value));
        }
        for (key, value) in [
            ("last_event_id", &self.last_event_id),
            ("last_event_sha256", &self.last_event_sha256),
        ] {
            fields.insert(
                key,
                value
                    .as_ref()
                    .map_or(CanonicalValue::Null, |v| CanonicalValue::String(v.clone())),
            );
        }
        fields
    }
}

pub(super) struct BusinessEffect {
    scope: Scope,
    digest: String,
    database: PathBuf,
    business_device: u64,
    business_inode: u64,
    source: ActivationBusinessSource,
    snapshot: IntentSnapshot,
    template: TerminalTemplateBinding,
    completion_policy: CompletionPolicy,
    config: RecoveryConfig,
}

/// Construction is private to this module's worker entry. Neither Clone nor owned permit.
pub(super) struct BusinessExecution<'a> {
    context: &'a ExecutionContext,
    effect: &'a BusinessEffect,
}

impl BusinessExecution<'_> {
    pub(super) fn operation_binding(&self) -> String {
        self.context.operation_binding()
    }

    #[cfg(test)]
    pub(super) fn business_commit_ack_lost(&self) -> bool {
        self.context.business_commit_ack_lost()
    }

    #[cfg(test)]
    pub(super) fn pending_source_binding(&self) -> Result<Option<String>, FenceError> {
        self.context.pending_source_binding()
    }

    pub(super) fn check(
        &self,
        store: &BusinessIntentStore,
        intent_id: &str,
    ) -> Result<(), FenceError> {
        let effect = self.effect;
        self.context
            .authorize_business(effect.digest(), &effect.scope)?;
        if intent_id != effect.snapshot.intent_id()
            || store.activation_database_path() != effect.database.to_str()
        {
            return Err(FenceError::EffectMismatch);
        }
        effect.check_resources()
    }
}

struct FixedAuthority<'a> {
    execution: &'a BusinessExecution<'a>,
    #[cfg(test)]
    queries: std::cell::Cell<usize>,
}

impl TerminalAuthorityPort for FixedAuthority<'_> {
    fn descriptor(&self) -> &AuthorityDescriptor {
        self.execution.effect.source.descriptor()
    }
    fn requery_terminal(
        &self,
        decision_id: &DecisionId,
    ) -> Result<AuthorityQuery, AuthorityQueryFailure> {
        let effect = self.execution.effect;
        if decision_id.as_str()
            != effect
                .snapshot
                .attested_ready_binding()
                .map_err(|_| AuthorityQueryFailure)?
                .decision_id
                .as_str()
            || self
                .execution
                .context
                .authorize_business(effect.digest(), &effect.scope)
                .is_err()
            || effect.check_resources().is_err()
        {
            return Err(AuthorityQueryFailure);
        }
        #[cfg(test)]
        {
            self.queries.set(self.queries.get() + 1);
            if self.queries.get() == 2 {
                self.execution
                    .context
                    .business_requery_pause()
                    .map_err(|_| AuthorityQueryFailure)?;
            }
        }
        effect.source.requery(&effect.snapshot, decision_id)
    }
}

struct FixedBindings<'a> {
    effect: &'a BusinessEffect,
    authority: &'a dyn TerminalAuthorityPort,
}
impl RecoveryBindingsPort for FixedBindings<'_> {
    fn resolve<'a>(
        &'a self,
        intent: &AttestedReadyIntent,
    ) -> Result<RecoveryBindings<'a>, RecoveryBindingError> {
        if intent.intent_id.as_str() != self.effect.snapshot.intent_id() {
            return Err(RecoveryBindingError::Unavailable);
        }
        Ok(RecoveryBindings::new(
            &self.effect.template,
            &self.effect.completion_policy,
            self.authority,
        ))
    }
}

impl BusinessEffect {
    pub(super) fn digest(&self) -> &str {
        &self.digest
    }

    pub(super) fn canonical_bytes(&self) -> Result<Vec<u8>, FenceError> {
        if self.source.class() != AuthorityClass::GenericCounted {
            return self.dedicated_canonical_bytes();
        }
        let mut fields = self.common_canonical_fields()?;
        let durable_binding = self
            .source
            .coordinator_binding()
            .ok_or(FenceError::EffectMismatch)?;
        for (key, value) in [
            ("durable_store_path", durable_binding.0.as_str()),
            ("durable_environment", durable_binding.3.as_str()),
            ("durable_owner", durable_binding.4.as_str()),
        ] {
            fields.insert(key, CanonicalValue::String(value.into()));
        }
        for (key, value) in [
            ("durable_store_device", durable_binding.1),
            ("durable_store_inode", durable_binding.2),
        ] {
            fields.insert(key, CanonicalValue::Unsigned(value));
        }
        Ok(canonical_preimage(
            "ActivationBusinessRecoveryEffect/v1",
            &fields,
        ))
    }

    fn dedicated_canonical_bytes(&self) -> Result<Vec<u8>, FenceError> {
        let mut fields = self.common_canonical_fields()?;
        fields.extend(
            self.source
                .dedicated_canonical_fields()
                .map_err(|_| FenceError::EffectMismatch)?,
        );
        Ok(canonical_preimage(
            "ActivationDedicatedBusinessRecoveryEffect/v1",
            &fields,
        ))
    }

    fn common_canonical_fields(
        &self,
    ) -> Result<BTreeMap<&'static str, CanonicalValue>, FenceError> {
        let mut fields = self.scope.canonical_fields();
        for (key, value) in [
            ("effect_id", "business-reconcile"),
            ("actor", "Finalizer"),
            ("action", "ReconcileBusiness"),
            ("work_class", "Recovery"),
            (
                "business_store_path",
                self.database.to_str().ok_or(FenceError::Store)?,
            ),
            ("template_id", self.template.template_id().as_str()),
            (
                "template_version",
                self.template.template_version().as_str(),
            ),
            ("template_sha256", self.template.sha256().as_str()),
        ] {
            fields.insert(key, CanonicalValue::String(value.into()));
        }
        for (key, value) in [
            ("business_store_device", self.business_device),
            ("business_store_inode", self.business_inode),
        ] {
            fields.insert(key, CanonicalValue::Unsigned(value));
        }
        fields.insert(
            "snapshot",
            CanonicalValue::Object(self.snapshot.activation_snapshot_fields()),
        );
        fields.insert(
            "recovery_config",
            CanonicalValue::Object(self.config.activation_fields()),
        );
        fields.insert(
            "completion_policy_bytes",
            CanonicalValue::Array(
                self.completion_policy
                    .activation_binding_bytes()
                    .into_iter()
                    .map(|value| CanonicalValue::Unsigned(u64::from(value)))
                    .collect(),
            ),
        );
        Ok(fields)
    }

    fn check_resources(&self) -> Result<(), FenceError> {
        let metadata = std::fs::metadata(&self.database).map_err(|_| FenceError::Store)?;
        if (metadata.dev(), metadata.ino()) != (self.business_device, self.business_inode)
            || self.source.check_resources().is_err()
            || raw_digest(&self.canonical_bytes()?).as_str() != self.digest
        {
            return Err(FenceError::EffectMismatch);
        }
        Ok(())
    }

    pub(super) fn execute(
        &self,
        context: &ExecutionContext,
    ) -> Result<BusinessRecoveryResult, FenceError> {
        context.authorize_business(&self.digest, &self.scope)?;
        context.before_effect()?;
        self.check_resources()?;
        let intent = self
            .snapshot
            .attested_ready_binding()
            .map_err(|_| FenceError::EffectMismatch)?;
        let mut store = BusinessIntentStore::open(&self.database).map_err(|_| FenceError::Store)?;
        if store
            .inspect(&intent.intent_id)
            .map_err(|_| FenceError::Store)?
            .as_ref()
            != Some(&self.snapshot)
        {
            return Err(FenceError::EffectMismatch);
        }
        let execution = BusinessExecution {
            context,
            effect: self,
        };
        let authority = FixedAuthority {
            execution: &execution,
            #[cfg(test)]
            queries: std::cell::Cell::new(0),
        };
        let bindings = FixedBindings {
            effect: self,
            authority: &authority,
        };
        let entry = reconcile_current(
            &execution,
            &mut store,
            self.snapshot.clone(),
            &self.config,
            &bindings,
        )
        .map_err(|_| FenceError::Store)?;
        drop(store);
        self.check_resources()?;
        let confirmed = BusinessIntentStore::open(&self.database).map_err(|_| FenceError::Store)?;
        let snapshot = confirmed
            .inspect(&intent.intent_id)
            .map_err(|_| FenceError::Store)?
            .ok_or(FenceError::Store)?;
        let chain = confirmed
            .inspect_transition_chain(&intent.intent_id)
            .map_err(|_| FenceError::Store)?;
        if snapshot.intent_id() != entry.intent_id()
            || snapshot.state() != entry.state()
            || snapshot.version() != entry.version()
            || snapshot.lease_generation() != entry.lease_generation()
        {
            return Err(FenceError::Store);
        }
        Ok(BusinessRecoveryResult {
            intent_id: intent.intent_id.as_str().into(),
            decision_id: intent.decision_id.as_str().into(),
            effect_sha256: self.digest.clone(),
            state: snapshot.state().as_str().into(),
            version: snapshot.version(),
            lease_generation: snapshot.lease_generation(),
            snapshot_sha256: raw_digest(&canonical_preimage(
                "ActivationBusinessRecoverySnapshot/v1",
                &snapshot.activation_snapshot_fields(),
            ))
            .as_str()
            .into(),
            transition_chain_sha256: transition_chain_digest(&chain),
            transition_count: chain.len() as u64,
            last_event_id: chain.last().map(|v| v.event_id().as_str().into()),
            last_event_sha256: chain.last().map(|v| v.canonical_sha256().as_str().into()),
            recovery_boundary: boundary_name(entry.boundary()).into(),
        })
    }

    #[cfg(test)]
    pub(super) fn bind_fixture(
        scope: &Scope,
        fixture: BusinessEffectFixture,
    ) -> Result<Self, FenceError> {
        if !scope.namespace.starts_with("Test:") {
            return Err(FenceError::ProductionRefused);
        }
        let source = ActivationBusinessSource::generic(Arc::clone(&fixture.coordinator))
            .map_err(|_| FenceError::Store)?;
        Self::bind_fixture_source(
            scope,
            fixture.database,
            fixture.snapshot,
            fixture.template,
            fixture.completion_policy,
            fixture.config,
            source,
            AuthorityClass::GenericCounted,
        )
    }

    #[cfg(test)]
    pub(super) fn bind_p01_fixture(
        scope: &Scope,
        fixture: BusinessEffectFixture,
        required_channel: ChannelId,
    ) -> Result<Self, FenceError> {
        if !scope.namespace.starts_with("Test:") {
            return Err(FenceError::ProductionRefused);
        }
        let source = ActivationBusinessSource::p01(
            Arc::clone(&fixture.coordinator),
            fixture.template.clone(),
            required_channel,
        )
        .map_err(|_| FenceError::EffectMismatch)?;
        Self::bind_fixture_source(
            scope,
            fixture.database,
            fixture.snapshot,
            fixture.template,
            fixture.completion_policy,
            fixture.config,
            source,
            AuthorityClass::P01Dedicated,
        )
    }

    #[cfg(test)]
    pub(super) fn bind_n02_fixture(
        scope: &Scope,
        fixture: N02BusinessEffectFixture,
    ) -> Result<Self, FenceError> {
        use chrono::Datelike;

        if !scope.namespace.starts_with("Test:") {
            return Err(FenceError::ProductionRefused);
        }
        let business_date = fixture
            .snapshot
            .attested_ready_binding()
            .map_err(|_| FenceError::EffectMismatch)?
            .business_date;
        let year = chrono::NaiveDate::parse_from_str(business_date.as_str(), "%Y-%m-%d")
            .map_err(|_| FenceError::EffectMismatch)?
            .year();
        let source = ActivationBusinessSource::n02(
            Arc::clone(&fixture.audit),
            fixture.template.clone(),
            fixture.required_channel,
            fixture.window,
            year,
        )
        .map_err(|_| FenceError::EffectMismatch)?;
        Self::bind_fixture_source(
            scope,
            fixture.database,
            fixture.snapshot,
            fixture.template,
            fixture.completion_policy,
            fixture.config,
            source,
            AuthorityClass::N02Dedicated,
        )
    }

    #[cfg(test)]
    fn bind_fixture_source(
        scope: &Scope,
        fixture_database: PathBuf,
        fixture_snapshot: IntentSnapshot,
        fixture_template: TerminalTemplateBinding,
        fixture_completion_policy: CompletionPolicy,
        fixture_config: RecoveryConfig,
        source: ActivationBusinessSource,
        expected_class: AuthorityClass,
    ) -> Result<Self, FenceError> {
        let intent = fixture_snapshot
            .attested_ready_binding()
            .map_err(|_| FenceError::EffectMismatch)?;
        let database = std::fs::canonicalize(&fixture_database).map_err(|_| FenceError::Store)?;
        let metadata = std::fs::metadata(&database).map_err(|_| FenceError::Store)?;
        let store = BusinessIntentStore::open(&database).map_err(|_| FenceError::Store)?;
        if fixture_snapshot.namespace() != scope.namespace
            || intent.unit_id.as_str() != scope.unit
            || source.namespace() != scope.namespace
            || fixture_template.sha256() != &intent.template_sha256
            || fixture_completion_policy.completion_owner().unit_id() != &intent.unit_id
            || fixture_completion_policy
                .completion_owner()
                .completion_owner()
                != &intent.completion_owner
            || !fixture_completion_policy.allows_authority(expected_class)
            || source.class() != expected_class
            || store
                .inspect(&intent.intent_id)
                .map_err(|_| FenceError::Store)?
                .as_ref()
                != Some(&fixture_snapshot)
        {
            return Err(FenceError::EffectMismatch);
        }
        if expected_class == AuthorityClass::P01Dedicated {
            let expected_occurrence = derive_occurrence_id(&OccurrenceIdentityMaterial::new(
                intent.business_date.clone(),
                OccurrenceFamily::try_new("p01-business-date".to_owned())
                    .map_err(|_| FenceError::EffectMismatch)?,
                OccurrenceKey::try_new(intent.business_date.as_str().to_owned())
                    .map_err(|_| FenceError::EffectMismatch)?,
            ));
            if intent.unit_id.as_str() != "MU-p01"
                || intent.subject != crate::monitor::push_job::SubjectId::Global
                || intent.occurrence != expected_occurrence
            {
                return Err(FenceError::EffectMismatch);
            }
        } else if expected_class == AuthorityClass::N02Dedicated {
            let window = source.n02_window().ok_or(FenceError::EffectMismatch)?;
            if intent.unit_id.as_str() != "MU-news-flash-aggregate"
                || intent.subject != crate::monitor::push_job::SubjectId::Global
                || !fixture_snapshot.sla_n02_occurrence_supported()
                || !fixture_snapshot.sla_n02_window_matches(window.label())
            {
                return Err(FenceError::EffectMismatch);
            }
        }
        if expected_class != AuthorityClass::GenericCounted
            && source
                .requery(&fixture_snapshot, &intent.decision_id)
                .is_err()
        {
            return Err(FenceError::EffectMismatch);
        }
        let mut effect = Self {
            scope: scope.clone(),
            digest: String::new(),
            database,
            business_device: metadata.dev(),
            business_inode: metadata.ino(),
            source,
            snapshot: fixture_snapshot,
            template: fixture_template,
            completion_policy: fixture_completion_policy,
            config: fixture_config,
        };
        effect.digest = raw_digest(&effect.canonical_bytes()?).as_str().into();
        Ok(effect)
    }
}

pub(super) fn transition_chain_digest(chain: &[TransitionReceipt]) -> String {
    raw_digest(&canonical_preimage(
        "ActivationBusinessRecoveryTransitionChain/v1",
        &BTreeMap::from([(
            "events",
            CanonicalValue::Array(
                chain
                    .iter()
                    .map(|event| CanonicalValue::String(event.canonical_sha256().as_str().into()))
                    .collect(),
            ),
        )]),
    ))
    .as_str()
    .into()
}

fn boundary_name(boundary: RecoveryBoundary) -> &'static str {
    match boundary {
        RecoveryBoundary::DispatchPending => "DispatchPending",
        RecoveryBoundary::LiveForeignLease => "LiveForeignLease",
        RecoveryBoundary::AuthorityBlocked => "AuthorityBlocked",
        RecoveryBoundary::RejectedAuthorizationRequired => "RejectedAuthorizationRequired",
        RecoveryBoundary::OperatorAuditRequired => "OperatorAuditRequired",
        RecoveryBoundary::ManualResolutionRequired => "ManualResolutionRequired",
        RecoveryBoundary::Finalized => "Finalized",
    }
}

#[cfg(test)]
pub(super) struct BusinessEffectFixture {
    pub(super) database: PathBuf,
    pub(super) coordinator: Arc<DurableDeliveryCoordinator>,
    pub(super) snapshot: IntentSnapshot,
    pub(super) template: TerminalTemplateBinding,
    pub(super) completion_policy: CompletionPolicy,
    pub(super) config: RecoveryConfig,
}

#[cfg(test)]
pub(super) struct N02BusinessEffectFixture {
    pub(super) database: PathBuf,
    pub(super) audit: Arc<AuditDispatcher>,
    pub(super) snapshot: IntentSnapshot,
    pub(super) template: TerminalTemplateBinding,
    pub(super) completion_policy: CompletionPolicy,
    pub(super) config: RecoveryConfig,
    pub(super) required_channel: ChannelId,
    pub(super) window: NewsFlashWindow,
}

#[cfg(test)]
#[path = "activation_business_effect_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "activation_dedicated_business_effect_tests.rs"]
mod dedicated_tests;
