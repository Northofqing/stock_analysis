//! W04 immutable, catalog-bound run context.

use std::collections::BTreeMap;

use chrono::NaiveDate;

use super::canonical::{canonical_digest, canonical_preimage, CanonicalValue};
use super::delivery::TemplateVersion;
use super::facts::{source_ref_value, ExternalId, SourceProvider, SourceRef, SourceRefId};
use super::identity::{derive_occurrence_id, namespace_value, validate_text};
use super::{
    BusinessDate, Namespace, OccurrenceFamily, OccurrenceId, OccurrenceIdentityMaterial,
    ProducerId, PushJobError, Result, RunId, Sha256Digest, SourceContractId, SourceContractVersion,
    UnitId, UtcMicros,
};

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

#[derive(Clone, Debug, Eq, PartialEq)]
enum RegisteredTrigger {
    Scheduled(ScheduleId),
    Event(ProducerId),
    Manual,
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunContextInput {
    run_id: RunId,
    calendar_date: CalendarDate,
    phase: PhaseEpic,
    trigger: Trigger,
    occurrence: OccurrenceIdentityMaterial,
    captured_business_time: UtcMicros,
}

#[derive(Clone, Debug)]
pub(crate) struct RunContextFactory {
    binding: CatalogRunBinding,
}

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
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
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
    WrongSchedule,
    WrongEventProducer,
    WrongEventSourceContract,
    WrongOccurrenceFamily,
    WrongTestNamespaceRun,
}

#[cfg(test)]
pub(super) fn context_fixture(case: ContextFixtureCase) -> Result<RunContext> {
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
        unit_id: UnitId::try_new("MU-auction".to_owned())?,
        trigger: registered_trigger,
        occurrence_family: OccurrenceFamily::try_new("auction-session".to_owned())?,
        activation_generation: 7,
        build_commit: GitSha40::parse("0123456789abcdef0123456789abcdef01234567")?,
        catalog_sha256: Sha256Digest::parse("fixture catalog", &"c".repeat(64))?,
        source_contract_id,
        source_contract_version: SourceContractVersion::try_new("auction-source-v2".to_owned())?,
        template_version: TemplateVersion::try_new("auction-card-v3".to_owned())?,
    });
    factory.build_context(RunContextInput {
        run_id: RunId::try_new("run-20260907-090500".to_owned())?,
        calendar_date: CalendarDate::parse("2026-09-07")?,
        phase: PhaseEpic::Auction,
        trigger,
        occurrence: OccurrenceIdentityMaterial::new(
            BusinessDate::parse("2026-09-07")?,
            occurrence_family,
            super::OccurrenceKey::try_new("main".to_owned())?,
        ),
        captured_business_time: UtcMicros::try_new(1_788_743_100_000_000)?,
    })
}

#[cfg(test)]
pub(super) fn run_context_preimage_fixture(context: &RunContext) -> Vec<u8> {
    canonical_preimage("RunContext/v1", &run_context_fields(context))
}
