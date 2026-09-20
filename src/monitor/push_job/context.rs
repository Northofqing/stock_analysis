//! W04 immutable, catalog-bound run context.

use std::collections::BTreeMap;

use chrono::NaiveDate;

use super::canonical::canonical_preimage;
use super::canonical::{canonical_digest, CanonicalValue};
use super::delivery::TemplateVersion;
use super::facts::PreparationCapture;
use super::facts::{source_ref_value, SourceRef};
#[cfg(test)]
use super::facts::{ExternalId, SourceProvider, SourceRefId};
use super::identity::{derive_occurrence_id, namespace_value, validate_text};
use super::{
    BusinessDate, Namespace, OccurrenceFamily, OccurrenceId, OccurrenceIdentityMaterial,
    ProducerId, PushJobError, Result, RunId, Sha256Digest, SourceContractId, SourceContractVersion,
    UnitId, UtcMicros,
};

#[cfg_attr(not(test), allow(dead_code))]
const RUN_CONTEXT_SCHEMA_VERSION: u32 = 1;

macro_rules! context_text {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn try_new(value: String) -> Result<Self> {
                validate_text($field, value).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

context_text!(ScheduleId, "schedule_id");
context_text!(CommandId, "command_id");
context_text!(AuthenticatedOperatorRef, "authenticated_operator_ref");
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct CalendarDate(String);

impl CalendarDate {
    pub fn parse(value: &str) -> Result<Self> {
        let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map_err(|_| PushJobError::InvalidCalendarDate(value.to_owned()))?;
        if parsed.format("%Y-%m-%d").to_string() != value {
            return Err(PushJobError::InvalidCalendarDate(value.to_owned()));
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct GitSha40(String);

impl GitSha40 {
    pub fn parse(value: &str) -> Result<Self> {
        let valid = value.len() == 40
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !valid {
            return Err(PushJobError::InvalidGitSha40);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum PhaseEpic {
    Preopen,
    Auction,
    Intraday,
    Postclose,
}

impl PhaseEpic {
    fn as_str(self) -> &'static str {
        match self {
            Self::Preopen => "Preopen",
            Self::Auction => "Auction",
            Self::Intraday => "Intraday",
            Self::Postclose => "Postclose",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TriggerKind {
    Scheduled {
        schedule_id: ScheduleId,
    },
    Event {
        producer_id: ProducerId,
        source_ref: SourceRef,
    },
    Manual {
        command_id: CommandId,
        authenticated_operator_ref: AuthenticatedOperatorRef,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Trigger(TriggerKind);

impl Trigger {
    pub fn scheduled(schedule_id: ScheduleId) -> Self {
        Self(TriggerKind::Scheduled { schedule_id })
    }

    pub fn event(producer_id: ProducerId, source_ref: SourceRef) -> Self {
        Self(TriggerKind::Event {
            producer_id,
            source_ref,
        })
    }

    pub fn manual(
        command_id: CommandId,
        authenticated_operator_ref: AuthenticatedOperatorRef,
    ) -> Self {
        Self(TriggerKind::Manual {
            command_id,
            authenticated_operator_ref,
        })
    }

    pub fn view(&self) -> TriggerView<'_> {
        match &self.0 {
            TriggerKind::Scheduled { schedule_id } => TriggerView::Scheduled { schedule_id },
            TriggerKind::Event {
                producer_id,
                source_ref,
            } => TriggerView::Event {
                producer_id,
                source_ref,
            },
            TriggerKind::Manual {
                command_id,
                authenticated_operator_ref,
            } => TriggerView::Manual {
                command_id,
                authenticated_operator_ref,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TriggerView<'a> {
    Scheduled {
        schedule_id: &'a ScheduleId,
    },
    Event {
        producer_id: &'a ProducerId,
        source_ref: &'a SourceRef,
    },
    Manual {
        command_id: &'a CommandId,
        authenticated_operator_ref: &'a AuthenticatedOperatorRef,
    },
}

// W06 catalog registration is the first non-test constructor of these binding values.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, Eq, PartialEq)]
enum RegisteredTrigger {
    Scheduled(ScheduleId),
    Event(ProducerId),
    Manual,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CatalogRunBinding {
    namespace: Namespace,
    unit_id: UnitId,
    trigger: RegisteredTrigger,
    occurrence_family: OccurrenceFamily,
    activation_generation: u64,
    build_commit: GitSha40,
    catalog_sha256: Sha256Digest,
    source_contract_id: SourceContractId,
    source_contract_version: SourceContractVersion,
    template_version: TemplateVersion,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunContextInput {
    run_id: RunId,
    calendar_date: CalendarDate,
    phase: PhaseEpic,
    trigger: Trigger,
    occurrence: OccurrenceIdentityMaterial,
    captured_business_time: UtcMicros,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug)]
pub(crate) struct RunContextFactory {
    binding: CatalogRunBinding,
}

#[cfg_attr(not(test), allow(dead_code))]
impl RunContextFactory {
    pub(crate) fn new(binding: CatalogRunBinding) -> Self {
        Self { binding }
    }

    pub(super) fn build_context(&self, input: RunContextInput) -> Result<RunContext> {
        if input.occurrence.occurrence_family() != &self.binding.occurrence_family {
            return Err(PushJobError::InvalidRunContext(
                "occurrence family does not match catalog binding",
            ));
        }
        if let Namespace::Test { run_id } = &self.binding.namespace {
            if run_id != &input.run_id {
                return Err(PushJobError::InvalidRunContext(
                    "test namespace run_id does not match context run_id",
                ));
            }
        }
        if !trigger_matches(&self.binding, &input.trigger) {
            return Err(PushJobError::InvalidRunContext(
                "trigger does not match catalog binding",
            ));
        }

        Ok(RunContext {
            schema_version: RUN_CONTEXT_SCHEMA_VERSION,
            run_id: input.run_id,
            unit_id: self.binding.unit_id.clone(),
            namespace: self.binding.namespace.clone(),
            business_date: input.occurrence.business_date().clone(),
            calendar_date: input.calendar_date,
            phase: input.phase,
            trigger: input.trigger,
            occurrence: derive_occurrence_id(&input.occurrence),
            captured_business_time: input.captured_business_time,
            activation_generation: self.binding.activation_generation,
            build_commit: self.binding.build_commit.clone(),
            catalog_sha256: self.binding.catalog_sha256.clone(),
            source_contract_version: self.binding.source_contract_version.clone(),
            template_version: self.binding.template_version.clone(),
        })
    }

    pub(super) fn expected_source_contract_id(&self) -> &SourceContractId {
        &self.binding.source_contract_id
    }

    pub(crate) fn begin_capture(self, input: RunContextInput) -> Result<PreparationCapture> {
        let context = self.build_context(input)?;
        Ok(PreparationCapture::new(
            context,
            self.expected_source_contract_id().clone(),
        ))
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn trigger_matches(binding: &CatalogRunBinding, trigger: &Trigger) -> bool {
    match (&binding.trigger, &trigger.0) {
        (
            RegisteredTrigger::Scheduled(expected),
            TriggerKind::Scheduled {
                schedule_id: actual,
            },
        ) => expected == actual,
        (
            RegisteredTrigger::Event(expected),
            TriggerKind::Event {
                producer_id,
                source_ref,
            },
        ) => {
            expected == producer_id
                && source_ref.source_contract_id() == &binding.source_contract_id
        }
        (RegisteredTrigger::Manual, TriggerKind::Manual { .. }) => true,
        _ => false,
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct RunContext {
    schema_version: u32,
    run_id: RunId,
    unit_id: UnitId,
    namespace: Namespace,
    business_date: BusinessDate,
    calendar_date: CalendarDate,
    phase: PhaseEpic,
    trigger: Trigger,
    occurrence: OccurrenceId,
    captured_business_time: UtcMicros,
    activation_generation: u64,
    build_commit: GitSha40,
    catalog_sha256: Sha256Digest,
    source_contract_version: SourceContractVersion,
    template_version: TemplateVersion,
}

impl RunContext {
    pub(crate) fn canonical_bytes(&self) -> Vec<u8> {
        canonical_preimage("RunContext/v1", &run_context_fields(self))
    }

    pub(crate) fn from_canonical_bytes(bytes: &[u8]) -> Result<Self> {
        let json = bytes
            .strip_prefix(b"RunContext/v1\0")
            .ok_or(PushJobError::InvalidRunContext("context codec domain"))?;
        let value: serde_json::Value = serde_json::from_slice(json)
            .map_err(|_| PushJobError::InvalidRunContext("context codec json"))?;
        let text = |key: &str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_str)
                .ok_or(PushJobError::InvalidRunContext("context codec field"))
        };
        let unsigned = |key: &str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_u64)
                .ok_or(PushJobError::InvalidRunContext("context codec field"))
        };
        let namespace = match value
            .get("namespace")
            .and_then(|item| item.get("kind"))
            .and_then(serde_json::Value::as_str)
        {
            Some("Production") => Namespace::Production,
            _ => return Err(PushJobError::InvalidRunContext("context namespace")),
        };
        let trigger = match value
            .get("trigger")
            .and_then(|item| item.get("kind"))
            .and_then(serde_json::Value::as_str)
        {
            Some("Scheduled") => Trigger::scheduled(ScheduleId::try_new(
                value
                    .get("trigger")
                    .and_then(|item| item.get("schedule_id"))
                    .and_then(serde_json::Value::as_str)
                    .ok_or(PushJobError::InvalidRunContext("context trigger"))?
                    .to_owned(),
            )?),
            _ => return Err(PushJobError::InvalidRunContext("context trigger")),
        };
        let occurrence = Sha256Digest::parse("occurrence", text("occurrence")?)?;
        let context = Self {
            schema_version: u32::try_from(unsigned("schema_version")?)
                .map_err(|_| PushJobError::InvalidRunContext("context schema"))?,
            run_id: RunId::try_new(text("run_id")?.to_owned())?,
            unit_id: UnitId::try_new(text("unit_id")?.to_owned())?,
            namespace,
            business_date: BusinessDate::parse(text("business_date")?)?,
            calendar_date: CalendarDate::parse(text("calendar_date")?)?,
            phase: match text("phase")? {
                "Postclose" => PhaseEpic::Postclose,
                _ => return Err(PushJobError::InvalidRunContext("context phase")),
            },
            trigger,
            occurrence: OccurrenceId::from_digest(&occurrence),
            captured_business_time: UtcMicros::try_new(
                i64::try_from(unsigned("captured_business_time")?)
                    .map_err(|_| PushJobError::InvalidRunContext("context time"))?,
            )?,
            activation_generation: unsigned("activation_generation")?,
            build_commit: GitSha40::parse(text("build_commit")?)?,
            catalog_sha256: Sha256Digest::parse("catalog_sha256", text("catalog_sha256")?)?,
            source_contract_version: SourceContractVersion::try_new(
                text("source_contract_version")?.to_owned(),
            )?,
            template_version: TemplateVersion::try_new(text("template_version")?.to_owned())?,
        };
        let fixed_trigger = matches!(
            context.trigger(),
            TriggerView::Scheduled { schedule_id }
                if schedule_id.as_str() == "chain-post-close-timer"
        );
        if context.schema_version != RUN_CONTEXT_SCHEMA_VERSION
            || context.activation_generation == 0
            || context.namespace != Namespace::Production
            || context.unit_id.as_str() != "MU-chain-post-close"
            || context.phase != PhaseEpic::Postclose
            || !fixed_trigger
            || context.source_contract_version.as_str() != "1"
            || context.template_version.as_str() != "chain-analysis-prepared-v1"
            || context.canonical_bytes() != bytes
        {
            return Err(PushJobError::InvalidRunContext(
                "context codec canonical bytes",
            ));
        }
        Ok(context)
    }
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    pub fn unit_id(&self) -> &UnitId {
        &self.unit_id
    }

    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }

    pub fn business_date(&self) -> &BusinessDate {
        &self.business_date
    }

    pub fn calendar_date(&self) -> &CalendarDate {
        &self.calendar_date
    }

    pub fn phase(&self) -> PhaseEpic {
        self.phase
    }

    pub fn trigger(&self) -> TriggerView<'_> {
        self.trigger.view()
    }

    pub fn occurrence(&self) -> &OccurrenceId {
        &self.occurrence
    }

    pub fn captured_business_time(&self) -> UtcMicros {
        self.captured_business_time
    }

    pub fn activation_generation(&self) -> u64 {
        self.activation_generation
    }

    pub fn build_commit(&self) -> &GitSha40 {
        &self.build_commit
    }

    pub fn catalog_sha256(&self) -> &Sha256Digest {
        &self.catalog_sha256
    }

    pub fn source_contract_version(&self) -> &SourceContractVersion {
        &self.source_contract_version
    }

    pub fn template_version(&self) -> &TemplateVersion {
        &self.template_version
    }

    pub fn canonical_sha256(&self) -> Sha256Digest {
        canonical_digest("RunContext/v1", &run_context_fields(self))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LocalChainPostCloseConfig {
    expected_catalog_sha256: Sha256Digest,
    build_commit: GitSha40,
    activation_generation: u64,
}

impl LocalChainPostCloseConfig {
    pub(crate) fn try_new(
        expected_catalog_sha256: Sha256Digest,
        build_commit: GitSha40,
        activation_generation: u64,
    ) -> Result<Self> {
        if activation_generation == 0 {
            return Err(PushJobError::InvalidRunContext(
                "activation generation must be positive",
            ));
        }
        Ok(Self {
            expected_catalog_sha256,
            build_commit,
            activation_generation,
        })
    }

    pub(crate) fn expected_catalog_sha256(&self) -> &Sha256Digest {
        &self.expected_catalog_sha256
    }

    pub(crate) fn accepts(&self, context: &RunContext) -> bool {
        context.catalog_sha256 == self.expected_catalog_sha256
            && context.build_commit == self.build_commit
            && context.activation_generation == self.activation_generation
    }
}

pub(crate) struct LocalChainPostCloseRunInput {
    run_id: RunId,
    calendar_date: CalendarDate,
    business_date: BusinessDate,
    captured_at: UtcMicros,
}

impl LocalChainPostCloseRunInput {
    pub(crate) fn try_new(
        run_id: RunId,
        calendar_date: CalendarDate,
        business_date: BusinessDate,
        captured_at: UtcMicros,
    ) -> Result<Self> {
        Ok(Self {
            run_id,
            calendar_date,
            business_date,
            captured_at,
        })
    }
}

pub(crate) struct LocalChainPostCloseContext {
    run_context: RunContext,
    occurrence_family: OccurrenceFamily,
    occurrence_key: super::OccurrenceKey,
    source_contract_id: SourceContractId,
}

impl LocalChainPostCloseContext {
    pub(crate) fn run_context(&self) -> &RunContext {
        &self.run_context
    }

    pub(crate) fn occurrence_family(&self) -> &OccurrenceFamily {
        &self.occurrence_family
    }

    pub(crate) fn occurrence_key(&self) -> &super::OccurrenceKey {
        &self.occurrence_key
    }

    pub(crate) fn source_contract_id(&self) -> &SourceContractId {
        &self.source_contract_id
    }
}

pub(crate) fn build_single_user_local_chain_post_close_context(
    config: &LocalChainPostCloseConfig,
    input: LocalChainPostCloseRunInput,
) -> Result<LocalChainPostCloseContext> {
    let catalog = super::MachineCatalog::bundled()
        .map_err(|_| PushJobError::InvalidRunContext("bundled catalog rejected"))?;
    if catalog.catalog_sha256() != &config.expected_catalog_sha256 {
        return Err(PushJobError::InvalidRunContext("catalog digest mismatch"));
    }
    let producer_id = ProducerId::try_new("chain-post-close-timer".to_owned())?;
    let producer = catalog
        .producer(&producer_id)
        .ok_or(PushJobError::InvalidRunContext(
            "chain post-close producer missing",
        ))?;
    if producer.unit_id().as_str() != "MU-chain-post-close"
        || producer.phase_epics() != [PhaseEpic::Postclose]
        || producer.occurrence_family().as_str()
            != "calendar date / 15:30≤t<15:35 / latest completed business date"
        || producer.completion_owner().as_str() != "monitor_loop::CHAIN_POST_LAST[calendar_date]"
    {
        return Err(PushJobError::InvalidRunContext(
            "chain post-close catalog mapping mismatch",
        ));
    }
    let source_contract_id =
        SourceContractId::try_new("chain-post-close-passed-input-v1".to_owned())?;
    let occurrence_key = super::OccurrenceKey::try_new(format!(
        "{}/15:30-15:35/{}",
        input.calendar_date.as_str(),
        input.business_date.as_str()
    ))?;
    let occurrence_family = producer.occurrence_family().clone();
    let binding = CatalogRunBinding {
        namespace: Namespace::Production,
        unit_id: producer.unit_id().clone(),
        trigger: RegisteredTrigger::Scheduled(ScheduleId::try_new(
            "chain-post-close-timer".to_owned(),
        )?),
        occurrence_family: occurrence_family.clone(),
        activation_generation: config.activation_generation,
        build_commit: config.build_commit.clone(),
        catalog_sha256: config.expected_catalog_sha256.clone(),
        source_contract_id: source_contract_id.clone(),
        source_contract_version: SourceContractVersion::try_new("1".to_owned())?,
        template_version: TemplateVersion::try_new("chain-analysis-prepared-v1".to_owned())?,
    };
    let occurrence = OccurrenceIdentityMaterial::new(
        input.business_date,
        occurrence_family.clone(),
        occurrence_key.clone(),
    );
    let run_context = RunContextFactory::new(binding).build_context(RunContextInput {
        run_id: input.run_id,
        calendar_date: input.calendar_date,
        phase: PhaseEpic::Postclose,
        trigger: Trigger::scheduled(ScheduleId::try_new("chain-post-close-timer".to_owned())?),
        occurrence,
        captured_business_time: input.captured_at,
    })?;
    Ok(LocalChainPostCloseContext {
        run_context,
        occurrence_family,
        occurrence_key,
        source_contract_id,
    })
}

fn run_context_fields(context: &RunContext) -> BTreeMap<&'static str, CanonicalValue> {
    BTreeMap::from([
        (
            "activation_generation",
            CanonicalValue::Unsigned(context.activation_generation),
        ),
        (
            "build_commit",
            CanonicalValue::String(context.build_commit.as_str().to_owned()),
        ),
        (
            "business_date",
            CanonicalValue::String(context.business_date.as_str().to_owned()),
        ),
        (
            "calendar_date",
            CanonicalValue::String(context.calendar_date.as_str().to_owned()),
        ),
        (
            "captured_business_time",
            CanonicalValue::Unsigned(context.captured_business_time.get() as u64),
        ),
        (
            "catalog_sha256",
            CanonicalValue::String(context.catalog_sha256.as_str().to_owned()),
        ),
        ("namespace", namespace_value(&context.namespace)),
        (
            "occurrence",
            CanonicalValue::String(context.occurrence.as_str().to_owned()),
        ),
        (
            "phase",
            CanonicalValue::String(context.phase.as_str().to_owned()),
        ),
        (
            "run_id",
            CanonicalValue::String(context.run_id.as_str().to_owned()),
        ),
        (
            "schema_version",
            CanonicalValue::Unsigned(u64::from(context.schema_version)),
        ),
        (
            "source_contract_version",
            CanonicalValue::String(context.source_contract_version.as_str().to_owned()),
        ),
        (
            "template_version",
            CanonicalValue::String(context.template_version.as_str().to_owned()),
        ),
        ("trigger", trigger_value(&context.trigger)),
        (
            "unit_id",
            CanonicalValue::String(context.unit_id.as_str().to_owned()),
        ),
    ])
}

fn trigger_value(trigger: &Trigger) -> CanonicalValue {
    match &trigger.0 {
        TriggerKind::Scheduled { schedule_id } => CanonicalValue::Object(BTreeMap::from([
            ("kind", CanonicalValue::String("Scheduled".to_owned())),
            (
                "schedule_id",
                CanonicalValue::String(schedule_id.as_str().to_owned()),
            ),
        ])),
        TriggerKind::Event {
            producer_id,
            source_ref,
        } => CanonicalValue::Object(BTreeMap::from([
            ("kind", CanonicalValue::String("Event".to_owned())),
            (
                "producer_id",
                CanonicalValue::String(producer_id.as_str().to_owned()),
            ),
            ("source_ref", source_ref_value(source_ref)),
        ])),
        TriggerKind::Manual {
            command_id,
            authenticated_operator_ref,
        } => CanonicalValue::Object(BTreeMap::from([
            (
                "authenticated_operator_ref",
                CanonicalValue::String(authenticated_operator_ref.as_str().to_owned()),
            ),
            (
                "command_id",
                CanonicalValue::String(command_id.as_str().to_owned()),
            ),
            ("kind", CanonicalValue::String("Manual".to_owned())),
        ])),
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(super) enum ContextFixtureCase {
    ValidScheduled,
    ValidEvent,
    ValidManual,
    ValidOtherUnit,
    ValidOtherOccurrence,
    ValidOtherCapturedTime,
    WrongSchedule,
    WrongEventProducer,
    WrongEventSourceContract,
    WrongOccurrenceFamily,
    WrongTestNamespaceRun,
}

#[cfg(test)]
fn context_fixture_parts(case: ContextFixtureCase) -> Result<(RunContextFactory, RunContextInput)> {
    let source_contract_id = SourceContractId::try_new("auction-source".to_owned())?;
    let event_source_contract = if matches!(case, ContextFixtureCase::WrongEventSourceContract) {
        SourceContractId::try_new("other-source".to_owned())?
    } else {
        source_contract_id.clone()
    };
    let source_ref = SourceRef::new(
        SourceRefId::try_new("source-event-1".to_owned())?,
        SourceProvider::try_new("fixture-provider".to_owned())?,
        ExternalId::try_new("external-event-1".to_owned())?,
        event_source_contract,
        Sha256Digest::parse("fixture event content", &"d".repeat(64))?,
    );

    let (registered_trigger, trigger) = match case {
        ContextFixtureCase::ValidEvent | ContextFixtureCase::WrongEventSourceContract => (
            RegisteredTrigger::Event(ProducerId::try_new("auction-event".to_owned())?),
            Trigger::event(ProducerId::try_new("auction-event".to_owned())?, source_ref),
        ),
        ContextFixtureCase::WrongEventProducer => (
            RegisteredTrigger::Event(ProducerId::try_new("auction-event".to_owned())?),
            Trigger::event(ProducerId::try_new("other-event".to_owned())?, source_ref),
        ),
        ContextFixtureCase::ValidManual => (
            RegisteredTrigger::Manual,
            Trigger::manual(
                CommandId::try_new("command-1".to_owned())?,
                AuthenticatedOperatorRef::try_new("operator-session-1".to_owned())?,
            ),
        ),
        ContextFixtureCase::WrongSchedule => (
            RegisteredTrigger::Scheduled(ScheduleId::try_new("auction-main".to_owned())?),
            Trigger::scheduled(ScheduleId::try_new("other-schedule".to_owned())?),
        ),
        _ => (
            RegisteredTrigger::Scheduled(ScheduleId::try_new("auction-main".to_owned())?),
            Trigger::scheduled(ScheduleId::try_new("auction-main".to_owned())?),
        ),
    };

    let namespace = if matches!(case, ContextFixtureCase::WrongTestNamespaceRun) {
        Namespace::test(RunId::try_new("namespace-run".to_owned())?)
    } else {
        Namespace::Production
    };
    let occurrence_family = if matches!(case, ContextFixtureCase::WrongOccurrenceFamily) {
        OccurrenceFamily::try_new("other-family".to_owned())?
    } else {
        OccurrenceFamily::try_new("auction-session".to_owned())?
    };

    let factory = RunContextFactory::new(CatalogRunBinding {
        namespace,
        unit_id: UnitId::try_new(
            if matches!(case, ContextFixtureCase::ValidOtherUnit) {
                "MU-other"
            } else {
                "MU-auction"
            }
            .to_owned(),
        )?,
        trigger: registered_trigger,
        occurrence_family: OccurrenceFamily::try_new("auction-session".to_owned())?,
        activation_generation: 7,
        build_commit: GitSha40::parse("0123456789abcdef0123456789abcdef01234567")?,
        catalog_sha256: Sha256Digest::parse("fixture catalog", &"c".repeat(64))?,
        source_contract_id,
        source_contract_version: SourceContractVersion::try_new("auction-source-v2".to_owned())?,
        template_version: TemplateVersion::try_new("auction-card-v3".to_owned())?,
    });
    Ok((
        factory,
        RunContextInput {
            run_id: RunId::try_new("run-20260907-090500".to_owned())?,
            calendar_date: CalendarDate::parse("2026-09-07")?,
            phase: PhaseEpic::Auction,
            trigger,
            occurrence: OccurrenceIdentityMaterial::new(
                BusinessDate::parse("2026-09-07")?,
                occurrence_family,
                super::OccurrenceKey::try_new(
                    if matches!(case, ContextFixtureCase::ValidOtherOccurrence) {
                        "other"
                    } else {
                        "main"
                    }
                    .to_owned(),
                )?,
            ),
            captured_business_time: UtcMicros::try_new(
                if matches!(case, ContextFixtureCase::ValidOtherCapturedTime) {
                    1_788_743_100_000_001
                } else {
                    1_788_743_100_000_000
                },
            )?,
        },
    ))
}

#[cfg(test)]
pub(super) fn context_fixture(case: ContextFixtureCase) -> Result<RunContext> {
    let (factory, input) = context_fixture_parts(case)?;
    factory.build_context(input)
}

#[cfg(test)]
pub(super) fn capture_capability_fixture() -> Result<PreparationCapture> {
    let (factory, input) = context_fixture_parts(ContextFixtureCase::ValidScheduled)?;
    factory.begin_capture(input)
}

#[cfg(test)]
pub(super) fn run_context_preimage_fixture(context: &RunContext) -> Vec<u8> {
    canonical_preimage("RunContext/v1", &run_context_fields(context))
}
